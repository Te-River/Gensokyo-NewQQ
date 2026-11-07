package handlers

// 素材管理（media-images）上传链：本地图片字节 → prepare（换预签名 URL）→ PUT 直传 COS
// → complete（换 qqbot-media:// 引用）。官方文档：/server-inter/message/material-management.html。
//
// 为什么需要：Markdown 图片写外链时由平台下载后转存，短签名外链（ChatGLM 等）与
// 预签名直链都会被平台拉取失败。素材管理把图片放进平台自己的素材库，Markdown 里只写
// qqbot-media://image/v1/<token> 引用，平台直接转存，不依赖任何外网资源。
//
// oss_type=7（ChatGLM）即「官方素材 API 模式」：Markdown/keyboard 本地图片只走素材链，
// 图床只在素材链失败（含 40035007 配额超限）时兜底，两条上传链不并存、不重复上传。
// 素材引用是平台托管的可复用引用（滑动有效期 90 天，每次被消息引用自动顺延），发送后
// 不删除，同一张图可被多条消息复用。qqbot-media:// 只适用于 Markdown/keyboard：卡片
// pic_url、/uploadpic、/uploadpicv2 等必须公网 URL 的路径维持既有外链上传，不走素材链。
// 配额（单机器人 100 张）只在上传遇到 40035007 时清理：按 created_at 从旧到新删 20 张、
// 跳过最近 24 小时创建的图片，再重试一次；仍失败才降级外链。
//
// 本文件不注册 OneBot action：当前只有 Markdown/keyboard 图片消费点（message_parser.go、
// shortlived_image.go）调用素材链；将来若要暴露 upload_media_image，复用 mediaImageUploadFlow 即可。

import (
	"bytes"
	"context"
	"crypto/sha1"
	"encoding/base64"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"net/http"
	"sort"
	"strings"
	"time"

	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/errs"
	"github.com/tencent-connect/botgo/openapi"
)

// maxMediaImageBytes 素材管理单图上限：官方写「≤10MB」未定义进制，取十进制保守值。
const maxMediaImageBytes = 10 * 1000 * 1000

// mediaQuotaPurgeCount 配额触顶（40035007）时一次清理的素材数量：一次删够余量，
// 避免每发一张图都要再清一轮。
const mediaQuotaPurgeCount = 20

// mediaQuotaProtectWindow 清理时保护的创建时间窗口：最近 24 小时内上传的素材不删，
// 刚发送成功的图仍可能被引用/排查，留着更稳（官方说明删除不影响已发送消息，
// 但引用窗口内的图没有理由冒险）。
const mediaQuotaProtectWindow = 24 * time.Hour

// mediaImageErrorCodes 官方 400350xx 错误码 → 中文可读说明（原文见素材管理页「错误码」表）
var mediaImageErrorCodes = []struct {
	code int
	text string
}{
	{40035001, "图片格式不支持（扩展名与 Content-Type 须为 png/jpeg/gif/webp 且一致）"},
	{40035002, "图片超过大小上限（单图不超过 10MB）"},
	{40035003, "上传任务不存在、已过期（超过 4 小时）或不属于当前机器人"},
	{40035006, "服务端内部错误（COS 处理或链接生成失败），可稍后重试"},
	{40035007, "图片数量超过上限（单机器人 100 张），请删除后重试"},
	{40035008, "图片不存在或已被删除"},
	{40035009, "图片已过期或已被删除，引用链接不再可用"},
	{40035010, "引用链接无效，或不属于当前机器人"},
}

// ossTypeShortLivedImage 当前 oss_type 是否启用「官方素材 API 模式」。ChatGLM（oss_type=7）
// 的外链带两层短签名（约 30 分钟 + 302 跳转后 15 分钟），平台转存拉不动，该模式下
// Markdown/keyboard 本地图片直接走素材链、不再先传图床；其余 provider 的外链稳定可用，
// 维持既有外链上传，不额外消耗素材配额。挂成注入缝只为离线单测可替换。
var ossTypeShortLivedImage = func() bool {
	return config.GetOssType() == config.OssTypeChatGLM
}

// resolveLocalImageMedia Markdown/keyboard 本地图片字节 → 可写进 markdown 的引用：
//  1. oss_type=7（官方素材 API 模式）：只走素材链换 qqbot-media:// 引用，不再先传 ChatGLM
//     图床（双上传已消除）；
//  2. 其余图床：走既有外链上传，外链本身是短签名形态时再用同一份字节换素材引用；
//  3. 素材链失败或 40035007 配额超限：退回既有外链/直链兜底，不阻断发送，不比现状差。
//
// 上传、空值守卫与短链处置统一落在 uploadBytesToPublicURL，本函数只负责调度
// 「oss_type=7 先换素材引用」这一层，避免与自取路径各写一份规则后漂移。
func resolveLocalImageMedia(apiv2 openapi.OpenAPI, imageData []byte) (string, bool) {
	if ossTypeShortLivedImage() {
		if ref, ok := uploadImageBytesToMediaStore(apiv2, imageData, "markdown"); ok {
			return ref, true
		}
		mylog.Printf("[markdown] 素材链不可用，降级既有外链上传: size=%d", len(imageData))
		// 同一份字节已按短签名 provider 试过素材链，外链结果仍是短链时不再重试一次；
		// 本地文件字节不做魔数门禁（bmp 等素材链不认的格式仍要走通外链上传）。
		return uploadBytesToPublicURL(apiv2, imageData, false, "")
	}
	// 其余图床外链稳定可用，只有外链结果意外仍是短签名形态时才换素材引用
	return uploadBytesToPublicURL(apiv2, imageData, false, "markdown")
}

// uploadBytesToPublicURL 内存字节 → 公网 URL（base64 → 既有外链上传流程）：
// Markdown/keyboard 的本地文件字节与短签名自取字节共用这一份规则，避免两处实现漂移。
//   - requireSupportedFormat=true 先按魔数嗅探，非 png/jpeg/gif/webp 直接拒绝（自取字节必须先过闸）；
//     本地文件字节传 false——bmp 等素材链嗅探表不认的格式仍要能走通外链上传；
//   - 空外链一律判失败，绝不回 ("", true)：否则 markdown 会被写成 `![]( )`；
//   - 上传结果仍是短签名域时，mediaStoreSource 非空先用同一份字节换平台素材引用（滑动 90 天，可复用）
//     （平台拒拉预签名直链，素材引用才稳），再退直链解析；都失败才拒绝替换。
//
// 任一步失败返回 ("", false)，调用方维持原引用（降级语义）。
func uploadBytesToPublicURL(apiv2 openapi.OpenAPI, data []byte, requireSupportedFormat bool, mediaStoreSource string) (string, bool) {
	if requireSupportedFormat && sniffImageFormat(data) == "" {
		mylog.Printf("[media] 字节不是受支持的图片格式（仅 png/jpeg/gif/webp），拒走外链上传: size=%d", len(data))
		return "", false
	}
	cdnURL, _, _, err := markdownImageUpload(base64.StdEncoding.EncodeToString(data), apiv2)
	if err != nil {
		mylog.Printf("[media] 图片走既有外链上传流程失败: size=%d err=%v", len(data), err)
		return "", false
	}
	if cdnURL == "" {
		mylog.Printf("[media] 既有外链上传流程返回空外链，判定失败: size=%d", len(data))
		return "", false
	}
	if isShortLivedImageURL(cdnURL) {
		// 上传链自身优先返回解析后的 302 直链，本地公网 URL 只是解析失败后的次级回退；
		// 仍回短签名域说明该 provider 未纳入上传链的直链解析——先换素材引用，再退直链解析。
		if mediaStoreSource != "" {
			if ref, ok := uploadImageBytesToMediaStore(apiv2, data, mediaStoreSource); ok {
				return ref, true
			}
		}
		directURL, resolveErr := resolveShortLivedURL(cdnURL)
		if resolveErr == nil {
			mylog.Printf("[media] 上传结果仍是短签名外链，改用解析后的直链: %s -> %s", cdnURL, directURL)
			return directURL, true
		}
		mylog.Printf("[media] 上传结果仍是短签名外链且直链解析失败，拒绝替换: size=%d url=%s err=%v", len(data), cdnURL, resolveErr)
		return "", false
	}
	return cdnURL, true
}

// uploadImageBytesToMediaStore 内存字节走素材链（prepare→PUT→complete）换 qqbot-media:// 引用。
// source 只作文件名前缀与日志标签。任一步失败返回 ("", false)，调用方维持原引用（降级语义）；
// 40035007（单机器人 100 张上限）单独记 warn，语义与其它失败一致——一律降级外链/直链。
func uploadImageBytesToMediaStore(apiv2 openapi.OpenAPI, data []byte, source string) (string, bool) {
	if apiv2 == nil {
		mylog.Printf("[media] apiv2 未注入，跳过素材链上传: size=%d", len(data))
		return "", false
	}
	if err := checkMediaImageSize(int64(len(data))); err != nil {
		mylog.Printf("[media] 图片超过素材链大小上限，跳过素材链: %v", err)
		return "", false
	}
	contentType, ext := sniffMediaImageFormat(data)
	if ext == "" {
		mylog.Printf("[media] 图片格式不受素材链支持（仅 png/jpeg/gif/webp），跳过素材链: size=%d", len(data))
		return "", false
	}
	ref, err := mediaImageUploadFlow(context.TODO(), apiv2, data, source+"."+ext, contentType, source)
	if err != nil {
		if errors.Is(err, errMediaImageQuotaExceeded) {
			mylog.Printf("[media] 素材配额超限（单机器人 100 张），本次降级直链/外链: %v", err)
		} else {
			mylog.Printf("[media] 素材链上传失败，降级原引用: %v", err)
		}
		return "", false
	}
	if !strings.HasPrefix(ref, "qqbot-media://") {
		mylog.Printf("[media] 素材链未回 qqbot-media 引用，降级原引用: url=%q", ref)
		return "", false
	}
	mylog.Printf("[media] 图片已转素材引用: size=%d source=%s", len(data), source)
	return ref, true
}

// mediaImageUploadFlow 素材链三步：本地 md5/sha1 → prepare → 预签名 PUT 直传 → complete，
// 成功返回可直接写进 markdown 的 qqbot-media:// 引用。source 为日志前缀（调用来源标签）。
func mediaImageUploadFlow(ctx context.Context, apiv2 openapi.OpenAPI, content []byte, fileName, contentType, source string) (string, error) {
	if apiv2 == nil {
		return "", errors.New("素材管理失败：apiv2 未注入")
	}
	// md5 与 sha1 一律本地计算，不接受调用方传入
	md5Hash := computeMD5(content)
	sha1Hash := computeMediaImageSHA1(content)

	prepareReq := &dto.MediaImagePrepareRequest{
		FileName:    fileName,
		FileSize:    uint64(len(content)),
		ContentType: contentType,
		MD5:         md5Hash,
		SHA1:        sha1Hash,
	}
	prepareResp, err := apiv2.PrepareMediaImage(ctx, prepareReq)
	if err != nil && isMediaImageErrorCode(err, 40035007) {
		mylog.Printf("%s: 素材配额超限，清理最旧素材后重试: %v", source, err)
		if purgeErr := purgeMediaImages(ctx, apiv2, source); purgeErr != nil {
			mylog.Printf("%s: 清理旧素材失败，不再重试: %v", source, purgeErr)
		} else {
			prepareResp, err = apiv2.PrepareMediaImage(ctx, prepareReq)
		}
	}
	if err != nil {
		mylog.Printf("%s: prepare 失败: %v", source, err)
		return "", mediaImageFlowError(err)
	}
	if prepareResp == nil || prepareResp.PresignedURL == "" {
		mylog.Printf("%s: prepare 未返回 presigned_url", source)
		return "", errors.New("素材管理失败：prepare 未返回 presigned_url")
	}

	etag, err := putMediaImageContent(prepareResp.PresignedURL, contentType, content)
	if err != nil {
		mylog.Printf("%s: 直传失败: %v", source, err)
		return "", err
	}

	// complete 的 md5 取 PUT 响应 ETag 去引号；COS 未回 ETag 时回退本地 md5
	md5ForComplete := normalizeMediaImageETag(etag)
	if md5ForComplete == "" {
		mylog.Printf("%s: 直传响应缺少 ETag，回退本地 md5", source)
		md5ForComplete = md5Hash
	} else if md5ForComplete != md5Hash {
		mylog.Printf("%s: ETag(%s) 与本地 md5(%s) 不一致，complete 以 ETag 为准", source, md5ForComplete, md5Hash)
	}

	completeReq := &dto.MediaImageCompleteRequest{
		UploadID: prepareResp.UploadID,
		MD5:      md5ForComplete,
	}
	completeResp, err := apiv2.CompleteMediaImage(ctx, completeReq)
	if err != nil && isMediaImageErrorCode(err, 40035007) {
		mylog.Printf("%s: 素材配额超限（complete），清理最旧素材后重试: %v", source, err)
		if purgeErr := purgeMediaImages(ctx, apiv2, source); purgeErr != nil {
			mylog.Printf("%s: 清理旧素材失败，不再重试: %v", source, purgeErr)
		} else {
			completeResp, err = apiv2.CompleteMediaImage(ctx, completeReq)
		}
	}
	if err != nil {
		mylog.Printf("%s: complete 失败: %v", source, err)
		return "", mediaImageFlowError(err)
	}
	if completeResp == nil {
		mylog.Printf("%s: complete 未返回内容", source)
		return "", errors.New("素材管理失败：complete 未返回内容")
	}

	mylog.Printf("%s: 上传完成 image_id=%s ttl=%d", source, completeResp.ImageID, completeResp.TTL)
	return completeResp.URL, nil
}

// ---------- 辅助函数 ----------

// sniffMediaImageFormat 按魔数嗅探 MIME 与素材链规范扩展名，仅认素材链允许的四种。
func sniffMediaImageFormat(data []byte) (contentType, ext string) {
	switch http.DetectContentType(data) {
	case "image/png":
		return "image/png", "png"
	case "image/jpeg":
		return "image/jpeg", "jpg"
	case "image/gif":
		return "image/gif", "gif"
	case "image/webp":
		return "image/webp", "webp"
	}
	return "", ""
}

// checkMediaImageSize 单图体积上限校验
func checkMediaImageSize(size int64) error {
	if size > maxMediaImageBytes {
		return fmt.Errorf("素材图片 %d 字节超过上限 %d 字节（10MB）", size, maxMediaImageBytes)
	}
	return nil
}

// computeMediaImageSHA1 计算内容 SHA1（40 位小写十六进制）
func computeMediaImageSHA1(data []byte) string {
	h := sha1.New()
	h.Write(data)
	return hex.EncodeToString(h.Sum(nil))
}

// normalizeMediaImageETag ETag → complete 需要的 md5：去弱校验前缀 W/、去引号、转小写
func normalizeMediaImageETag(etag string) string {
	v := strings.TrimSpace(etag)
	v = strings.TrimPrefix(v, "W/")
	v = strings.Trim(v, "\"")
	return strings.ToLower(v)
}

// mediaImagePutClient 素材直传专用客户端（包级复用，避免每次上传新建连接池）。
// 预签名 PUT 不跟随重定向：301/302 会被 Go 降级成 GET 并丢掉请求体，回 2xx 就成了
// "假成功"；用 ErrUseLastResponse 把重定向原样返回给状态码判定。
var mediaImagePutClient = &http.Client{
	Timeout: 120 * time.Second,
	CheckRedirect: func(*http.Request, []*http.Request) error {
		return http.ErrUseLastResponse
	},
}

// putMediaImageContent 直传预签名 URL（COS）：
// 预签名 URL 自带鉴权，绝不能带 QQBot Authorization；Content-Type 必须与 prepare 声明完全一致，否则 COS 返回 403。
func putMediaImageContent(presignedURL, contentType string, data []byte) (string, error) {
	req, err := http.NewRequest(http.MethodPut, presignedURL, bytes.NewReader(data))
	if err != nil {
		return "", fmt.Errorf("素材图片直传失败：构造请求出错: %v", err)
	}
	req.Header.Set("Content-Type", contentType)

	resp, err := mediaImagePutClient.Do(req)
	if err != nil {
		return "", fmt.Errorf("素材图片直传失败: %v", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		body, _ := io.ReadAll(io.LimitReader(resp.Body, 2048))
		return "", fmt.Errorf("素材图片直传失败：COS 返回 %d %s", resp.StatusCode, strings.TrimSpace(string(body)))
	}
	// 读完剩余响应体，保证连接可复用
	_, _ = io.Copy(io.Discard, io.LimitReader(resp.Body, 4096))
	return resp.Header.Get("ETag"), nil
}

// describeMediaImageError 把上游错误还原成「中文说明(官方码)」形式，
// 官方码原值保留在文本里，便于调用方自行判定（40035010 = 跨机器人引用）；
// SDK 错误带 traceID 时一并保留（40035006 服务端内部错误要靠它找腾讯侧日志）。
// SDK 错误以结构体码为准，文本再兜一层（错误文本里带了码也能命中）。
func describeMediaImageError(err error) string {
	body := err.Error()
	code := 0
	traceID := ""
	var sdkErr *errs.Err
	if errors.As(err, &sdkErr) {
		body = sdkErr.Text()
		code = sdkErr.Code()
		traceID = sdkErr.Trace()
	}
	for _, item := range mediaImageErrorCodes {
		if item.code == code || strings.Contains(body, fmt.Sprint(item.code)) {
			if traceID != "" {
				return fmt.Sprintf("素材管理失败(%d)：%s trace_id=%s", item.code, item.text, traceID)
			}
			return fmt.Sprintf("素材管理失败(%d)：%s", item.code, item.text)
		}
	}
	return fmt.Sprintf("素材管理失败：%v", err)
}

// errMediaImageQuotaExceeded 素材数量触顶（40035007）：清理旧素材并重试一次后仍失败，
// 或没有可清理的旧素材时返回；调用方据此降级外链/直链，不阻断发送。
var errMediaImageQuotaExceeded = errors.New(
	"素材管理失败(40035007)：图片数量超过上限（单机器人 100 张），本次降级外链/直链")

// isMediaImageErrorCode 判定上游错误是否命中指定官方码（SDK 结构体码优先，错误文本兜底）。
func isMediaImageErrorCode(err error, code int) bool {
	if err == nil {
		return false
	}
	var sdkErr *errs.Err
	if errors.As(err, &sdkErr) && sdkErr.Code() == code {
		return true
	}
	return strings.Contains(err.Error(), fmt.Sprint(code))
}

// mediaImageFlowError 素材链上游错误 → 调用方可判定的错误：
// 40035007 包成哨兵 errMediaImageQuotaExceeded，其余保留「中文说明(官方码)」。
func mediaImageFlowError(err error) error {
	if isMediaImageErrorCode(err, 40035007) {
		return fmt.Errorf("%w: %s", errMediaImageQuotaExceeded, describeMediaImageError(err))
	}
	return errors.New(describeMediaImageError(err))
}

// purgeMediaImages 配额触顶（40035007）时的清理：拉全量列表，按 created_at 从旧到新
// 删除最多 mediaQuotaPurgeCount 张，跳过最近 mediaQuotaProtectWindow 内创建的图片
// （官方说明删除不影响已发送消息，但 24 小时内的图仍在引用/排查窗口内，不删）。
// 返回 nil 表示删掉了至少一张（可以重试上传），否则返回原因（调用方直接降级）。
func purgeMediaImages(ctx context.Context, apiv2 openapi.OpenAPI, source string) error {
	images, err := listMediaImagesOldestFirst(ctx, apiv2)
	if err != nil {
		return fmt.Errorf("拉取素材列表失败: %w", err)
	}
	protectAfter := time.Now().Add(-mediaQuotaProtectWindow).Unix()
	deleted := 0
	for _, img := range images {
		if deleted >= mediaQuotaPurgeCount {
			break
		}
		if img.ImageID == "" || img.CreatedAt >= protectAfter {
			continue
		}
		if err := apiv2.DeleteMediaImage(ctx, img.ImageID); err != nil {
			if isMediaImageErrorCode(err, 40035008) {
				continue // 已被删除，不算失败
			}
			mylog.Printf("%s: 删除旧素材失败 image_id=%s: %v", source, img.ImageID, err)
			continue
		}
		deleted++
	}
	if deleted == 0 {
		return errors.New("没有可清理的旧素材（最近 24 小时内创建的不删）")
	}
	mylog.Printf("%s: 配额超限已清理 %d 张旧素材", source, deleted)
	return nil
}

// listMediaImagesOldestFirst 分页拉取素材列表并按 created_at 升序（最旧在前）。
// 页码上限 10 防游标异常时死循环（单机器人上限 100 张，正常几页即可拉完）。
func listMediaImagesOldestFirst(ctx context.Context, apiv2 openapi.OpenAPI) ([]*dto.MediaImage, error) {
	var all []*dto.MediaImage
	cursor := ""
	for page := 0; page < 10; page++ {
		list, err := apiv2.ListMediaImages(ctx, cursor)
		if err != nil {
			return nil, err
		}
		if list == nil {
			break
		}
		for _, img := range list.Images {
			if img != nil {
				all = append(all, img)
			}
		}
		if list.NextCursor == "" {
			break
		}
		cursor = list.NextCursor
	}
	sort.Slice(all, func(i, j int) bool { return all[i].CreatedAt < all[j].CreatedAt })
	return all, nil
}

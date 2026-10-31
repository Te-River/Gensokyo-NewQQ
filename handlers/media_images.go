package handlers

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
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/errs"
	"github.com/tencent-connect/botgo/openapi"
)

func init() {
	callapi.RegisterHandler("upload_media_image", UploadMediaImage)
	callapi.RegisterHandler("get_media_image_list", GetMediaImageList)
	callapi.RegisterHandler("delete_media_image", DeleteMediaImage)
}

// maxMediaImageBytes 素材管理单图上限：官方写「≤10MB」未定义进制，
// 取十进制保守值，与 upload_helper.go 的 5MB 口径保持一致。
const maxMediaImageBytes = 10 * 1000 * 1000

// mediaImageContentTypes content_type → 规范扩展名（官方只认这四种）
var mediaImageContentTypes = map[string]string{
	"image/png":  "png",
	"image/jpeg": "jpg",
	"image/gif":  "gif",
	"image/webp": "webp",
}

// mediaImageExtContentTypes 扩展名 → content_type，jpg/jpeg 视作同一族
var mediaImageExtContentTypes = map[string]string{
	"png":  "image/png",
	"jpg":  "image/jpeg",
	"jpeg": "image/jpeg",
	"gif":  "image/gif",
	"webp": "image/webp",
}

var (
	// errMediaImageFileUnreadable 文件读不到（对应 OneBot retcode 1404）
	errMediaImageFileUnreadable = errors.New("素材图片读取失败")
	// errMediaImageTooLarge 超过单图大小上限（对应 OneBot retcode 1400）
	errMediaImageTooLarge = errors.New("素材图片超过大小上限")
)

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

// MediaImageUploadResponse upload_media_image 的 data
type MediaImageUploadResponse struct {
	ImageID    string `json:"image_id"`
	URL        string `json:"url"`
	TTL        uint32 `json:"ttl"`
	PreviewURL string `json:"preview_url"`
	FileName   string `json:"file_name"`
	ExpiresIn  uint32 `json:"expires_in"`
	ExpiresAt  int64  `json:"expires_at,omitempty"`
}

// MediaImageItem get_media_image_list 的列表项
type MediaImageItem struct {
	ImageID    string `json:"image_id"`
	URL        string `json:"url"`
	FileName   string `json:"file_name"`
	PreviewURL string `json:"preview_url"`
	CreatedAt  int64  `json:"created_at"`
	ExpiresAt  int64  `json:"expires_at"`
}

// MediaImageListResponse get_media_image_list 的 data
type MediaImageListResponse struct {
	Images     []MediaImageItem `json:"images"`
	NextCursor string           `json:"next_cursor"`
	Total      int              `json:"total"`
	Next       bool             `json:"next"` // next_cursor 非空即还有下一页
}

// MediaImageDryRunResponse 是 shadow 预演下的占位回执：结构与真实 data 同层，
// 但不含任何上游结果——shadow 只做预演，不比对新链结果。
type MediaImageDryRunResponse struct {
	DryRun  bool   `json:"dry_run"`
	Action  string `json:"action"`
	Message string `json:"message"`
}

func newMediaImageDryRunResult(action string) MediaImageDryRunResponse {
	return MediaImageDryRunResponse{
		DryRun:  true,
		Action:  action,
		Message: "shadow 预演：未调用上游素材接口",
	}
}

// UploadMediaImage 素材管理：上传单图，返回可直接写进 Markdown 的 qqbot-media:// 引用
// params: file(必填: base64:// 裸 base64 / data: URI / file:// 或本地路径), file_name(可选), content_type(可选)
func UploadMediaImage(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	if message.ShadowDryRun {
		// shadow 预演：不读文件、不 prepare/直传/complete（真上传会占 100 张配额），只回占位
		mylog.Printf("upload_media_image: shadow dry-run，跳过真实上传")
		return sendActionResultWithData(client, message, newMediaImageDryRunResult("upload_media_image"))
	}
	if apiv2 == nil {
		return sendActionResult(client, message, "素材管理失败：OpenAPI v2 未初始化", 1401)
	}

	content, fileName, err := loadMediaImageContent(message.Params.File, strings.TrimSpace(message.Params.FileName))
	if err != nil {
		retCode := 1400
		if errors.Is(err, errMediaImageFileUnreadable) {
			retCode = 1404
		}
		mylog.Printf("upload_media_image: 读取图片失败: %v", err)
		return sendActionResult(client, message, err.Error(), retCode)
	}

	fileName, contentType, err := normalizeMediaImageMeta(fileName, message.Params.ContentType)
	if err != nil {
		mylog.Printf("upload_media_image: 参数校验失败: %v", err)
		return sendActionResult(client, message, err.Error(), 1400)
	}

	if err := checkMediaImageSize(int64(len(content))); err != nil {
		mylog.Printf("upload_media_image: 大小校验失败: %v", err)
		return sendActionResult(client, message, err.Error(), 1400)
	}

	// md5 与 sha1 一律本地计算，不接受调用方传入
	md5Hash := computeMD5(content)
	sha1Hash := computeMediaImageSHA1(content)

	ctx := context.Background()
	prepareResp, err := apiv2.PrepareMediaImage(ctx, &dto.MediaImagePrepareRequest{
		FileName:    fileName,
		FileSize:    uint64(len(content)),
		ContentType: contentType,
		MD5:         md5Hash,
		SHA1:        sha1Hash,
	})
	if err != nil {
		mylog.Printf("upload_media_image: prepare 失败: %v", err)
		return sendActionResult(client, message, describeMediaImageError(err), 1401)
	}
	if prepareResp == nil || prepareResp.PresignedURL == "" {
		mylog.Printf("upload_media_image: prepare 未返回 presigned_url")
		return sendActionResult(client, message, "素材管理失败：prepare 未返回 presigned_url", 1401)
	}

	etag, err := putMediaImageContent(prepareResp.PresignedURL, contentType, content)
	if err != nil {
		mylog.Printf("upload_media_image: 直传失败: %v", err)
		return sendActionResult(client, message, err.Error(), 1401)
	}

	// complete 的 md5 取 PUT 响应 ETag 去引号；COS 未回 ETag 时回退本地 md5
	md5ForComplete := normalizeMediaImageETag(etag)
	if md5ForComplete == "" {
		mylog.Printf("upload_media_image: 直传响应缺少 ETag，回退本地 md5")
		md5ForComplete = md5Hash
	} else if md5ForComplete != md5Hash {
		mylog.Printf("upload_media_image: ETag(%s) 与本地 md5(%s) 不一致，complete 以 ETag 为准", md5ForComplete, md5Hash)
	}

	completeResp, err := apiv2.CompleteMediaImage(ctx, &dto.MediaImageCompleteRequest{
		UploadID: prepareResp.UploadID,
		MD5:      md5ForComplete,
	})
	if err != nil {
		mylog.Printf("upload_media_image: complete 失败: %v", err)
		return sendActionResult(client, message, describeMediaImageError(err), 1401)
	}
	if completeResp == nil {
		mylog.Printf("upload_media_image: complete 未返回内容")
		return sendActionResult(client, message, "素材管理失败：complete 未返回内容", 1401)
	}

	mylog.Printf("upload_media_image: 上传完成 image_id=%s ttl=%d", completeResp.ImageID, completeResp.TTL)
	return sendActionResultWithData(client, message, MediaImageUploadResponse{
		ImageID:    completeResp.ImageID,
		URL:        completeResp.URL,
		TTL:        completeResp.TTL,
		PreviewURL: completeResp.PreviewURL,
		FileName:   fileName,
		ExpiresIn:  prepareResp.ExpiresIn,
		ExpiresAt:  completeResp.ExpiresAt,
	})
}

// GetMediaImageList 素材管理：图片列表
// params: cursor(可选, 取上页 next_cursor)
func GetMediaImageList(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	if message.ShadowDryRun {
		// shadow 预演：不请求列表（只读接口，同样不比对新链结果）
		mylog.Printf("get_media_image_list: shadow dry-run，跳过上游请求")
		return sendActionResultWithData(client, message, newMediaImageDryRunResult("get_media_image_list"))
	}
	if apiv2 == nil {
		return sendActionResult(client, message, "素材管理失败：OpenAPI v2 未初始化", 1401)
	}

	list, err := apiv2.ListMediaImages(context.Background(), message.Params.Cursor)
	if err != nil {
		mylog.Printf("get_media_image_list: 拉取失败: %v", err)
		return sendActionResult(client, message, describeMediaImageError(err), 1401)
	}

	images := make([]MediaImageItem, 0)
	nextCursor := ""
	total := 0
	if list != nil {
		nextCursor = list.NextCursor
		total = list.Total
		for _, img := range list.Images {
			if img == nil {
				continue
			}
			images = append(images, MediaImageItem{
				ImageID:    img.ImageID,
				URL:        img.URL,
				FileName:   img.FileName,
				PreviewURL: img.PreviewURL,
				CreatedAt:  img.CreatedAt,
				ExpiresAt:  img.ExpiresAt,
			})
		}
	}

	mylog.Printf("get_media_image_list: 共 %d 张, total=%d", len(images), total)
	return sendActionResultWithData(client, message, MediaImageListResponse{
		Images:     images,
		NextCursor: nextCursor,
		Total:      total,
		Next:       nextCursor != "",
	})
}

// DeleteMediaImage 素材管理：删除图片（释放配额；已发送消息不受影响）
// params: image_id(必填)
func DeleteMediaImage(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	if message.ShadowDryRun {
		// shadow 预演：绝不真删（会释放配额且不可回滚），只回占位
		mylog.Printf("delete_media_image: shadow dry-run，跳过真实删除")
		return sendActionResultWithData(client, message, newMediaImageDryRunResult("delete_media_image"))
	}
	if apiv2 == nil {
		return sendActionResult(client, message, "素材管理失败：OpenAPI v2 未初始化", 1401)
	}

	imageID := strings.TrimSpace(message.Params.ImageID)
	if imageID == "" {
		return sendActionResult(client, message, "image_id 不能为空", 1400)
	}

	if err := apiv2.DeleteMediaImage(context.Background(), imageID); err != nil {
		mylog.Printf("delete_media_image: 删除 %s 失败: %v", imageID, err)
		return sendActionResult(client, message, describeMediaImageError(err), 1401)
	}

	mylog.Printf("delete_media_image: 已删除 image_id=%s", imageID)
	return sendActionResultWithData(client, message, struct{}{})
}

// ---------- 辅助函数 ----------

// loadMediaImageContent 按 file 入参前缀读取图片字节，并给出尽量准确的文件名
func loadMediaImageContent(file, fileName string) ([]byte, string, error) {
	raw := strings.TrimSpace(file)
	if raw == "" {
		return nil, "", fmt.Errorf("file 不能为空（支持 base64:// 前缀、data: 前缀、file:// 前缀或本地路径）")
	}

	lower := strings.ToLower(raw)
	switch {
	case strings.HasPrefix(lower, "http://"), strings.HasPrefix(lower, "https://"):
		// prepare 需要本地计算 md5/sha1，远端内容无法在不下载的前提下满足，明确拒绝而不是静默下载
		return nil, "", fmt.Errorf("file 不支持 http(s) 链接：素材管理需在本地计算 md5 与 sha1，请传本地路径或 base64")
	case strings.HasPrefix(raw, "base64://"):
		data, err := base64.StdEncoding.DecodeString(strings.TrimPrefix(raw, "base64://"))
		if err != nil {
			return nil, "", fmt.Errorf("base64 解码失败: %v", err)
		}
		return data, fileName, nil
	case strings.HasPrefix(lower, "data:"):
		return decodeMediaImageDataURI(raw, fileName)
	}

	localPath := trimFilePrefix(raw)
	info, err := os.Stat(localPath)
	if err != nil {
		return nil, "", fmt.Errorf("%w: %v", errMediaImageFileUnreadable, err)
	}
	if info.IsDir() {
		return nil, "", fmt.Errorf("%w: %s 是目录", errMediaImageFileUnreadable, localPath)
	}
	if err := checkMediaImageSize(info.Size()); err != nil {
		return nil, "", err
	}
	// 与 file_data 直传支路同源：arch_mode=new 时经 MediaService，否则回退 os.ReadFile
	data, err := readLocalMedia(localPath)
	if err != nil {
		return nil, "", fmt.Errorf("%w: %v", errMediaImageFileUnreadable, err)
	}
	if fileName == "" {
		fileName = filepath.Base(localPath)
	}
	return data, fileName, nil
}

// decodeMediaImageDataURI 解析 data:image/png;base64,xxxx 形式的入参
func decodeMediaImageDataURI(raw, fileName string) ([]byte, string, error) {
	idx := strings.Index(raw, ",")
	if idx < 0 {
		return nil, "", fmt.Errorf("data URI 缺少逗号分隔的内容体")
	}
	meta := strings.ToLower(raw[len("data:"):idx])
	payload := raw[idx+1:]
	if strings.Contains(meta, ";base64") {
		data, err := base64.StdEncoding.DecodeString(payload)
		if err != nil {
			return nil, "", fmt.Errorf("data URI base64 解码失败: %v", err)
		}
		return data, fileName, nil
	}
	// 非 base64 的 data URI 是百分号编码
	decoded, err := url.QueryUnescape(payload)
	if err != nil {
		return nil, "", fmt.Errorf("data URI 解码失败: %v", err)
	}
	return []byte(decoded), fileName, nil
}

// normalizeMediaImageMeta 归一文件名与 content_type：
// content_type 留空时按扩展名推导；扩展名与 content_type 必须同族（jpg/jpeg 算同族）
func normalizeMediaImageMeta(fileName, contentType string) (string, string, error) {
	fileName = strings.TrimSpace(fileName)
	if fileName == "" {
		fileName = fmt.Sprintf("media_image_%d", time.Now().Unix())
	}
	ct := strings.ToLower(strings.TrimSpace(contentType))
	ext := strings.ToLower(strings.TrimPrefix(filepath.Ext(fileName), "."))

	if ct == "" {
		mapped, ok := mediaImageExtContentTypes[ext]
		if !ok {
			return fileName, "", fmt.Errorf("无法从文件名 %q 推导 content_type，请显式传 image/png、image/jpeg、image/gif 或 image/webp", fileName)
		}
		ct = mapped
	}
	canonicalExt, ok := mediaImageContentTypes[ct]
	if !ok {
		return fileName, "", fmt.Errorf("content_type %q 不支持，仅支持 image/png、image/jpeg、image/gif、image/webp", ct)
	}
	if ext == "" {
		fileName = fileName + "." + canonicalExt
		ext = canonicalExt
	}
	mapped, ok := mediaImageExtContentTypes[ext]
	if !ok {
		return fileName, "", fmt.Errorf("扩展名 %q 不支持，仅支持 png/jpg/jpeg/gif/webp", ext)
	}
	if mapped != ct {
		return fileName, "", fmt.Errorf("扩展名 %q 与 content_type %q 不一致", ext, ct)
	}
	return fileName, ct, nil
}

// checkMediaImageSize 单图体积上限校验
func checkMediaImageSize(size int64) error {
	if size > maxMediaImageBytes {
		return fmt.Errorf("%w: %d 字节超过上限 %d 字节（10MB）", errMediaImageTooLarge, size, maxMediaImageBytes)
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

// putMediaImageContent 直传预签名 URL（COS）：
// 预签名 URL 自带鉴权，绝不能带 QQBot Authorization；Content-Type 必须与 prepare 声明完全一致，否则 COS 返回 403。
func putMediaImageContent(presignedURL, contentType string, data []byte) (string, error) {
	req, err := http.NewRequest(http.MethodPut, presignedURL, bytes.NewReader(data))
	if err != nil {
		return "", fmt.Errorf("素材图片直传失败：构造请求出错: %v", err)
	}
	req.Header.Set("Content-Type", contentType)

	client := &http.Client{Timeout: 120 * time.Second}
	resp, err := client.Do(req)
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
// 官方码原值保留在 message 里，便于调用方自行判定（40035010 = 跨机器人引用）。
func describeMediaImageError(err error) string {
	body := err.Error()
	var sdkErr *errs.Err
	if errors.As(err, &sdkErr) {
		body = sdkErr.Text()
	}
	for _, item := range mediaImageErrorCodes {
		if strings.Contains(body, fmt.Sprint(item.code)) {
			return fmt.Sprintf("素材管理失败(%d)：%s", item.code, item.text)
		}
	}
	return fmt.Sprintf("素材管理失败：%v", err)
}

package handlers

// 短签名图床外链的自取转传（2026-11-07 修复 oss_type=7 ChatGLM 图片在 QQ 不显示）。
//
// 根因：ChatGLM 图床外链带两层短签名（约 30 分钟 + 302 跳转后再叠 15 分钟），
// 直传给 QQ 官方 API（富媒体 URL 拉取 / Markdown 图片外链）时转存失败或查看时已过期。
// 修复：消费点自取字节后走既有上传链——
//   - 富媒体 url_image/url_images → 本地字节上传链（≤5,000,000 file_data 直传，超限自动分片，
//     复用 createAndUploadMediaMessage / createAndUploadMediaMessagePrivate，不新造上传实现）；
//   - Markdown / keyboard 外链 → 素材管理链（prepare→PUT→complete，mediaImageUploadFlow），
//     产出平台持久的 qqbot-media:// 引用替换原 URL。
//
// 降级语义：自取或上传任一步失败 → 记 warn、维持原 URL 直传（不比现状差），不阻断发送。

import (
	"context"
	"encoding/base64"
	"net/http"
	"strings"

	"github.com/hoshinonyaruko/gensokyo/imagehosting"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

// 注入缝（同 qqMusicSongInfo 口径，供 httptest 离线单测替换）。
// 约定：handlers 套件的用例一律禁止 t.Parallel()——下面三处注入缝与 config 单例、echo 计数
// 都是包级全局态，并行覆写会互相串扰，同包测试必须串行跑。
var (
	isShortLivedImageURL = imagehosting.IsShortLivedImageURL
	// selfFetchBytes 自取字节：SSRF 判据复用本包既有的 isPrivateOrLoopback
	//（首跳 + 每一跳重定向都复检，复检点见 imagehosting.FetchBytes）。
	selfFetchBytes = func(raw string) ([]byte, error) {
		return imagehosting.FetchBytes(raw, isPrivateOrLoopback)
	}
	// selfFetchSSRFBlocked 自取入口的私有/回环判定（同 isPrivateOrLoopback）：
	// 入口先判可以在建连前就降级并给出专门 warn，FetchBytes 的逐跳复检是第二层防线。
	selfFetchSSRFBlocked = isPrivateOrLoopback
)

// selfFetchImageURL 还原 legacy foundItems 剥离 scheme 后的完整 URL：
// url_image 消费点补 http、url_images 消费点补 https（与原直传分支口径一致）；
// 已带 scheme 的值（cqparse avatar 全 URL 透传形态）原样返回。
func selfFetchImageURL(value, defaultScheme string) string {
	if strings.HasPrefix(value, "http://") || strings.HasPrefix(value, "https://") {
		return value
	}
	return defaultScheme + "://" + value
}

// trySelfFetchGroupImage 群聊富媒体消费点：命中短签名域则自取字节并走本地字节上传链。
// 返回 (msg, true) 表示已转传；未命中域名或任一步失败返回 (nil, false)，调用方维持原逻辑。
func trySelfFetchGroupImage(rawURL, eventid, groupid, id string, msgseq int, apiv2 openapi.OpenAPI) (*dto.MessageToCreate, bool) {
	if !isShortLivedImageURL(rawURL) {
		return nil, false
	}
	if selfFetchSSRFBlocked(rawURL) {
		mylog.Printf("[shortlived] 群图片目标为私有/回环地址，拒绝自取并降级原外链直传: %v", rawURL)
		return nil, false
	}
	data, err := selfFetchBytes(rawURL)
	if err != nil {
		mylog.Printf("[shortlived] 群图片自取失败，降级原外链直传: %v err=%v", rawURL, err)
		return nil, false
	}
	// 与素材链同口径：魔数不是受支持的图片格式就不进上传链（防被污染的外链回非图片字节）
	if _, ext := sniffMediaImageFormat(data); ext == "" {
		mylog.Printf("[shortlived] 群自取内容不是受支持的图片格式（仅 png/jpeg/gif/webp），降级原外链直传: %v size=%d", rawURL, len(data))
		return nil, false
	}
	base64Encoded := base64.StdEncoding.EncodeToString(data)
	mtc, err := createAndUploadMediaMessage(context.TODO(), base64Encoded, eventid, 1, false, "", groupid, id, msgseq, apiv2)
	if err != nil {
		mylog.Printf("[shortlived] 群自取图片上传 QQ CDN 失败，降级原外链直传: %v err=%v", rawURL, err)
		return nil, false
	}
	mylog.Printf("[shortlived] 群图片已自取转传: size=%d url=%s", len(data), rawURL)
	return mtc, true
}

// trySelfFetchPrivateImage 单聊富媒体消费点，语义同 trySelfFetchGroupImage
//（字节上传走 createAndUploadMediaMessagePrivate）。
func trySelfFetchPrivateImage(rawURL, eventid, userid, id string, msgseq int, apiv2 openapi.OpenAPI) (*dto.MessageToCreate, bool) {
	if !isShortLivedImageURL(rawURL) {
		return nil, false
	}
	if selfFetchSSRFBlocked(rawURL) {
		mylog.Printf("[shortlived] 私聊图片目标为私有/回环地址，拒绝自取并降级原外链直传: %v", rawURL)
		return nil, false
	}
	data, err := selfFetchBytes(rawURL)
	if err != nil {
		mylog.Printf("[shortlived] 私聊图片自取失败，降级原外链直传: %v err=%v", rawURL, err)
		return nil, false
	}
	// 与素材链同口径：魔数不是受支持的图片格式就不进上传链
	if _, ext := sniffMediaImageFormat(data); ext == "" {
		mylog.Printf("[shortlived] 私聊自取内容不是受支持的图片格式（仅 png/jpeg/gif/webp），降级原外链直传: %v size=%d", rawURL, len(data))
		return nil, false
	}
	base64Encoded := base64.StdEncoding.EncodeToString(data)
	mtc, err := createAndUploadMediaMessagePrivate(context.TODO(), base64Encoded, eventid, 1, false, "", userid, id, msgseq, apiv2)
	if err != nil {
		mylog.Printf("[shortlived] 私聊自取图片上传 QQ CDN 失败，降级原外链直传: %v err=%v", rawURL, err)
		return nil, false
	}
	mylog.Printf("[shortlived] 私聊图片已自取转传: size=%d url=%s", len(data), rawURL)
	return mtc, true
}

// selfFetchToMediaReference Markdown / keyboard 消费点：短签名外链自取字节后走素材链，
// 返回平台持久的 qqbot-media:// 引用。未命中域名返回 ("", false)（零开销原样透传，
// 行为与其他域一致）；命中但被 SSRF 判据拒绝、或任一步失败同样返回 ("", false)，
// 由调用方维持原 URL。
func selfFetchToMediaReference(apiv2 openapi.OpenAPI, mediaPath string) (string, bool) {
	if !isShortLivedImageURL(mediaPath) {
		return "", false
	}
	if selfFetchSSRFBlocked(mediaPath) {
		mylog.Printf("[shortlived] Markdown 图片目标为私有/回环地址，拒绝自取并维持原外链: %v", mediaPath)
		return "", false
	}
	data, err := selfFetchBytes(mediaPath)
	if err != nil {
		mylog.Printf("[shortlived] Markdown 图片自取失败，维持原外链: %v err=%v", mediaPath, err)
		return "", false
	}
	return uploadImageBytesToMediaStore(apiv2, data)
}

// uploadImageBytesToMediaStore 内存字节走素材链（prepare→PUT→complete）换 qqbot-media:// 引用，
// 供短签名外链自取后与「图床已回短签名 URL 的本地产物」两种场景复用。
// 任一步失败返回 ("", false)，调用方维持原 URL（降级语义）。
func uploadImageBytesToMediaStore(apiv2 openapi.OpenAPI, data []byte) (string, bool) {
	if apiv2 == nil {
		// 全特性唯一的静默失败路径：没有 apiv2 就没有素材链，记 warn 让排障看得见
		mylog.Printf("[shortlived] apiv2 未注入，跳过素材链上传，维持原外链: size=%d", len(data))
		return "", false
	}
	if err := checkMediaImageSize(int64(len(data))); err != nil {
		mylog.Printf("[shortlived] 图片超过素材链大小上限，维持原外链: %v", err)
		return "", false
	}
	contentType, ext := sniffMediaImageFormat(data)
	if ext == "" {
		mylog.Printf("[shortlived] 图片格式不受素材链支持（仅 png/jpeg/gif/webp），维持原外链")
		return "", false
	}
	resp, err := mediaImageUploadFlow(context.TODO(), apiv2, data, "selffetch."+ext, contentType, "shortlived")
	if err != nil {
		mylog.Printf("[shortlived] 素材链上传失败，维持原外链: %v", err)
		return "", false
	}
	if !strings.HasPrefix(resp.URL, "qqbot-media://") {
		mylog.Printf("[shortlived] 素材链未回 qqbot-media 引用，维持原外链: url=%q", resp.URL)
		return "", false
	}
	mylog.Printf("[shortlived] 图片已转素材引用: image_id=%s size=%d", resp.ImageID, len(data))
	return resp.URL, true
}

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

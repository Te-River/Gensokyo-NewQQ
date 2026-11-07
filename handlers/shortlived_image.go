package handlers

// 短签名图床外链的自取转传（2026-11-07 修复 oss_type=7 ChatGLM 图片在 QQ 不显示）。
//
// 根因：ChatGLM 图床外链带两层短签名（约 30 分钟 + 302 跳转后再叠 15 分钟），
// 直传给 QQ 官方 API（富媒体 URL 拉取 / Markdown 图片外链）时转存失败或查看时已过期。
// 修复：消费点自取字节后走既有上传链——
//   - 富媒体 url_image/url_images → 本地字节上传链（≤5,000,000 file_data 直传，超限自动分片，
//     复用 images.CreateAndUploadMediaMessage / CreateAndUploadMediaMessagePrivate，不新造上传实现）；
//   - Markdown / keyboard 外链 → 复用 ResolveMarkdownImages 本地图片分支那条既有外链上传流程
//     （images.UploadBase64ImageToServer），把自取到的字节当"本地字节的图"重新上传换外链。
//
// 与 main 版差异：015 基线无素材管理 API（无 qqbot-media:// 链），也无 outbound_bridge 预投递，
// Markdown/keyboard 只能落"自取→既有外链上传流程"；自取返回值在 legacy 媒体链路里由
// *dto.MessageToCreate 既有分支接住（图文混合预投递与 sendGroupMsgKeyMap 循环），不产生二次发送。
// oss_type=7 时 UploadBase64ImageToServer 已把图床短签名外链回退为本地公网 URL，故这里期望
// 拿到稳定外链（仍回短链说明回退缺失，判失败维持原外链）。
//
// 降级语义：自取或上传任一步失败 → 记 warn、维持原 URL 直传（不比现状差），不阻断发送。
// SSRF 防护：入口先过 selfFetchSSRFBlocked（私有/回环即拒），FetchBytes 内首跳与
// 每一跳重定向再复检 isPrivateOrLoopback；上传前按魔数嗅探格式，非图字节不进上传链。

import (
	"context"
	"encoding/base64"
	"net/http"
	"strings"

	"github.com/hoshinonyaruko/gensokyo/imagehosting"
	"github.com/hoshinonyaruko/gensokyo/images"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

// 注入缝（供 httptest 离线单测替换真实网络与真实图床）。
// 约定：handlers 套件的用例一律禁止 t.Parallel()——注入缝与 config 单例、echo 计数
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
	// markdownImageUpload 就是 Markdown/keyboard 本地图片分支使用的既有外链上传流程，
	// 挂成注入缝只为离线单测可替换，不新造上传实现。
	markdownImageUpload = images.UploadBase64ImageToServer
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
	// 上传前格式嗅探：魔数不是受支持的图片格式就不进上传链（防被污染的外链回非图字节）
	if sniffImageFormat(data) == "" {
		mylog.Printf("[shortlived] 群自取内容不是受支持的图片格式（仅 png/jpeg/gif/webp），降级原外链直传: %v size=%d", rawURL, len(data))
		return nil, false
	}
	base64Encoded := base64.StdEncoding.EncodeToString(data)
	mtc, err := images.CreateAndUploadMediaMessage(context.TODO(), base64Encoded, eventid, 1, false, "", groupid, id, msgseq, apiv2)
	if err != nil {
		mylog.Printf("[shortlived] 群自取图片上传 QQ CDN 失败，降级原外链直传: %v err=%v", rawURL, err)
		return nil, false
	}
	mylog.Printf("[shortlived] 群图片已自取转传: size=%d url=%s", len(data), rawURL)
	return mtc, true
}

// trySelfFetchPrivateImage 单聊富媒体消费点，语义同 trySelfFetchGroupImage
// （字节上传走 images.CreateAndUploadMediaMessagePrivate）。
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
	// 上传前格式嗅探：魔数不是受支持的图片格式就不进上传链
	if sniffImageFormat(data) == "" {
		mylog.Printf("[shortlived] 私聊自取内容不是受支持的图片格式（仅 png/jpeg/gif/webp），降级原外链直传: %v size=%d", rawURL, len(data))
		return nil, false
	}
	base64Encoded := base64.StdEncoding.EncodeToString(data)
	mtc, err := images.CreateAndUploadMediaMessagePrivate(context.TODO(), base64Encoded, eventid, 1, false, "", userid, id, msgseq, apiv2)
	if err != nil {
		mylog.Printf("[shortlived] 私聊自取图片上传 QQ CDN 失败，降级原外链直传: %v err=%v", rawURL, err)
		return nil, false
	}
	mylog.Printf("[shortlived] 私聊图片已自取转传: size=%d url=%s", len(data), rawURL)
	return mtc, true
}

// selfFetchToStableImageURL Markdown / keyboard 消费点：短签名外链自取字节后，
// 走本地图片那条既有上传流程重新上传换外链。未命中域名返回 ("", false)（零开销原样透传，
// 行为与其他域一致）；命中但被 SSRF 判据拒绝、或任一步失败同样返回 ("", false)，
// 由调用方维持原 URL。
func selfFetchToStableImageURL(apiv2 openapi.OpenAPI, mediaPath string) (string, bool) {
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
	return uploadImageBytesToPublicURL(apiv2, data)
}

// uploadImageBytesToPublicURL 内存字节走既有外链上传流程（base64 → UploadBase64ImageToServer），
// 与 ResolveMarkdownImages 本地文件分支同一条链，产出本仓库形态的外链。
// 任一步失败返回 ("", false)，调用方维持原 URL（降级语义）。
func uploadImageBytesToPublicURL(apiv2 openapi.OpenAPI, data []byte) (string, bool) {
	// 上传前格式嗅探：魔数不是受支持的图片格式就不进上传链（防被污染的外链回非图字节）
	if sniffImageFormat(data) == "" {
		mylog.Printf("[shortlived] 自取内容不是受支持的图片格式（仅 png/jpeg/gif/webp），维持原外链: size=%d", len(data))
		return "", false
	}
	base64Encoded := base64.StdEncoding.EncodeToString(data)
	cdnURL, _, _, err := markdownImageUpload(base64Encoded, apiv2)
	if err != nil {
		mylog.Printf("[shortlived] 自取图片走本地上传流程失败，维持原外链: size=%d err=%v", len(data), err)
		return "", false
	}
	if cdnURL == "" {
		mylog.Printf("[shortlived] 本地上传流程返回空外链，维持原外链: size=%d", len(data))
		return "", false
	}
	if isShortLivedImageURL(cdnURL) {
		// 不变量：UploadBase64ImageToServer 已把图床短签名外链回退为本地公网 URL。
		// 仍回短签名域说明上传链新增了未纳入回退的 provider：拒绝替换，
		// 维持原外链（接近过期但可用）并留痕，不把新短链当成稳定外链写进 Markdown。
		mylog.Printf("[shortlived] 上传流程仍回短签名外链，拒绝替换并维持原外链: size=%d url=%s", len(data), cdnURL)
		return "", false
	}
	mylog.Printf("[shortlived] 图片已自取转传为外链: size=%d url=%s", len(data), cdnURL)
	return cdnURL, true
}

// sniffImageFormat 按魔数嗅探图片格式，仅认 png/jpeg/gif/webp 四种；
// 无法识别（含 text/html 等被污染外链的响应体）返回空串，由调用方拒绝进上传链。
func sniffImageFormat(data []byte) string {
	switch http.DetectContentType(data) {
	case "image/png":
		return "png"
	case "image/jpeg":
		return "jpg"
	case "image/gif":
		return "gif"
	case "image/webp":
		return "webp"
	}
	return ""
}

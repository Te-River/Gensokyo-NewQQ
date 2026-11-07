package handlers

// 短签名图床外链的自取转传（2026-11-07 修复 oss_type=7 ChatGLM 图片在 QQ 不显示）。
//
// 根因：ChatGLM 图床外链带两层短签名（约 30 分钟 + 302 跳转后再叠 15 分钟），
// 直传给 QQ 官方 API（富媒体 URL 拉取 / Markdown 图片外链）时转存失败或查看时已过期。
// 修复：消费点自取字节后走既有上传链——
//   - 富媒体 url_image/url_images → 本地字节上传链（≤5,000,000 file_data 直传，超限自动分片，
//     复用 createAndUploadMediaMessage / createAndUploadMediaMessagePrivate，不新造上传实现）；
//   - Markdown / keyboard 外链 → 复用 ResolveMarkdownImages 本地图片分支那条既有外链上传流程
//     （images.UploadBase64ImageToServer），把自取到的字节当"本地字节的图"重新上传换外链。
//
// 与 Refactor 版差异：main 未接入素材管理 API（无 qqbot-media:// 链），Markdown/keyboard
// 只能落"自取→既有外链上传流程"；oss_type=7 时 UploadBase64ImageToServer 优先把图床短签名
// 外链解析为 302 直链，解析失败才回退本地公网 URL；降级路径内若仍拿到短链，再解析一次，
// 仍失败才拒绝替换并维持原外链。
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
	// resolveShortLivedURL 短签名外链 → 302 末跳直链（只读响应头，首跳与每一跳都过
	// isPrivateOrLoopback；复检点见 imagehosting.ResolveRedirectURL）。
	resolveShortLivedURL = func(raw string) (string, error) {
		return imagehosting.ResolveRedirectURL(raw, isPrivateOrLoopback)
	}
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
	mtc, err := createAndUploadMediaMessage(context.TODO(), base64Encoded, eventid, 1, false, "", groupid, id, msgseq, apiv2)
	if err != nil {
		mylog.Printf("[shortlived] 群自取图片上传 QQ CDN 失败，降级原外链直传: %v err=%v", rawURL, err)
		return nil, false
	}
	mylog.Printf("[shortlived] 群图片已自取转传: size=%d url=%s", len(data), rawURL)
	return mtc, true
}

// trySelfFetchPrivateImage 单聊富媒体消费点，语义同 trySelfFetchGroupImage
// （字节上传走 createAndUploadMediaMessagePrivate）。
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
	mtc, err := createAndUploadMediaMessagePrivate(context.TODO(), base64Encoded, eventid, 1, false, "", userid, id, msgseq, apiv2)
	if err != nil {
		mylog.Printf("[shortlived] 私聊自取图片上传 QQ CDN 失败，降级原外链直传: %v err=%v", rawURL, err)
		return nil, false
	}
	mylog.Printf("[shortlived] 私聊图片已自取转传: size=%d url=%s", len(data), rawURL)
	return mtc, true
}

// selfFetchToStableImageURL Markdown / keyboard 消费点：短签名外链优先解析 302 到最终直链
//（OSS 预签名，约 15 分钟有效），出站只需 URL，不必下载字节再上传；解析失败再走
//「自取字节 → 既有上传流程换外链」的降级链。未命中域名返回 ("", false)（零开销原样透传，
// 行为与其他域一致）；命中但解析与自取转传都失败同样返回 ("", false)，由调用方维持原 URL。
func selfFetchToStableImageURL(apiv2 openapi.OpenAPI, mediaPath string) (string, bool) {
	if !isShortLivedImageURL(mediaPath) {
		return "", false
	}
	// ① 解析直链：QQ 不跟随短链的 302，直链则可直接拉取；比自取字节少一次下载与一次上传。
	if directURL, err := resolveShortLivedURL(mediaPath); err == nil {
		mylog.Printf("[shortlived] Markdown 图片短签名外链已解析为直链: %s -> %s", mediaPath, directURL)
		return directURL, true
	} else {
		mylog.Printf("[shortlived] Markdown 图片直链解析失败，回退自取转传链: %v err=%v", mediaPath, err)
	}
	// ② 解析失败 → 自取字节后走既有上传流程换外链。
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
		// 上传链自身优先返回解析后的 302 直链，本地公网 URL 只是解析失败后的次级回退；
		// 仍回短签名域说明该 provider 未纳入上传链的直链解析——降级路径内再解析一次，
		// 成功就采纳直链，失败才拒绝替换（维持接近过期但可用的原外链并留痕）。
		if directURL, err := resolveShortLivedURL(cdnURL); err == nil {
			mylog.Printf("[shortlived] 降级路径内层直链解析成功，采纳: %s -> %s", cdnURL, directURL)
			return directURL, true
		} else {
			mylog.Printf("[shortlived] 上传流程仍回短签名外链且直链解析失败，拒绝替换并维持原外链: size=%d url=%s err=%v", len(data), cdnURL, err)
			return "", false
		}
	}
	mylog.Printf("[shortlived] 图片已自取转传为外链: size=%d url=%s", len(data), cdnURL)
	return cdnURL, true
}

// sniffImageFormat 按魔数嗅探图片格式，仅认 png/jpeg/gif/webp 四种；
// 无法识别（含 text/html 等被污染外链的响应体）返回空串，由调用方拒绝进上传链。
// main 无 Refactor 的素材链嗅探函数（sniffMediaImageFormat 属素材链），
// 这里以 http.DetectContentType 实现同口径的最小判定。
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

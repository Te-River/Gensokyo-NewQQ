package handlers

// 短签名图床外链的自取转传（2026-11-07 修复 oss_type=7 ChatGLM 图片在 QQ 不显示）。
//
// 根因：ChatGLM 图床外链带两层短签名（约 30 分钟 + 302 跳转后再叠 15 分钟），
// 直传给 QQ 官方 API（富媒体 URL 拉取 / Markdown 图片外链）时转存失败或查看时已过期。
// 修复：消费点自取字节后走既有上传链——
//   - 富媒体 url_image/url_images → 本地字节上传链（≤5,000,000 file_data 直传，超限自动分片，
//     复用 images.CreateAndUploadMediaMessage / CreateAndUploadMediaMessagePrivate，不新造上传实现）；
//   - Markdown / keyboard 外链 → 素材管理链（prepare→PUT→complete，见 media_asset.go），
//     产出平台素材引用（滑动 90 天，可复用）qqbot-media:// 替换原 URL；素材链不可用时才退回直链解析/既有外链上传。
//
// 与 main 版差异：015 基线无 outbound_bridge 预投递，自取返回值在 legacy 媒体链路里由
// *dto.MessageToCreate 既有分支接住（图文混合预投递与 sendGroupMsgKeyMap 循环），不产生二次发送。
// oss_type=7 下 UploadBase64ImageToServer 只作为素材链失败后的兜底：优先把图床短签名外链解析为
// 302 直链，解析失败才回退本地公网 URL；降级路径内若仍拿到短链，再解析一次，仍失败才拒绝替换并维持原外链。
//
// 降级语义：自取或上传任一步失败 → 记 warn、维持原 URL 直传（不比现状差），不阻断发送。
// SSRF 防护：入口先过 selfFetchSSRFBlocked（私有/回环即拒），FetchBytes 内首跳与
// 每一跳重定向再复检 isPrivateOrLoopback；上传前按魔数嗅探格式，非图字节不进上传链。

import (
	"context"
	"encoding/base64"
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

// selfFetchToStableImageURL Markdown / keyboard 消费点：短签名外链自取字节后优先走素材链，
// 换平台素材引用（滑动 90 天，可复用）qqbot-media://；素材链不可用时退回直链解析与既有外链上传换外链。
// 未命中域名返回 ("", false)（零开销原样透传，行为与其他域一致）；命中但被 SSRF 判据拒绝、
// 或全链失败同样返回 ("", false)，由调用方维持原 URL。
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
		// 自取失败时先退回直链解析（只读响应头，成本低）：解析出的直链仍可交给平台直传
		directURL, resolveErr := resolveShortLivedURL(mediaPath)
		if resolveErr == nil {
			mylog.Printf("[shortlived] Markdown 图片自取失败，改用直链: %v err=%v", mediaPath, err)
			return directURL, true
		}
		mylog.Printf("[shortlived] Markdown 图片自取与直链解析均失败，维持原外链: %v err=%v resolveErr=%v", mediaPath, err, resolveErr)
		return "", false
	}
	// ① 素材链：平台素材引用（滑动 90 天，可复用），不依赖外链能被 QQ 拉取（短签名直链同样会被平台拒绝）
	if ref, ok := uploadImageBytesToMediaStore(apiv2, data, "shortlived"); ok {
		return ref, true
	}
	// ② 兜底一：直链解析（只读响应头，成本低）
	if directURL, err := resolveShortLivedURL(mediaPath); err == nil {
		mylog.Printf("[shortlived] 素材链不可用，Markdown 图片改用直链: %s -> %s", mediaPath, directURL)
		return directURL, true
	} else {
		mylog.Printf("[shortlived] 素材链与直链解析均不可用，回退既有外链上传: %v err=%v", mediaPath, err)
	}
	// ③ 兜底二：既有"上传外链"链路（含短链再解析）
	return uploadImageBytesToPublicURL(apiv2, data)
}

// uploadImageBytesToPublicURL 自取字节走既有外链上传流程（Markdown / keyboard 的兜底路径）：
// 魔数门禁 / 空外链守卫 / 短链再解析这三条规则与本地图片分支共用 uploadBytesToPublicURL，
// 这里只固定两处差异——自取字节必须先过魔数门禁，且不做素材链升级（素材链已在上游试过）。
// 任一步失败返回 ("", false)，调用方维持原 URL（降级语义）。
func uploadImageBytesToPublicURL(apiv2 openapi.OpenAPI, data []byte) (string, bool) {
	return uploadBytesToPublicURL(apiv2, data, true, "")
}

// sniffImageFormat 按魔数嗅探图片扩展名，仅认 png/jpeg/gif/webp 四种；
// 无法识别（含 text/html 等被污染外链的响应体）返回空串，由调用方拒绝进上传链。
func sniffImageFormat(data []byte) string {
	_, ext := sniffMediaImageFormat(data)
	return ext
}

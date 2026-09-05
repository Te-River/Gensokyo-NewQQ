package cqparse

import (
	"net/url"
	"path/filepath"
	"runtime"
	"strings"
)

// 媒体类 handler：image / record(voice) / video / file / avatar（架构设计 §7）。
// 修 M2（参数切分式解析，附加参数不再污染 URL）；
// 修 m4（值含逗号不截断、未知前缀入 unknown_* 不再泄漏）；
// 修 M5（avatar 逐 Token 独立反查，产物直接入 foundItems 不回写正文）。

type mediaHandler struct{ action string }

func (h mediaHandler) Kind() Kind { return h.kindOf() }
func (h mediaHandler) kindOf() Kind {
	if h.action == "avatar" {
		return KindContent
	}
	return KindMedia
}
func (mediaHandler) Scope() Scope { return ScopeGroup | ScopePrivate | ScopeForward }

func init() {
	for _, a := range []string{"image", "record", "voice", "video", "file", "avatar"} {
		Register(a, mediaHandler{action: a})
	}
}

func (h mediaHandler) Resolve(ctx *ResolveCtx, tok Token) Outcome {
	switch h.action {
	case "image", "record", "voice", "video":
		return resolveMediaValue(h.action, tok)
	case "file":
		return resolveFileValue(tok)
	case "avatar":
		return resolveAvatar(ctx, tok)
	}
	return Outcome{Replacement: tok.Raw}
}

// resolveMediaValue 解析 image/record/voice/video 的 file（兼容 url）参数。
func resolveMediaValue(kind string, tok Token) Outcome {
	// 修 C-fix：voice 与 record 同义，foundItems 键统一为 *_record——
	// legacy 管道（case "voice","record"）与全部下游消费者只认 record 系键
	if kind == "voice" {
		kind = "record"
	}
	file := tok.Params["file"]
	if file == "" {
		// 兼容部分客户端使用 url 而非 file
		file = tok.Params["url"]
	}
	if file == "" {
		// 修 Minor：裸码（无 file/url 参数）legacy 保留字面，new 对齐不静默移除
		return Outcome{Replacement: tok.Raw}
	}
	var key, value string
	switch {
	case strings.HasPrefix(file, "base64://"):
		key, value = "base64_"+kind, strings.TrimPrefix(file, "base64://")
	case strings.HasPrefix(file, "http://"):
		key, value = "url_"+kind, strings.TrimPrefix(file, "http://")
	case strings.HasPrefix(file, "https://"):
		key, value = "url_"+kind+"s", strings.TrimPrefix(file, "https://")
	case strings.HasPrefix(file, "file://"):
		safePath, err := resolveLocalMediaPath(file)
		if err != nil {
			// 安全校验失败：码移除 + 日志，不泄漏不注入
			return Outcome{Replacement: "", Warn: "[cqparse] 安全校验失败,跳过本地" + kind + ": " + err.Error()}
		}
		key, value = "local_"+kind, safePath
	default:
		// M2：go-cqhttp 风格文件名（xx.image）与无法识别的值入 unknown_*，码不泄漏
		key, value = "unknown_"+kind, file
	}
	return Outcome{
		Replacement: "",
		Found:       []FoundItem{{Key: key, Value: value}},
	}
}

// resolveFileValue 解析 [CQ:file,file=xxx,file_name=yyy]。
func resolveFileValue(tok Token) Outcome {
	file := tok.Params["file"]
	if file == "" {
		// 修 Minor：裸码（无 file 参数）legacy 保留字面，new 对齐不静默移除
		return Outcome{Replacement: tok.Raw}
	}
	fileName := tok.Params["file_name"]
	var key, value string
	switch {
	case strings.HasPrefix(file, "file://"):
		safePath, err := resolveLocalMediaPath(file)
		if err != nil {
			return Outcome{Replacement: "", Warn: "[cqparse] 安全校验失败,跳过本地文件: " + err.Error()}
		}
		key, value = "local_file", safePath
	case strings.HasPrefix(file, "http://"):
		key, value = "url_file", strings.TrimPrefix(file, "http://")
	case strings.HasPrefix(file, "https://"):
		key, value = "url_files", strings.TrimPrefix(file, "https://")
	case strings.HasPrefix(file, "base64://"):
		key, value = "base64_file", strings.TrimPrefix(file, "base64://")
	default:
		// m4：未知前缀不再 return match 泄漏
		key, value = "unknown_file", file
	}
	found := []FoundItem{{Key: key, Value: value}}
	if fileName != "" {
		found = append(found, FoundItem{Key: "file_name", Value: fileName})
	}
	return Outcome{Replacement: "", Found: found}
}

// resolveAvatar 头像码：逐 Token 独立反查（修 M5 两头像同 URL），
// 产物直接入 foundItems，不再生成 [CQ:image,...] 回写正文。
func resolveAvatar(ctx *ResolveCtx, tok Token) Outcome {
	qq := tok.Params["qq"]
	if qq == "" {
		return Outcome{Replacement: tok.Raw}
	}
	if ctx.Deps == nil || ctx.Deps.AvatarURL == nil {
		return Outcome{Replacement: "", Warn: "[cqparse] AvatarURL 未注入,已丢弃 avatar 码: " + tok.Raw}
	}
	u, err := ctx.Deps.AvatarURL(qq, ctx.Input.GroupID, ctx.Input.HasGroup)
	if err != nil || u == "" {
		// 反查失败兜底：码移除 + 日志，不产出破损 URL（修 avatar.go 反查失败掩盖问题）
		return Outcome{Replacement: "", Warn: "[cqparse] avatar 反查失败,已丢弃: qq=" + qq + " err=" + err.Error()}
	}
	if strings.HasPrefix(u, "http://") {
		return Outcome{Replacement: "", Found: []FoundItem{{Key: "url_image", Value: u}}}
	}
	// https:// 全 URL 与生产侧已剥离 scheme 的 URL 均入 url_images（值原样透传）
	return Outcome{Replacement: "", Found: []FoundItem{{Key: "url_images", Value: u}}}
}

// ---------- 本地媒体路径安全校验（与 handlers.resolveLocalMedia 同源，仅依赖 stdlib） ----------

// trimFilePrefix 剥离 file:// 协议前缀（区分 Windows 的 file:/// 和 Unix 的 file://）
func trimFilePrefix(fileContent string) string {
	if runtime.GOOS == "windows" {
		return strings.TrimPrefix(fileContent, "file:///")
	}
	return strings.TrimPrefix(fileContent, "file://")
}

// resolveLocalMediaPath 解析 file:// 本地路径，返回安全化的绝对路径。
func resolveLocalMediaPath(fileContent string) (string, error) {
	cleanContent := trimFilePrefix(fileContent)
	// URL 解码（如 %E7%A5%9E → 神）
	decoded, err := url.PathUnescape(cleanContent)
	if err != nil {
		return "", err
	}
	// 明确拒绝包含 .. 的路径（须在 Clean 之前检查）
	if strings.Contains(decoded, "..") {
		return "", errUnsafeLocalPath
	}
	abs, err := filepath.Abs(filepath.Clean(decoded))
	if err != nil {
		return "", err
	}
	return abs, nil
}

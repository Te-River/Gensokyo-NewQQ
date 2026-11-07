// 短时效签名图床 URL 的仓库内自取
// ChatGLM（oss_type=7）等图床返回的外链带两层短签名（约 30 分钟 + 跳转后 15 分钟），
// 直传给 QQ 官方 API 时服务端转存拉取失败、或用户查看时签名已过期，导致图片不显示。
// 消费点先用 IsShortLivedImageURL 判定域名，再三级处理：ResolveRedirectURL 解析 302 直链
// → 自取字节走本地公网 URL 上传链 → 原行为，不把短签名外链交给 QQ。
package imagehosting

import (
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strings"
	"time"
)

// shortLivedImageHostSuffixes 需自取转传的图床域名清单（按 host 边界匹配，含其子域）。
var shortLivedImageHostSuffixes = []string{"chatglm.cn"}

// selfFetchMaxBytes 单次自取的字节上限（十进制 20MB）。
// 包级变量便于单测注入小上限；超限视为自取失败，由调用方走降级语义。
var selfFetchMaxBytes int64 = 20 * 1000 * 1000

// selfFetchTimeout 单次自取的总超时（覆盖整条重定向链与读体），与 providerHTTPClient 同口径。
const selfFetchTimeout = 30 * time.Second

// selfFetchMaxHops 自取的重定向跳数上限（不含首跳），与 handlers 包级 httpClient 的口径一致。
const selfFetchMaxHops = 10

// resolveMaxHops 直链解析的重定向跳数上限（不含首跳）：短签名外链的正常形态是 1 跳
//（302 → OSS 预签名直链），5 跳足以覆盖多级 CDN 跳转，超出视为异常交给调用方降级。
const resolveMaxHops = 5

// resolveTimeout 直链解析的总超时：只取响应头、不读图片字节，10 秒远宽于正常两跳。
const resolveTimeout = 10 * time.Second

// selfFetchUA 浏览器 UA：部分图床对非浏览器 UA 拒答。
const selfFetchUA = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36"

// SSRFGuard 是 FetchBytes 的 SSRF 判定回调：入参为即将访问的完整 URL，返回 true 表示禁止访问。
// 生产口径由消费方注入（handlers 包复用其既有的 isPrivateOrLoopback，含域名解析），
// 本包不内置判据，避免同一口径在两处各写一遍。
type SSRFGuard func(rawURL string) bool

// IsShortLivedImageURL 判断图片 URL 是否为需要仓库内自取转传的短签名外链。
// 入参可以是完整 URL，也可以是 legacy foundItems 已剥离 scheme 的形式。
// 按 host 边界判定：host 等于清单域名、或以 ".域名" 后缀结尾（覆盖 t1./pfiles. 等子域）才为真，
// evil-chatglm.cn.example.com 这类拼接口径不会误判。
func IsShortLivedImageURL(u string) bool {
	host := imageURLHost(u)
	if host == "" {
		return false
	}
	for _, suffix := range shortLivedImageHostSuffixes {
		if host == suffix || strings.HasSuffix(host, "."+suffix) {
			return true
		}
	}
	return false
}

// imageURLHost 解析 URL 的 host（小写、去端口）；解析不出 host 时返回空串。
//
// 判据是「解析结果有没有 Host」，不是字符串里有没有 "//"：legacy 已剥 scheme 的裸域值，
// 其 path/query 完全可以含 "//"（如 pfiles.chatglm.cn/oss/a.png?sign=abc//def、
// t1.chatglm.cn:443/a.png?sig=a//b），用 Contains("//") 判有无 scheme 会跳过补全，
// Host 解析为空 → 自取静默不触发，恰是本次要修的形态。
// 先按原样 url.Parse；仅当结果没有 Host 时才补 "//" 前缀重解一次，覆盖三种畸形：
//   - 裸域（无 scheme）：Scheme 与 Host 都空；
//   - "域:端口/路径"：Go 会把首段字母开头的 "t1.chatglm.cn" 当成 Scheme、Host 为空；
//   - "IP:端口/路径"：Go 直接报「first path segment in URL cannot contain colon」。
func imageURLHost(raw string) string {
	raw = strings.TrimSpace(raw)
	if raw == "" {
		return ""
	}
	if parsed, err := url.Parse(raw); err == nil && parsed.Host != "" {
		return strings.ToLower(parsed.Hostname())
	}
	parsed, err := url.Parse("//" + raw)
	if err != nil {
		return ""
	}
	return strings.ToLower(parsed.Hostname())
}

// ensureFetchScheme 还原自取用的完整 URL：
//   - 已带 scheme 且 Host 可解析：原样返回（scheme 的大小写由 Go 的 url.Parse 归一化，
//     真正非法的形态交给 Do 报错，由调用方降级）；
//   - 协议相对形态（"//host/path"）：补 https；
//   - legacy foundItems 剥离 scheme 的裸域形态：按 https 补全（短签名图床外链均为 https）；
//   - 连 host 都解析不出来：原样返回，交给 http.NewRequest/Do 报错降级。
func ensureFetchScheme(raw string) string {
	if parsed, err := url.Parse(raw); err == nil && parsed.Host != "" {
		if parsed.Scheme != "" {
			return raw
		}
		parsed.Scheme = "https"
		return parsed.String()
	}
	if imageURLHost(raw) != "" {
		return "https://" + raw
	}
	return raw
}

// FetchBytes GET 拉取 URL 并返回响应字节：
// 用自取专用客户端（总超时 selfFetchTimeout，覆盖整条重定向链与读体；显式跳数上限 selfFetchMaxHops），
// 默认跟随重定向（chatglm 302 → OSS 预签名正是两跳场景），带浏览器 UA，读取上限 selfFetchMaxBytes。
// reject 为调用方注入的 SSRF 判据：首跳与每一跳重定向目标都复检，命中即拒绝——
// 被污染的图床域名（DNS/重定向可控）会把跳转指向 169.254.169.254 等内网元数据地址，
// 响应字节还会被上传到 QQ CDN 变成可见图片（读侧外带），所以这里不能走裸 client。
// reject 为 nil 视为调用方疏漏，直接报错，不发无防护的请求。
// 失败返回错误，调用方记录 warn 后维持原 URL 直传（降级语义）。
func FetchBytes(raw string, reject SSRFGuard) ([]byte, error) {
	if raw == "" {
		return nil, fmt.Errorf("图片自取: URL 为空")
	}
	if reject == nil {
		return nil, fmt.Errorf("图片自取: 缺少 SSRF 判定回调，拒绝发起请求")
	}
	raw = ensureFetchScheme(raw)
	if reject(raw) {
		return nil, fmt.Errorf("图片自取: SSRF 阻止: 首跳目标为私有/回环地址: %s", raw)
	}
	req, err := http.NewRequest(http.MethodGet, raw, nil)
	if err != nil {
		return nil, fmt.Errorf("图片自取: 创建请求失败: %w", err)
	}
	req.Header.Set("User-Agent", selfFetchUA)

	client := &http.Client{
		Timeout: selfFetchTimeout,
		CheckRedirect: func(req *http.Request, via []*http.Request) error {
			if len(via) >= selfFetchMaxHops {
				return fmt.Errorf("图片自取: 重定向次数超过上限 %d", selfFetchMaxHops)
			}
			if reject(req.URL.String()) {
				return fmt.Errorf("图片自取: SSRF 阻止: 重定向目标为私有/回环地址: %s", req.URL.String())
			}
			return nil
		},
	}

	resp, err := client.Do(req)
	if err != nil {
		return nil, fmt.Errorf("图片自取: 请求失败: %w", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("图片自取: 返回 HTTP %d: %s", resp.StatusCode, raw)
	}
	data, err := io.ReadAll(io.LimitReader(resp.Body, selfFetchMaxBytes+1))
	if err != nil {
		return nil, fmt.Errorf("图片自取: 读取响应失败: %w", err)
	}
	if int64(len(data)) > selfFetchMaxBytes {
		return nil, fmt.Errorf("图片自取: 响应超过大小上限 %d 字节: %s", selfFetchMaxBytes, raw)
	}
	return data, nil
}

// ResolveRedirectURL 把带重定向的短签名外链解析为最终直链，返回末跳地址（resp.Request.URL）。
//
// 只读响应头：请求带 Range: bytes=0-0（图床至多回 1 字节），拿到响应后立即 Close Body、
// 不读任何正文——短链的 302 目标就是 OSS 预签名直链（约 15 分钟有效），出站只需要这个 URL，
// 下载整图纯属白费内存与带宽（QQ 拉取成功后会自行转存）。
//
// reject 与 FetchBytes 同口径：首跳与每一跳重定向都复检，命中即拒绝——被污染的图床域名
// 可以把跳转指向 169.254.169.254 等内网元数据地址，读侧外带同样成立，不能走裸 client。
// reject 为 nil 视为调用方疏漏，直接报错。
//
// 未发生重定向（末跳地址与入参相同）按失败返回：调用方要的是能替代短链的直链，
// 把同一个短链交回去没有意义，必须让它去走降级链。
func ResolveRedirectURL(raw string, reject SSRFGuard) (string, error) {
	if raw == "" {
		return "", fmt.Errorf("图片直链解析: URL 为空")
	}
	if reject == nil {
		return "", fmt.Errorf("图片直链解析: 缺少 SSRF 判定回调，拒绝发起请求")
	}
	raw = ensureFetchScheme(raw)
	if reject(raw) {
		return "", fmt.Errorf("图片直链解析: SSRF 阻止: 首跳目标为私有/回环地址: %s", raw)
	}
	req, err := http.NewRequest(http.MethodGet, raw, nil)
	if err != nil {
		return "", fmt.Errorf("图片直链解析: 创建请求失败: %w", err)
	}
	req.Header.Set("User-Agent", selfFetchUA)
	req.Header.Set("Range", "bytes=0-0")

	client := &http.Client{
		Timeout: resolveTimeout,
		CheckRedirect: func(req *http.Request, via []*http.Request) error {
			if len(via) >= resolveMaxHops {
				return fmt.Errorf("图片直链解析: 重定向次数超过上限 %d", resolveMaxHops)
			}
			if reject(req.URL.String()) {
				return fmt.Errorf("图片直链解析: SSRF 阻止: 重定向目标为私有/回环地址: %s", req.URL.String())
			}
			return nil
		},
	}

	resp, err := client.Do(req)
	if err != nil {
		return "", fmt.Errorf("图片直链解析: 请求失败: %w", err)
	}
	// 只取 resp.Request.URL，正文一个字节都不读。
	resp.Body.Close()

	if resp.StatusCode < 200 || resp.StatusCode > 299 {
		return "", fmt.Errorf("图片直链解析: 末跳返回 HTTP %d: %s", resp.StatusCode, raw)
	}
	final := resp.Request.URL.String()
	if final == raw {
		return "", fmt.Errorf("图片直链解析: 未发生重定向，原链仍是短签名外链: %s", raw)
	}
	return final, nil
}

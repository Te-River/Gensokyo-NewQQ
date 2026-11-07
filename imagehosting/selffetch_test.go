package imagehosting

// 短签名图床自取工具的离线回归：域名边界判定、无 scheme 判据（Major-1）、
// 302 两跳跟随、UA、状态码与大小上限、SSRF 逐跳复检与跳数上限（Major-2）。
// 全部 httptest 离线，不触真实网络；约定与 handlers 套件一致，用例不得并行。

import (
	"net/http"
	"net/http/httptest"
	"net/url"
	"strconv"
	"strings"
	"sync/atomic"
	"testing"
)

// allowAllGuard 放行全部目标：以下用例只验自取链本身的行为（两跳跟随、UA、状态码、大小上限）。
// SSRF 判据由调用方注入（生产口径是 handlers.isPrivateOrLoopback，本包不内置），
// 判据生效与否由后面三个 reject 用例钉死。
var allowAllGuard SSRFGuard = func(string) bool { return false }

// rejectHosts 返回按主机名精确拒绝的判据（离线用例点名被污染的目标，不做真实 DNS 解析）。
func rejectHosts(hosts ...string) SSRFGuard {
	blocked := make(map[string]bool, len(hosts))
	for _, h := range hosts {
		blocked[strings.ToLower(h)] = true
	}
	return func(rawURL string) bool {
		parsed, err := url.Parse(rawURL)
		if err != nil {
			return true // 解析失败一律拒绝，与 isPrivateOrLoopback 同口径
		}
		return blocked[strings.ToLower(parsed.Hostname())]
	}
}

func TestIsShortLivedImageURL(t *testing.T) {
	cases := []struct {
		name string
		url  string
		want bool
	}{
		{"chatglm 子域完整外链", "https://t1.chatglm.cn/file/abc.png?expired_at=1730000000&sign=xyz&ext=png", true},
		{"chatglm pfiles 子域", "https://pfiles.chatglm.cn/oss/x.png?x-oss-expires=899", true},
		{"chatglm 裸域", "https://chatglm.cn/a.png", true},
		{"已剥 scheme 的 foundItems 形态", "t1.chatglm.cn/file/a.png?sign=x&expired_at=1", true},
		{"带端口的子域", "https://t1.chatglm.cn:443/a.png", true},
		{"大写 host", "https://T1.ChatGLM.cn/A.PNG", true},
		// Major-1 回归：裸域（legacy 已剥 scheme）的 path/query 含 "//"，
		// 旧判据 Contains(raw,"//") 会当成「已带 scheme」跳过补全 → Host 为空 → 自取静默不触发。
		{"裸域 query 含 //（Major-1）", "pfiles.chatglm.cn/oss/a.png?sign=abc//def", true},
		{"裸域带端口且 query 含 //（Major-1）", "t1.chatglm.cn:443/a.png?sig=a//b", true},
		{"裸域 path 含 //（Major-1）", "chatglm.cn/oss//a.png?sign=x", true},
		{"协议相对形态且 query 含 //", "//pfiles.chatglm.cn/oss/a.png?x=y//z", true},
		{"scheme 大小写混拼（判据看 Host，Go 归一化 scheme）", "htTps://pfiles.chatglm.cn/a.png", true},
		{"IP 形态带端口不得误判成图床", "127.0.0.1:8080/a.png?x=y//z", false},
		{"拼接前缀伪装不得误判", "https://evil-chatglm.cn.example.com/a.png", false},
		{"后缀反转伪装", "https://chatglm.cn.evil.org/a.png", false},
		{"相似域名", "https://notchatglm.cn/a.png", false},
		{"bilibili 图床", "https://i0.hdslb.com/bfs/xxx.png", false},
		{"example.com", "example.com", false},
		{"qq.com", "https://qq.com/a.png", false},
		{"空串", "", false},
		{"仅路径", "/file/a.png", false},
		{"空白串", "   ", false},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			if got := IsShortLivedImageURL(c.url); got != c.want {
				t.Errorf("IsShortLivedImageURL(%q) = %v, want %v", c.url, got, c.want)
			}
		})
	}
}

// TestEnsureFetchScheme 自取 URL 的 scheme 补全口径：判据同样是「解析结果有无 Host」，
// 不再是 Contains(raw,"//")；已带 scheme 的值原样交给 http（大小写混拼由 Go 归一化）。
func TestEnsureFetchScheme(t *testing.T) {
	cases := []struct {
		in, want string
	}{
		{"https://t1.chatglm.cn/a.png", "https://t1.chatglm.cn/a.png"},
		{"http://t1.chatglm.cn/a.png?x=y//z", "http://t1.chatglm.cn/a.png?x=y//z"},
		{"pfiles.chatglm.cn/oss/a.png?sign=abc//def", "https://pfiles.chatglm.cn/oss/a.png?sign=abc//def"},
		{"t1.chatglm.cn:443/a.png?sig=a//b", "https://t1.chatglm.cn:443/a.png?sig=a//b"},
		{"//pfiles.chatglm.cn/a.png?x=y//z", "https://pfiles.chatglm.cn/a.png?x=y//z"},
		{"htTps://pfiles.chatglm.cn/a.png", "htTps://pfiles.chatglm.cn/a.png"},
		// 连 host 都解析不出来：原样返回，交给 NewRequest/Do 报错降级
		{"/file/a.png", "/file/a.png"},
		{"", ""},
	}
	for _, c := range cases {
		if got := ensureFetchScheme(c.in); got != c.want {
			t.Errorf("ensureFetchScheme(%q) = %q, want %q", c.in, got, c.want)
		}
	}
}

// TestFetchBytesTwoHopRedirect 模拟 ChatGLM 外链 302 → OSS 预签名两跳，断言拿到最终字节。
func TestFetchBytesTwoHopRedirect(t *testing.T) {
	want := []byte("\x89PNG\r\n\x1a\nselffetch-two-hop-body")
	var ossHit bool
	var ossUA string
	ossSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		ossHit = true
		ossUA = r.Header.Get("User-Agent")
		if !strings.Contains(r.URL.RawQuery, "x-oss-expires=899") {
			t.Errorf("OSS 预签名参数丢失: %s", r.URL.RawQuery)
		}
		w.Header().Set("Content-Type", "image/png")
		w.Write(want)
	}))
	defer ossSrv.Close()

	var signUA string
	signSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		signUA = r.Header.Get("User-Agent")
		http.Redirect(w, r, ossSrv.URL+"/file/signed.png?x-oss-expires=899&sign=abc", http.StatusFound)
	}))
	defer signSrv.Close()

	got, err := FetchBytes(signSrv.URL+"/file/abc.png?expired_at=1&sign=xyz&ext=png", allowAllGuard)
	if err != nil {
		t.Fatalf("FetchBytes 失败: %v", err)
	}
	if string(got) != string(want) {
		t.Fatalf("字节不符: got %d bytes, want %d bytes", len(got), len(want))
	}
	if !ossHit {
		t.Fatal("未跟随 302 到达 OSS 一跳")
	}
	if !strings.HasPrefix(signUA, "Mozilla/") || !strings.HasPrefix(ossUA, "Mozilla/") {
		t.Errorf("两跳都必须携带浏览器 UA: sign=%q oss=%q", signUA, ossUA)
	}
}

// TestFetchBytesNilGuardRejected 缺 SSRF 判据时必须拒绝发起，而不是退化成裸请求（静默失守）。
func TestFetchBytesNilGuardRejected(t *testing.T) {
	if _, err := FetchBytes("https://t1.chatglm.cn/a.png", nil); err == nil {
		t.Fatal("reject 为 nil 必须报错（不得发无防护的裸请求）")
	} else if !strings.Contains(err.Error(), "SSRF") {
		t.Fatalf("错误信息需说明 SSRF 判据缺失: %v", err)
	}
}

// TestFetchBytesRejectsPrivateFirstHop 首跳指向回环/私有地址 → 判据命中即拒，不发起任何请求。
func TestFetchBytesRejectsPrivateFirstHop(t *testing.T) {
	var hit int32
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		atomic.AddInt32(&hit, 1)
		w.Write(slPNGBytes())
	}))
	defer srv.Close()

	_, err := FetchBytes(srv.URL+"/file/a.png?sign=abc//def", rejectHosts("127.0.0.1"))
	if err == nil {
		t.Fatal("首跳为回环地址必须拒绝")
	}
	if !strings.Contains(err.Error(), "SSRF 阻止") {
		t.Fatalf("错误信息需含 SSRF 阻止: %v", err)
	}
	if n := atomic.LoadInt32(&hit); n != 0 {
		t.Fatalf("拒绝后不得发起请求，实际命中 %d 次", n)
	}
}

// TestFetchBytesRejectsPrivateRedirectTarget Major-2 核心场景：
// 被污染的 *.chatglm.cn 用 302 把跳转指向内网元数据地址 169.254.169.254（读侧外带），
// 每一跳都必须复检，命中即拒，且不得真的连上去。
func TestFetchBytesRejectsPrivateRedirectTarget(t *testing.T) {
	const metaURL = "http://169.254.169.254/latest/meta-data/iam/security-credentials/"
	var redirectChecked []string
	guard := func(rawURL string) bool {
		if strings.Contains(rawURL, "169.254.169.254") {
			redirectChecked = append(redirectChecked, rawURL)
			return true
		}
		return false
	}

	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		http.Redirect(w, r, metaURL, http.StatusFound)
	}))
	defer srv.Close()

	_, err := FetchBytes(srv.URL+"/file/abc.png?sign=glm", guard)
	if err == nil {
		t.Fatal("302 指向内网元数据地址必须拒绝")
	}
	if !strings.Contains(err.Error(), "重定向目标") {
		t.Fatalf("错误信息需说明是重定向跳被拒: %v", err)
	}
	if len(redirectChecked) == 0 {
		t.Fatal("CheckRedirect 未对重定向目标复检 SSRF")
	}
}

// TestFetchBytesRedirectHopLimit 显式跳数上限（≤10）：每跳都不同 URL，排除 Go 自带的同址保护。
func TestFetchBytesRedirectHopLimit(t *testing.T) {
	var hops int32
	var srv *httptest.Server
	srv = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		n := atomic.AddInt32(&hops, 1)
		if n > 64 { // 防御：判据失效时不要让测试无限跳
			w.WriteHeader(http.StatusInsufficientStorage)
			return
		}
		http.Redirect(w, r, srv.URL+"/loop?n="+strconv.Itoa(int(n)), http.StatusFound)
	}))
	defer srv.Close()

	_, err := FetchBytes(srv.URL+"/file/a.png?sign=x", allowAllGuard)
	if err == nil {
		t.Fatalf("超过跳数上限必须报错（实际跳了 %d 次）", atomic.LoadInt32(&hops))
	}
	if !strings.Contains(err.Error(), "重定向次数超过上限") {
		t.Fatalf("错误信息需含跳数上限: %v", err)
	}
	if n := atomic.LoadInt32(&hops); n > selfFetchMaxHops {
		t.Fatalf("跳数必须被限制在 %d 以内，实际 %d", selfFetchMaxHops, n)
	}
}

// TestFetchBytesMixedCaseSchemeDegradesSafely 大小写混拼的 scheme（htTps://）：
// Go 的 url.Parse 会把 scheme 归一化为 https（不会静默降级成明文 HTTP），
// 对明文 httptest 服务发 https 必然失败 → 返回错误由调用方降级原外链，不 panic。
func TestFetchBytesMixedCaseSchemeDegradesSafely(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.Write(slPNGBytes())
	}))
	defer srv.Close()

	target := "htTps" + strings.TrimPrefix(srv.URL, "http") + "/file/a.png?sign=x"
	data, err := FetchBytes(target, allowAllGuard)
	if err == nil {
		t.Fatalf("混拼 scheme 对明文服务发 https 必须报错（由调用方降级），实际拿到 %d 字节", len(data))
	}
	if data != nil {
		t.Fatalf("报错时不得返回字节: %d", len(data))
	}
}

// TestFetchBytesStatusError 上游非 200 → 报错（调用方降级）。
func TestFetchBytesStatusError(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusInternalServerError)
	}))
	defer srv.Close()

	if _, err := FetchBytes(srv.URL+"/a.png?sign=1", allowAllGuard); err == nil {
		t.Fatal("HTTP 500 必须报错（由调用方降级）")
	} else if !strings.Contains(err.Error(), "HTTP 500") {
		t.Fatalf("错误信息需含状态码: %v", err)
	}
}

// TestFetchBytesSizeLimit 超过 selfFetchMaxBytes → 报错（调用方降级）。
func TestFetchBytesSizeLimit(t *testing.T) {
	old := selfFetchMaxBytes
	selfFetchMaxBytes = 8
	defer func() { selfFetchMaxBytes = old }()

	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Write([]byte("0123456789")) // 10 > 8
	}))
	defer srv.Close()

	if _, err := FetchBytes(srv.URL+"/big.png", allowAllGuard); err == nil {
		t.Fatal("超过 selfFetchMaxBytes 必须报错")
	} else if !strings.Contains(err.Error(), "超过大小上限") {
		t.Fatalf("错误信息需含大小上限说明: %v", err)
	}
}

// TestFetchBytesEmptyURL 空 URL 直接报错，不触网络。
func TestFetchBytesEmptyURL(t *testing.T) {
	if _, err := FetchBytes("", allowAllGuard); err == nil {
		t.Fatal("空 URL 必须报错")
	}
}

// TestSelfFetchRealMatcherTwoHopIntegration 端到端（离线）：用真实 IsShortLivedImageURL
// 判定（不是 handlers 侧的 slMatcher 替换）驱动真实 FetchBytes 走完 302 两跳。
// 域名清单是包内变量，这里临时把 httptest 的回环主机名并入清单，
// 从而在不触真实网络的前提下跑完「判定 → 两跳自取 → 字节」全链；用例结束即还原。
func TestSelfFetchRealMatcherTwoHopIntegration(t *testing.T) {
	oldSuffixes := shortLivedImageHostSuffixes
	shortLivedImageHostSuffixes = append(append([]string{}, oldSuffixes...), "127.0.0.1")
	defer func() { shortLivedImageHostSuffixes = oldSuffixes }()

	want := slPNGBytes()
	ossSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "image/png")
		w.Write(want)
	}))
	defer ossSrv.Close()

	// query 里带 "//"：正是 Major-1 里会让旧判据失灵的形态
	target := ossSrv.URL + "/oss/a.png?sign=abc//def"
	if !IsShortLivedImageURL(target) {
		t.Fatalf("真实判据应命中清单域名: %q", target)
	}
	stripped := strings.TrimPrefix(target, "http://")
	if !IsShortLivedImageURL(stripped) {
		t.Fatalf("真实判据应命中「裸域 + query 含 //」形态: %q", stripped)
	}

	got, err := FetchBytes(target, allowAllGuard)
	if err != nil {
		t.Fatalf("真实判据下的自取失败: %v", err)
	}
	if string(got) != string(want) {
		t.Fatalf("字节不符: got %d want %d", len(got), len(want))
	}
}

// slPNGBytes 带 PNG 魔数的假图字节（离线用例的统一响应体）。
func slPNGBytes() []byte {
	return []byte("\x89PNG\r\n\x1a\nselffetch-offline-body")
}

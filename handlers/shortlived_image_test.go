package handlers

// 短签名图床外链（oss_type=7 ChatGLM）自取转传回归（main 形态，无素材链）：
// 富媒体 302 两跳自取 → 本地字节上传（非 URL 直传）；Markdown / keyboard 外链 →
// 自取字节后走本地图片那条既有上传流程换外链；自取/上传失败 → 降级维持原外链直传；
// SSRF 私有/回环目标 → 入口拒绝并降级；非图字节 → 嗅探拒绝进上传链；
// 图文混合（SendFoundItems*）预投递 → 一次自取、一条消息。全部 httptest 离线，不触真实网络。
// 约定：handlers 套件用例禁用 t.Parallel()（注入缝与 config、echo 都是包级全局态）。

import (
	"context"
	"encoding/base64"
	"errors"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/imagehosting"
	"github.com/hoshinonyaruko/gensokyo/template"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/dto/keyboard"
	"github.com/tencent-connect/botgo/openapi"
)

// slPNG 带 PNG 魔数的假图字节（http.DetectContentType 识别为 image/png）。
func slPNG() []byte {
	return []byte("\x89PNG\r\n\x1a\nshortlived-selffetch-body")
}

// allowLoopbackGuard 离线自取用的 SSRF 放行缝（httptest 后端是回环地址，真实判据会拒）。
var allowLoopbackGuard imagehosting.SSRFGuard = func(string) bool { return false }

// withShortLivedSeams 覆写判定注入缝并保留真实 FetchBytes（对 httptest 走真实两跳 GET）。
// 两处 SSRF 缝（入口 selfFetchSSRFBlocked、FetchBytes 的逐跳判据）在这里放行：
// 真实判据是 isPrivateOrLoopback，会把 httptest 的回环首跳/每一跳一律拒掉；
// 自取链本身（imagehosting.FetchBytes + 专用 client + 两跳 GET）仍是真实代码路径，
// SSRF 闸门另有专门用例钉死（TestShortLivedSelfFetchSSRFRejectsPrivateTarget）。
func withShortLivedSeams(t *testing.T, matcher func(string) bool) {
	t.Helper()
	oldMatch, oldFetch, oldSSRF := isShortLivedImageURL, selfFetchBytes, selfFetchSSRFBlocked
	isShortLivedImageURL = matcher
	selfFetchBytes = func(raw string) ([]byte, error) {
		return imagehosting.FetchBytes(raw, allowLoopbackGuard)
	}
	selfFetchSSRFBlocked = func(string) bool { return false }
	t.Cleanup(func() {
		isShortLivedImageURL, selfFetchBytes, selfFetchSSRFBlocked = oldMatch, oldFetch, oldSSRF
	})
}

// slMatcher 只匹配本用例的 httptest 外链（真实域名边界判定见 imagehosting 包测试）。
func slMatcher(want string) func(string) bool {
	return func(u string) bool { return u == want }
}

// slUploadSeam 记录 Markdown / keyboard 外链转传触达的既有上传流程调用（base64 入参）。
type slUploadSeam struct {
	gotBase64 []string
	wantURL   string
	wantErr   error
}

func (s *slUploadSeam) upload(_ string, apiv2 openapi.OpenAPI) (string, int, int, error) {
	return s.wantURL, 0, 0, s.wantErr
}

// withShortLivedUploadSeam 覆写外链上传注入缝，用例结束后恢复。
func withShortLivedUploadSeam(t *testing.T, seam *slUploadSeam) {
	t.Helper()
	old := markdownImageUpload
	markdownImageUpload = func(base64Image string, apiv2 openapi.OpenAPI) (string, int, int, error) {
		seam.gotBase64 = append(seam.gotBase64, base64Image)
		return seam.upload(base64Image, apiv2)
	}
	t.Cleanup(func() { markdownImageUpload = old })
}

// withShortLivedResolveSeam 覆写直链解析注入缝，用例结束后恢复。
func withShortLivedResolveSeam(t *testing.T, resolve func(string) (string, error)) {
	t.Helper()
	old := resolveShortLivedURL
	resolveShortLivedURL = resolve
	t.Cleanup(func() { resolveShortLivedURL = old })
}

// newShortLivedUpstream 模拟 ChatGLM 外链两跳：外链 302 → OSS 预签名 → PNG 字节。
func newShortLivedUpstream(t *testing.T) string {
	t.Helper()
	var ossHit bool
	var ossUA string
	ossSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		ossHit = true
		ossUA = r.Header.Get("User-Agent")
		if !strings.Contains(r.URL.RawQuery, "x-oss-expires=899") {
			t.Errorf("OSS 预签名参数丢失: %s", r.URL.RawQuery)
		}
		w.Header().Set("Content-Type", "image/png")
		w.Write(slPNG())
	}))
	t.Cleanup(ossSrv.Close)

	var signUA string
	signSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		signUA = r.Header.Get("User-Agent")
		http.Redirect(w, r, ossSrv.URL+"/file/signed.png?x-oss-expires=899&sign=oss", http.StatusFound)
	}))
	t.Cleanup(signSrv.Close)
	t.Cleanup(func() {
		if !ossHit {
			t.Error("未跟随 302 走到 OSS 第二跳")
		}
		if !strings.HasPrefix(signUA, "Mozilla/") || !strings.HasPrefix(ossUA, "Mozilla/") {
			t.Errorf("两跳都必须携带浏览器 UA: sign=%q oss=%q", signUA, ossUA)
		}
	})
	return signSrv.URL + "/file/abc.png?expired_at=1730000000&sign=glm&ext=png"
}

// slMockAPI 记录富媒体上传调用（main 无素材链，只需群/C2C 两个方法）。
type slMockAPI struct {
	openapi.OpenAPI

	groupMsgs []dto.APIMessage
	c2cMsgs   []dto.APIMessage
}

func (m *slMockAPI) PostGroupMessage(_ context.Context, _ string, msg dto.APIMessage) (*dto.GroupMessageResponse, error) {
	m.groupMsgs = append(m.groupMsgs, msg)
	return &dto.GroupMessageResponse{MediaResponse: &dto.MediaResponse{FileInfo: "group-file-info"}}, nil
}

func (m *slMockAPI) PostC2CMessage(_ context.Context, _ string, msg dto.APIMessage) (*dto.C2CMessageResponse, error) {
	m.c2cMsgs = append(m.c2cMsgs, msg)
	return &dto.C2CMessageResponse{MediaResponse: &dto.MediaResponse{FileInfo: "c2c-file-info"}}, nil
}

// slLoadTemplateConfig 以完整模板加载 config 单例（url_pic_transfer 模板默认 false，
// 降级用例据此断言「原外链直传」分支可达）。
func slLoadTemplateConfig(t *testing.T) {
	t.Helper()
	path := filepath.Join(t.TempDir(), "config.yml")
	if err := os.WriteFile(path, []byte(template.ConfigTemplate), 0600); err != nil {
		t.Fatalf("写入测试配置失败: %v", err)
	}
	if _, err := config.LoadConfig(path, false); err != nil {
		t.Fatalf("加载测试配置失败: %v", err)
	}
}

func assertBytesUploaded(t *testing.T, msg dto.APIMessage, wantB64 string) *dto.RichMediaMessage {
	t.Helper()
	rich, ok := msg.(*dto.RichMediaMessage)
	if !ok {
		t.Fatalf("上传载体应为 RichMediaMessage，实际 %T", msg)
	}
	if rich.URL != "" {
		t.Errorf("已自取转传后不得再 URL 直传: URL=%q", rich.URL)
	}
	if rich.FileData != wantB64 {
		t.Errorf("file_data 应为自取字节的 base64: got %d want %d 字节", len(rich.FileData), len(wantB64))
	}
	return rich
}

// TestShortLivedGroupImageSelfFetchUsesLocalBytesUpload 群聊 url_image（剥离 scheme 形态）：
// 302 两跳自取 → file_data 本地字节上传 → 返回 msg_type=7 的 MessageToCreate。
func TestShortLivedGroupImageSelfFetchUsesLocalBytesUpload(t *testing.T) {
	imgURL := newShortLivedUpstream(t)
	withShortLivedSeams(t, slMatcher(imgURL))
	mock := &slMockAPI{}
	stripped := strings.TrimPrefix(imgURL, "http://")

	got := generateGroupMessage("mid-1", "eid-1", map[string][]string{"url_image": {stripped}}, "", 7, mock, "group-openid")
	mtc, ok := got.(*dto.MessageToCreate)
	if !ok {
		t.Fatalf("应走本地字节上传返回 MessageToCreate，实际 %T: %+v", got, got)
	}
	if mtc.MsgType != 7 || mtc.Media == nil || mtc.Media.FileInfo != "group-file-info" {
		t.Fatalf("msg_type=7 / file_info 不符: %+v", mtc)
	}
	if len(mock.groupMsgs) != 1 {
		t.Fatalf("应恰好一次 QQ CDN 上传，实际 %d", len(mock.groupMsgs))
	}
	wantB64 := base64.StdEncoding.EncodeToString(slPNG())
	if rich := assertBytesUploaded(t, mock.groupMsgs[0], wantB64); rich.FileType != 1 {
		t.Errorf("FileType 应为图片(1)，实际 %d", rich.FileType)
	}
}

// TestShortLivedGroupImagesBranchAcceptsSchemedValue 群聊 url_images 带完整 scheme 的值
// （cqparse avatar 透传形态）同样命中自取转传。
func TestShortLivedGroupImagesBranchAcceptsSchemedValue(t *testing.T) {
	imgURL := newShortLivedUpstream(t)
	withShortLivedSeams(t, slMatcher(imgURL))
	mock := &slMockAPI{}

	got := generateGroupMessage("mid-2", "eid-2", map[string][]string{"url_images": {imgURL}}, "", 3, mock, "group-openid")
	mtc, ok := got.(*dto.MessageToCreate)
	if !ok || mtc.Media == nil || mtc.Media.FileInfo != "group-file-info" {
		t.Fatalf("url_images 带 scheme 值应自取转传，实际 %T: %+v", got, got)
	}
	if len(mock.groupMsgs) != 1 {
		t.Fatalf("应恰好一次上传，实际 %d", len(mock.groupMsgs))
	}
	assertBytesUploaded(t, mock.groupMsgs[0], base64.StdEncoding.EncodeToString(slPNG()))
}

// TestShortLivedPrivateImageSelfFetch 单聊消费点同口径（createAndUploadMediaMessagePrivate）。
func TestShortLivedPrivateImageSelfFetch(t *testing.T) {
	imgURL := newShortLivedUpstream(t)
	withShortLivedSeams(t, slMatcher(imgURL))
	mock := &slMockAPI{}
	stripped := strings.TrimPrefix(imgURL, "http://")

	got := generatePrivateMessage("mid-3", "eid-3", map[string][]string{"url_image": {stripped}}, "", 5, mock, "user-openid")
	mtc, ok := got.(*dto.MessageToCreate)
	if !ok || mtc.Media == nil || mtc.Media.FileInfo != "c2c-file-info" {
		t.Fatalf("私聊应走本地字节上传，实际 %T: %+v", got, got)
	}
	if len(mock.c2cMsgs) != 1 {
		t.Fatalf("应恰好一次 C2C 上传，实际 %d", len(mock.c2cMsgs))
	}
	assertBytesUploaded(t, mock.c2cMsgs[0], base64.StdEncoding.EncodeToString(slPNG()))
}

// TestShortLivedGroupImageFetchFailureDegrades 自取 500 → 记 warn 后维持原 URL 直传
// （url_pic_transfer=false 的既有直传语义，不比现状差、不阻断发送）。
func TestShortLivedGroupImageFetchFailureDegrades(t *testing.T) {
	slLoadTemplateConfig(t)
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		http.Error(w, "boom", http.StatusInternalServerError)
	}))
	defer srv.Close()
	imgURL := srv.URL + "/file/gone.png?expired_at=1&sign=x"
	withShortLivedSeams(t, slMatcher(imgURL))

	mock := &slMockAPI{}
	stripped := strings.TrimPrefix(imgURL, "http://")
	got := generateGroupMessage("mid-4", "eid-4", map[string][]string{"url_image": {stripped}}, "", 1, mock, "group-openid")
	rich, ok := got.(*dto.RichMediaMessage)
	if !ok {
		t.Fatalf("自取失败应降级为原 URL 直传，实际 %T: %+v", got, got)
	}
	if rich.URL != imgURL {
		t.Fatalf("降级必须维持原 URL 原样: %q", rich.URL)
	}
	if len(mock.groupMsgs) != 0 {
		t.Fatal("降级路径不得触达字节上传")
	}
}

// TestResolveMarkdownImagesShortLivedRewrittenToUploadedURL Markdown 短签名外链：
// 自取两跳 → 把字节喂进本地图片那条既有上传流程 → 用换到的外链替换原 URL。
func TestResolveMarkdownImagesShortLivedRewrittenToUploadedURL(t *testing.T) {
	imgURL := newShortLivedUpstream(t)
	withShortLivedSeams(t, slMatcher(imgURL))
	const stableURL = "https://cdn.example.com/selffetch-stable.png"
	seam := &slUploadSeam{wantURL: stableURL}
	withShortLivedUploadSeam(t, seam)

	in := "看看 ![梗图](" + imgURL + ") 完毕"
	out := ResolveMarkdownImages(in, &slMockAPI{})
	want := "看看 ![梗图](" + stableURL + ") 完毕"
	if out != want {
		t.Fatalf("短签名外链应替换为自取转传后的外链:\n got: %s\nwant: %s", out, want)
	}
	if len(seam.gotBase64) != 1 {
		t.Fatalf("应恰好一次本地上传流程调用，实际 %d", len(seam.gotBase64))
	}
	// 上传流程收到的必须是自取到的字节（302 两跳后的 PNG），而不是原 URL
	if gotB64 := seam.gotBase64[0]; gotB64 != base64.StdEncoding.EncodeToString(slPNG()) {
		t.Fatalf("上传流程入参应为自取字节的 base64: got %d 字节", len(gotB64))
	}
}

// TestResolveMarkdownImagesShortLivedUploadFailureKeepsOriginal 既有上传流程失败 →
// 维持原外链（降级语义），但已按链路尝试过自取与上传。
func TestResolveMarkdownImagesShortLivedUploadFailureKeepsOriginal(t *testing.T) {
	imgURL := newShortLivedUpstream(t)
	withShortLivedSeams(t, slMatcher(imgURL))
	seam := &slUploadSeam{wantErr: errors.New("图床上传失败")}
	withShortLivedUploadSeam(t, seam)

	in := "![a](" + imgURL + ")"
	out := ResolveMarkdownImages(in, &slMockAPI{})
	if out != in {
		t.Fatalf("上传失败应维持原外链: got %q", out)
	}
	if len(seam.gotBase64) != 1 {
		t.Fatalf("应尝试过一次既有上传流程，实际 %d", len(seam.gotBase64))
	}
}

// TestResolveMarkdownImagesOtherDomainsUnchanged 其他域（bilibili 等）行为零变化：
// 不判短签名、不动 URL、不触任何上传。
func TestResolveMarkdownImagesOtherDomainsUnchanged(t *testing.T) {
	const in = "![b](https://i0.hdslb.com/bfs/a.png) [链接](https://example.com/x.png)"
	seam := &slUploadSeam{wantURL: "https://cdn.example.com/should-not-be-used.png"}
	withShortLivedUploadSeam(t, seam)

	out := ResolveMarkdownImages(in, &slMockAPI{})
	if out != in {
		t.Fatalf("非短签名域不得改写: %q", out)
	}
	if len(seam.gotBase64) != 0 {
		t.Fatal("非短签名域不得触达上传链")
	}
}

// TestUploadImageBytesToPublicURLEmptyResult 既有上传流程回空外链（不报错也不给 URL）→
// 判定失败并降级，不得把空字符串写进 Markdown。
func TestUploadImageBytesToPublicURLEmptyResult(t *testing.T) {
	seam := &slUploadSeam{wantURL: ""}
	withShortLivedUploadSeam(t, seam)
	if url, ok := uploadImageBytesToPublicURL(&slMockAPI{}, slPNG()); ok || url != "" {
		t.Fatalf("空外链应降级: url=%q ok=%v", url, ok)
	}
	if len(seam.gotBase64) != 1 {
		t.Fatalf("空外链判定前应恰好调用一次上传流程，实际 %d", len(seam.gotBase64))
	}
}

// TestUploadImageBytesToPublicURLRejectsShortLivedResult 上传链仍回短签名域（例如新增 provider
// 未纳入直链解析）且降级路径内层解析也失败时必须拒绝替换：不把短链当稳定外链写进 Markdown。
func TestUploadImageBytesToPublicURLRejectsShortLivedResult(t *testing.T) {
	const shortURL = "https://t1.chatglm.cn/file/x.png?expired_at=1&sign=a"
	seam := &slUploadSeam{wantURL: shortURL}
	withShortLivedUploadSeam(t, seam)
	withShortLivedResolveSeam(t, func(string) (string, error) { return "", errors.New("直链解析失败") })
	if url, ok := uploadImageBytesToPublicURL(&slMockAPI{}, slPNG()); ok || url != "" {
		t.Fatalf("上传链回短签名外链且解析失败时必须拒绝替换: url=%q ok=%v", url, ok)
	}
	if len(seam.gotBase64) != 1 {
		t.Fatalf("应恰好调用一次上传流程，实际 %d", len(seam.gotBase64))
	}
}

// TestUploadImageBytesToPublicURLAdoptsResolvedDirect 上传链仍回短签名外链时，降级路径内
// 再解析一次 302：成功则采纳直链（不再判失败）。
func TestUploadImageBytesToPublicURLAdoptsResolvedDirect(t *testing.T) {
	const shortURL = "https://t1.chatglm.cn/file/x.png?expired_at=1&sign=a"
	const directURL = "https://pfiles.chatglm.cn/file/x.png?x-oss-expires=899&sign=oss"
	seam := &slUploadSeam{wantURL: shortURL}
	withShortLivedUploadSeam(t, seam)
	withShortLivedResolveSeam(t, func(string) (string, error) { return directURL, nil })

	if url, ok := uploadImageBytesToPublicURL(&slMockAPI{}, slPNG()); !ok || url != directURL {
		t.Fatalf("降级路径内层解析成功应采纳直链: url=%q ok=%v", url, ok)
	}
	if len(seam.gotBase64) != 1 {
		t.Fatalf("应恰好调用一次上传流程，实际 %d", len(seam.gotBase64))
	}
}

// TestSelfFetchToStableImageURLResolvedDirect ① 直链解析成功分支：直接返回直链，
// 不触发自取、不触达上传链（计数缝证明零网络）。
func TestSelfFetchToStableImageURLResolvedDirect(t *testing.T) {
	const shortURL = "https://t1.chatglm.cn/file/x.png?expired_at=1&sign=a"
	const directURL = "https://pfiles.chatglm.cn/file/x.png?x-oss-expires=899&sign=oss"
	var fetchCalls int
	oldFetch := selfFetchBytes
	selfFetchBytes = func(string) ([]byte, error) {
		fetchCalls++
		return nil, errors.New("解析成功分支不应自取")
	}
	t.Cleanup(func() { selfFetchBytes = oldFetch })
	withShortLivedResolveSeam(t, func(raw string) (string, error) {
		if raw != shortURL {
			t.Errorf("解析入参应为短签名外链: %q", raw)
		}
		return directURL, nil
	})
	seam := &slUploadSeam{wantURL: "https://cdn.example.com/should-not-be-used.png"}
	withShortLivedUploadSeam(t, seam)

	got, ok := selfFetchToStableImageURL(&slMockAPI{}, shortURL)
	if !ok || got != directURL {
		t.Fatalf("解析成功应直接采纳直链: url=%q ok=%v", got, ok)
	}
	if fetchCalls != 0 {
		t.Fatalf("解析成功后不得自取字节，实际 %d 次", fetchCalls)
	}
	if len(seam.gotBase64) != 0 {
		t.Fatalf("解析成功后不得触达上传链，实际 %d 次", len(seam.gotBase64))
	}
}

// TestSelfFetchImageURLSchemeRestore legacy foundItems 剥离 scheme 形态按消费点默认补全，
// 带 scheme 的值原样返回（avatar 透传形态）。
func TestSelfFetchImageURLSchemeRestore(t *testing.T) {
	cases := []struct {
		value, scheme, want string
	}{
		{"t1.chatglm.cn/file/a.png?sign=1", "https", "https://t1.chatglm.cn/file/a.png?sign=1"},
		{"t1.chatglm.cn/file/a.png?sign=1", "http", "http://t1.chatglm.cn/file/a.png?sign=1"},
		{"https://t1.chatglm.cn/file/a.png?sign=1", "http", "https://t1.chatglm.cn/file/a.png?sign=1"},
		{"http://t1.chatglm.cn/file/a.png", "https", "http://t1.chatglm.cn/file/a.png"},
	}
	for _, c := range cases {
		if got := selfFetchImageURL(c.value, c.scheme); got != c.want {
			t.Errorf("selfFetchImageURL(%q, %q) = %q, want %q", c.value, c.scheme, got, c.want)
		}
	}
}

// TestKeyboardShortLivedRewritten keyboard 按钮图片同型消费 URL：同样自取后走既有上传流程换外链。
func TestKeyboardShortLivedRewritten(t *testing.T) {
	imgURL := newShortLivedUpstream(t)
	withShortLivedSeams(t, slMatcher(imgURL))
	const stableURL = "https://cdn.example.com/kb-stable.png"
	seam := &slUploadSeam{wantURL: stableURL}
	withShortLivedUploadSeam(t, seam)

	kb := &keyboard.MessageKeyboard{Content: &keyboard.CustomKeyboard{
		Rows: []*keyboard.Row{{
			Buttons: []*keyboard.Button{{
				RenderData: &keyboard.RenderData{
					Label:        "![btn](" + imgURL + ")",
					VisitedLabel: "![btn](" + imgURL + ")",
				},
			}},
		}},
	}}
	ResolveKeyboardImages(kb, &slMockAPI{})
	got := kb.Content.Rows[0].Buttons[0].RenderData
	want := "![btn](" + stableURL + ")"
	if got.Label != want || got.VisitedLabel != want {
		t.Fatalf("keyboard 短签名外链应替换为自取转传后的外链: got label=%q visited=%q want %q", got.Label, got.VisitedLabel, want)
	}
	if len(seam.gotBase64) != 2 {
		t.Fatalf("label 与 visited_label 应各触达一次上传流程，实际 %d", len(seam.gotBase64))
	}
	for _, b64 := range seam.gotBase64 {
		if b64 != base64.StdEncoding.EncodeToString(slPNG()) {
			t.Fatalf("上传流程入参应为自取字节的 base64: got %d 字节", len(b64))
		}
	}
}

// TestShortLivedSelfFetchSSRFRejectsPrivateTarget Major-2：自取不得成为 SSRF 的旁路。
// 域名判定命中（真实污染形态：被污染的 *.chatglm.cn 指向内网/回环），
// SSRF 缝保持生产默认（isPrivateOrLoopback），入口即拒 → 不发起自取、不触字节上传链、降级原外链直传。
func TestShortLivedSelfFetchSSRFRejectsPrivateTarget(t *testing.T) {
	slLoadTemplateConfig(t)
	var upstreamHit int32
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		atomic.AddInt32(&upstreamHit, 1)
		w.Write(slPNG())
	}))
	defer srv.Close()
	imgURL := srv.URL + "/file/a.png?sign=abc//def"

	oldMatch, oldFetch, oldSSRF := isShortLivedImageURL, selfFetchBytes, selfFetchSSRFBlocked
	isShortLivedImageURL = slMatcher(imgURL)
	selfFetchBytes = func(raw string) ([]byte, error) {
		t.Errorf("私有/回环目标必须在入口就被拒，不得发起自取: %q", raw)
		return nil, errors.New("自取不得被调用")
	}
	// selfFetchSSRFBlocked 不覆写：用生产默认的 isPrivateOrLoopback（httptest 是 127.0.0.1 回环）
	t.Cleanup(func() {
		isShortLivedImageURL, selfFetchBytes, selfFetchSSRFBlocked = oldMatch, oldFetch, oldSSRF
	})

	mock := &slMockAPI{}
	stripped := strings.TrimPrefix(imgURL, "http://")
	got := generateGroupMessage("mid-ssrf", "eid-ssrf", map[string][]string{"url_image": {stripped}}, "", 1, mock, "group-openid")
	rich, ok := got.(*dto.RichMediaMessage)
	if !ok {
		t.Fatalf("SSRF 拒绝后应降级为原 URL 直传，实际 %T", got)
	}
	if rich.URL != imgURL {
		t.Fatalf("降级必须维持原 URL 原样: %q", rich.URL)
	}
	if len(mock.groupMsgs) != 0 {
		t.Fatal("SSRF 拒绝路径不得触达字节上传链")
	}
	// Markdown/keyboard 消费点同口径
	if ref, done := selfFetchToStableImageURL(mock, imgURL); done || ref != "" {
		t.Fatalf("Markdown 入口同样必须拒绝私有/回环目标: ref=%q ok=%v", ref, done)
	}
	if n := atomic.LoadInt32(&upstreamHit); n != 0 {
		t.Fatalf("入口拒绝后不得有任何网络访问，实际命中 %d 次", n)
	}
}

// TestShortLivedGroupImageRejectsNonImageBytes 富媒体自取上传前先嗅探格式，
// 非 png/jpeg/gif/webp 一律 warn+降级，不把任意字节塞进 QQ CDN。
func TestShortLivedGroupImageRejectsNonImageBytes(t *testing.T) {
	slLoadTemplateConfig(t)
	imgURL := "http://t1.chatglm.cn/file/a.png?sign=abc//def"
	oldMatch, oldFetch, oldSSRF := isShortLivedImageURL, selfFetchBytes, selfFetchSSRFBlocked
	isShortLivedImageURL = slMatcher(imgURL)
	selfFetchBytes = func(string) ([]byte, error) { return []byte("plain-text-not-an-image"), nil }
	selfFetchSSRFBlocked = func(string) bool { return false }
	t.Cleanup(func() {
		isShortLivedImageURL, selfFetchBytes, selfFetchSSRFBlocked = oldMatch, oldFetch, oldSSRF
	})

	mock := &slMockAPI{}
	stripped := strings.TrimPrefix(imgURL, "http://")
	got := generateGroupMessage("mid-fmt", "eid-fmt", map[string][]string{"url_image": {stripped}}, "", 1, mock, "group-openid")
	rich, ok := got.(*dto.RichMediaMessage)
	if !ok || rich.URL != imgURL {
		t.Fatalf("非法格式应降级原外链直传，实际 %T: %+v", got, got)
	}
	if len(mock.groupMsgs) != 0 {
		t.Fatal("非法格式不得触达字节上传链")
	}
}

// slCountSends 按载体类型拆分 apiv2 收到的消息：RichMediaMessage 是 CDN 上传（=一次远程自取），
// MessageToCreate 是真正发出的会话消息。
func slCountSends(msgs []dto.APIMessage) (uploads, sends int, last *dto.MessageToCreate) {
	for _, m := range msgs {
		switch v := m.(type) {
		case *dto.RichMediaMessage:
			uploads++
		case *dto.MessageToCreate:
			sends++
			last = v
		}
	}
	return uploads, sends, last
}

// TestSendFoundItemsGroupSelfFetchSendsSingleMessage Major-3：新链（arch_mode=new 走本桥）
// 图文混合 + 短签名外链，预投递必须接住自取返回的 MessageToCreate——
// 否则断言失败既不发文本也不删媒体键，媒体循环会二次远程自取 + 二次上传，一条消息变两条。
func TestSendFoundItemsGroupSelfFetchSendsSingleMessage(t *testing.T) {
	slLoadTemplateConfig(t)
	imgURL := newShortLivedUpstream(t)
	withShortLivedSeams(t, slMatcher(imgURL))

	mock := &slMockAPI{}
	stripped := strings.TrimPrefix(imgURL, "http://")
	found := map[string][]string{"url_image": {stripped}}
	if _, err := SendFoundItemsGroup(context.Background(), "group-openid", "图文混合文本", found, mock); err != nil {
		t.Fatalf("新链发送失败: %v", err)
	}

	uploads, sends, last := slCountSends(mock.groupMsgs)
	if uploads != 1 {
		t.Fatalf("同图只能有一次远程自取 + CDN 上传，实际 %d 次", uploads)
	}
	if sends != 1 {
		t.Fatalf("文本 + 图必须仍是一条消息，实际发出 %d 条", sends)
	}
	if last == nil || last.MsgType != 7 || last.Media == nil || last.Media.FileInfo != "group-file-info" {
		t.Fatalf("发出的应是 msg_type=7 带 file_info 的消息: %+v", last)
	}
	if last.Content != "图文混合文本" {
		t.Fatalf("文本必须并入同一条消息，实际 content=%q", last.Content)
	}
	if _, ok := found["url_image"]; ok {
		t.Fatal("预投递消费后必须删除媒体键（与 markdown 键同口径）")
	}
}

// TestSendFoundItemsPrivateSelfFetchSendsSingleMessage 私聊预投递的对称点，口径同群聊。
func TestSendFoundItemsPrivateSelfFetchSendsSingleMessage(t *testing.T) {
	slLoadTemplateConfig(t)
	imgURL := newShortLivedUpstream(t)
	withShortLivedSeams(t, slMatcher(imgURL))

	mock := &slMockAPI{}
	stripped := strings.TrimPrefix(imgURL, "http://")
	found := map[string][]string{"url_image": {stripped}}
	if _, err := SendFoundItemsPrivate(context.Background(), "user-openid", "图文混合文本", found, false, mock); err != nil {
		t.Fatalf("新链私聊发送失败: %v", err)
	}

	uploads, sends, last := slCountSends(mock.c2cMsgs)
	if uploads != 1 {
		t.Fatalf("同图只能有一次远程自取 + CDN 上传，实际 %d 次", uploads)
	}
	if sends != 1 {
		t.Fatalf("文本 + 图必须仍是一条消息，实际发出 %d 条", sends)
	}
	if last == nil || last.MsgType != 7 || last.Content != "图文混合文本" ||
		last.Media == nil || last.Media.FileInfo != "c2c-file-info" {
		t.Fatalf("发出的消息不符: %+v", last)
	}
	if _, ok := found["url_image"]; ok {
		t.Fatal("预投递消费后必须删除媒体键")
	}
}

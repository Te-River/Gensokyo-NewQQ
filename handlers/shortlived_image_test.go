package handlers

// 短签名图床外链（oss_type=7 ChatGLM）自取转传回归：
// 富媒体 302 两跳自取 → 本地字节上传（非 URL 直传）；Markdown 外链 → qqbot-media:// 素材引用；
// 自取/素材链失败 → 降级维持原外链直传；SSRF 私有/回环目标 → 入口拒绝并降级；
// 新链图文混合（SendFoundItems*）预投递 → 一次自取、一条消息。全部 httptest 离线，不触真实网络。
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

// slMockAPI 记录富媒体上传与素材链上游调用。
type slMockAPI struct {
	openapi.OpenAPI

	groupMsgs []dto.APIMessage
	c2cMsgs   []dto.APIMessage

	prepareReq   *dto.MediaImagePrepareRequest
	prepareResp  *dto.MediaImagePrepareResponse
	completeReq  *dto.MediaImageCompleteRequest
	completeResp *dto.MediaImageCompleteResponse
	completeErr  error
}

func (m *slMockAPI) PostGroupMessage(_ context.Context, _ string, msg dto.APIMessage) (*dto.GroupMessageResponse, error) {
	m.groupMsgs = append(m.groupMsgs, msg)
	return &dto.GroupMessageResponse{MediaResponse: &dto.MediaResponse{FileInfo: "group-file-info"}}, nil
}

func (m *slMockAPI) PostC2CMessage(_ context.Context, _ string, msg dto.APIMessage) (*dto.C2CMessageResponse, error) {
	m.c2cMsgs = append(m.c2cMsgs, msg)
	return &dto.C2CMessageResponse{MediaResponse: &dto.MediaResponse{FileInfo: "c2c-file-info"}}, nil
}

func (m *slMockAPI) PrepareMediaImage(_ context.Context, req *dto.MediaImagePrepareRequest) (*dto.MediaImagePrepareResponse, error) {
	m.prepareReq = req
	return m.prepareResp, nil
}

func (m *slMockAPI) CompleteMediaImage(_ context.Context, req *dto.MediaImageCompleteRequest) (*dto.MediaImageCompleteResponse, error) {
	m.completeReq = req
	return m.completeResp, m.completeErr
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
//（cqparse avatar 透传形态）同样命中自取转传。
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
//（url_pic_transfer=false 的既有直传语义，不比现状差、不阻断发送）。
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

// newSLPutServer 素材链 PUT 直传 mock：校验 Content-Type 且不携带 Authorization，回带引号 ETag。
func newSLPutServer(t *testing.T, wantContentType, wantMD5 string) *httptest.Server {
	t.Helper()
	return httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodPut {
			t.Errorf("预签名 URL 只接受 PUT，实际 %s", r.Method)
		}
		if got := r.Header.Get("Content-Type"); got != wantContentType {
			t.Errorf("Content-Type: got %q, want %q", got, wantContentType)
		}
		if got := r.Header.Get("Authorization"); got != "" {
			t.Errorf("PUT 预签名 URL 不得携带 Authorization，实际 %q", got)
		}
		w.Header().Set("ETag", `"`+wantMD5+`"`)
		w.WriteHeader(http.StatusOK)
	}))
}

// TestResolveMarkdownImagesShortLivedRewrittenToMediaReference Markdown 短签名外链：
// 自取两跳 → 素材链（prepare→PUT→complete）→ qqbot-media:// 引用替换原 URL。
func TestResolveMarkdownImagesShortLivedRewrittenToMediaReference(t *testing.T) {
	imgURL := newShortLivedUpstream(t)
	withShortLivedSeams(t, slMatcher(imgURL))

	content := slPNG()
	putSrv := newSLPutServer(t, "image/png", computeMD5(content))
	defer putSrv.Close()
	mock := &slMockAPI{
		prepareResp:  &dto.MediaImagePrepareResponse{UploadID: "up-sl", PresignedURL: putSrv.URL, ExpiresIn: 3600},
		completeResp: &dto.MediaImageCompleteResponse{ImageID: "img-sl", URL: "qqbot-media://image/v1/token-sl"},
	}

	in := "看看 ![梗图](" + imgURL + ") 完毕"
	out := ResolveMarkdownImages(in, mock)
	want := "看看 ![梗图](qqbot-media://image/v1/token-sl) 完毕"
	if out != want {
		t.Fatalf("短签名外链应替换为素材引用:\n got: %s\nwant: %s", out, want)
	}
	if mock.prepareReq == nil {
		t.Fatal("未调用 prepare")
	}
	if mock.prepareReq.ContentType != "image/png" ||
		!strings.HasPrefix(mock.prepareReq.FileName, "selffetch.png") &&
			!strings.HasPrefix(mock.prepareReq.FileName, "selffetch.") {
		t.Fatalf("prepare 参数不符: %+v", mock.prepareReq)
	}
	if mock.prepareReq.MD5 != computeMD5(content) || mock.prepareReq.SHA1 != computeMediaImageSHA1(content) {
		t.Fatalf("prepare 校验和不符: %+v", mock.prepareReq)
	}
	if mock.completeReq == nil || mock.completeReq.UploadID != "up-sl" || mock.completeReq.MD5 != computeMD5(content) {
		t.Fatalf("complete 参数不符: %+v", mock.completeReq)
	}
}

// TestResolveMarkdownImagesShortLivedUploadFailureKeepsOriginal 素材链 complete 失败 →
// 维持原外链（降级语义），但已按链路尝试过 prepare。
func TestResolveMarkdownImagesShortLivedUploadFailureKeepsOriginal(t *testing.T) {
	imgURL := newShortLivedUpstream(t)
	withShortLivedSeams(t, slMatcher(imgURL))

	putSrv := newSLPutServer(t, "image/png", computeMD5(slPNG()))
	defer putSrv.Close()
	mock := &slMockAPI{
		prepareResp: &dto.MediaImagePrepareResponse{UploadID: "up-sl", PresignedURL: putSrv.URL, ExpiresIn: 3600},
		completeErr: errors.New("complete 上游失败"),
	}

	in := "![a](" + imgURL + ")"
	out := ResolveMarkdownImages(in, mock)
	if out != in {
		t.Fatalf("素材链失败应维持原外链: got %q", out)
	}
	if mock.prepareReq == nil {
		t.Fatal("应尝试过素材链上传")
	}
}

// TestResolveMarkdownImagesOtherDomainsUnchanged 其他域（bilibili 等）行为零变化：
// 不判短签名、不动 URL、不触任何上传。
func TestResolveMarkdownImagesOtherDomainsUnchanged(t *testing.T) {
	const in = "![b](https://i0.hdslb.com/bfs/a.png) [链接](https://example.com/x.png)"
	mock := &slMockAPI{}
	out := ResolveMarkdownImages(in, mock)
	if out != in {
		t.Fatalf("非短签名域不得改写: %q", out)
	}
	if mock.prepareReq != nil || len(mock.groupMsgs) != 0 {
		t.Fatal("非短签名域不得触达任何上传链")
	}
}

// TestUploadImageBytesToMediaStoreNilAPI apiv2 为 nil → 直接降级，不 panic。
func TestUploadImageBytesToMediaStoreNilAPI(t *testing.T) {
	if ref, ok := uploadImageBytesToMediaStore(nil, slPNG()); ok || ref != "" {
		t.Fatalf("nil apiv2 应降级: ref=%q ok=%v", ref, ok)
	}
}

// TestUploadImageBytesToMediaStoreUnsupportedFormat 非素材链四格式（png/jpeg/gif/webp）→ 降级。
func TestUploadImageBytesToMediaStoreUnsupportedFormat(t *testing.T) {
	mock := &slMockAPI{}
	if ref, ok := uploadImageBytesToMediaStore(mock, []byte("plain-text-not-an-image")); ok || ref != "" {
		t.Fatalf("不受支持格式应降级: ref=%q ok=%v", ref, ok)
	}
	if mock.prepareReq != nil {
		t.Fatal("格式不受支持时不得触达 prepare")
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

// TestKeyboardShortLivedRewritten keyboard 按钮图片同型消费 URL：同样自取转素材引用。
func TestKeyboardShortLivedRewritten(t *testing.T) {
	imgURL := newShortLivedUpstream(t)
	withShortLivedSeams(t, slMatcher(imgURL))

	putSrv := newSLPutServer(t, "image/png", computeMD5(slPNG()))
	defer putSrv.Close()
	mock := &slMockAPI{
		prepareResp:  &dto.MediaImagePrepareResponse{UploadID: "up-kb", PresignedURL: putSrv.URL, ExpiresIn: 3600},
		completeResp: &dto.MediaImageCompleteResponse{ImageID: "img-kb", URL: "qqbot-media://image/v1/token-kb"},
	}

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
	ResolveKeyboardImages(kb, mock)
	got := kb.Content.Rows[0].Buttons[0].RenderData
	want := "![btn](qqbot-media://image/v1/token-kb)"
	if got.Label != want || got.VisitedLabel != want {
		t.Fatalf("keyboard 短签名外链应替换为素材引用: got label=%q visited=%q want %q", got.Label, got.VisitedLabel, want)
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
	if ref, done := selfFetchToMediaReference(mock, imgURL); done || ref != "" {
		t.Fatalf("Markdown 入口同样必须拒绝私有/回环目标: ref=%q ok=%v", ref, done)
	}
	if n := atomic.LoadInt32(&upstreamHit); n != 0 {
		t.Fatalf("入口拒绝后不得有任何网络访问，实际命中 %d 次", n)
	}
}

// TestShortLivedGroupImageRejectsNonImageBytes Minor：富媒体自取上传前先嗅探格式，
// 非 png/jpeg/gif/webp 一律 warn+降级（与素材链同口径），不把任意字节塞进 QQ CDN。
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

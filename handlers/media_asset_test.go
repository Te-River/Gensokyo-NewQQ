package handlers

// 素材管理链（media-images）回归：本地图片字节 → prepare → 预签名 PUT → complete →
// qqbot-media:// 引用；短签名 provider 跳过图床；素材链失败回退既有外链；
// 短签名外链自取路径同样产出素材引用。全部离线（httptest 模拟预签名 PUT），不触真实网络。
// 约定：handlers 套件用例禁用 t.Parallel()（注入缝与 config、echo 都是包级全局态）。

import (
	"bytes"
	"context"
	"crypto/sha1"
	"encoding/hex"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/errs"
	"github.com/tencent-connect/botgo/openapi"
)

// mediaStoreMock 仅覆写素材链两步，其余方法由嵌入的 nil 接口兜底。
type mediaStoreMock struct {
	openapi.OpenAPI

	prepareReq   *dto.MediaImagePrepareRequest
	prepareResp  *dto.MediaImagePrepareResponse
	prepareErr   error
	completeReq  *dto.MediaImageCompleteRequest
	completeResp *dto.MediaImageCompleteResponse
	completeErr  error
}

func (m *mediaStoreMock) PrepareMediaImage(_ context.Context, req *dto.MediaImagePrepareRequest) (*dto.MediaImagePrepareResponse, error) {
	m.prepareReq = req
	return m.prepareResp, m.prepareErr
}

func (m *mediaStoreMock) CompleteMediaImage(_ context.Context, req *dto.MediaImageCompleteRequest) (*dto.MediaImageCompleteResponse, error) {
	m.completeReq = req
	return m.completeResp, m.completeErr
}

// newMediaPutServer 预签名 URL 的 mock：校验 PUT / Content-Type / 不携带 Authorization，
// 回读请求体与准备上传的字节逐字节比对（不回读的话 md5/size 对不上也永远绿），并回带引号 ETag；
// etag 为空串时不回 ETag（覆盖"直传响应缺 ETag"的降级分支）。
func newMediaPutServer(t *testing.T, wantContentType string, wantBody []byte, etag string) *httptest.Server {
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
		body, err := io.ReadAll(r.Body)
		if err != nil {
			t.Errorf("读取 PUT 请求体失败: %v", err)
		}
		if !bytes.Equal(body, wantBody) {
			t.Errorf("PUT 请求体与准备上传的字节不一致: got %d 字节(%s) want %d 字节(%s)",
				len(body), computeMD5(body), len(wantBody), computeMD5(wantBody))
		}
		if etag != "" {
			w.Header().Set("ETag", `"`+etag+`"`)
		}
		w.WriteHeader(http.StatusOK)
	}))
}

// newMediaStoreOK 完整成功的素材链 mock：prepare 指向 httptest 预签名 URL，complete 回固定引用。
func newMediaStoreOK(t *testing.T, content []byte) (*mediaStoreMock, *httptest.Server) {
	t.Helper()
	putSrv := newMediaPutServer(t, "image/png", content, computeMD5(content))
	t.Cleanup(putSrv.Close)
	return newMediaStoreWithPutServer(putSrv.URL), putSrv
}

// newMediaStoreWithPutServer 素材链 mock：prepare 回给定预签名 URL，complete 回固定引用。
func newMediaStoreWithPutServer(presignedURL string) *mediaStoreMock {
	return &mediaStoreMock{
		prepareResp: &dto.MediaImagePrepareResponse{UploadID: "up-1", PresignedURL: presignedURL, ExpiresIn: 3600},
		completeResp: &dto.MediaImageCompleteResponse{
			ImageID: "img-1",
			URL:     "qqbot-media://image/v1/token-1",
			TTL:     7776000,
		},
	}
}

// withShortLivedOSSProvider 覆写「当前 oss_type 是否为短签名图床」注入缝。
func withShortLivedOSSProvider(t *testing.T, shortLived bool) {
	t.Helper()
	old := ossTypeShortLivedImage
	ossTypeShortLivedImage = func() bool { return shortLived }
	t.Cleanup(func() { ossTypeShortLivedImage = old })
}

// TestResolveLocalImageMediaShortLivedProviderSkipsCDN 本地图片 + 短签名图床（ChatGLM）：
// 直接走素材链产出 qqbot-media:// 引用，且完全不调用图床 provider；
// prepare 入参的文件名/大小/content_type/md5/sha1 与 complete 的 upload_id/md5(ETag 去引号) 逐项校验。
func TestResolveLocalImageMediaShortLivedProviderSkipsCDN(t *testing.T) {
	content := slPNG()
	mock, _ := newMediaStoreOK(t, content)
	withShortLivedOSSProvider(t, true)
	seam := &slUploadSeam{wantURL: "https://cdn.example.com/should-not-be-used.png"}
	withShortLivedUploadSeam(t, seam)

	ref, ok := resolveLocalImageMedia(mock, content)
	if !ok || ref != "qqbot-media://image/v1/token-1" {
		t.Fatalf("本地图片应产出素材引用: ref=%q ok=%v", ref, ok)
	}
	if len(seam.gotBase64) != 0 {
		t.Fatalf("短签名图床不得再调用图床 provider，实际 %d 次", len(seam.gotBase64))
	}

	req := mock.prepareReq
	if req == nil {
		t.Fatal("prepare 未被调用")
	}
	if req.FileName != "markdown.png" || req.ContentType != "image/png" {
		t.Fatalf("prepare 文件名/类型不符: %+v", req)
	}
	if req.FileSize != uint64(len(content)) {
		t.Fatalf("prepare file_size: got %d want %d", req.FileSize, len(content))
	}
	if req.MD5 != computeMD5(content) {
		t.Fatalf("prepare md5: got %q", req.MD5)
	}
	sum := sha1.Sum(content)
	if req.SHA1 != hex.EncodeToString(sum[:]) {
		t.Fatalf("prepare sha1: got %q", req.SHA1)
	}
	if mock.completeReq == nil || mock.completeReq.UploadID != "up-1" || mock.completeReq.MD5 != computeMD5(content) {
		t.Fatalf("complete 入参不符: %+v", mock.completeReq)
	}
}

// TestResolveLocalImageMediaFallsBackToCDN 素材链失败 → 退回既有外链上传，发送不被阻断。
func TestResolveLocalImageMediaFallsBackToCDN(t *testing.T) {
	withShortLivedOSSProvider(t, true)
	seam := &slUploadSeam{wantURL: "https://cdn.example.com/fallback.png"}
	withShortLivedUploadSeam(t, seam)
	mock := &mediaStoreMock{prepareErr: errors.New("code:40035006 服务端内部错误")}

	ref, ok := resolveLocalImageMedia(mock, slPNG())
	if !ok || ref != "https://cdn.example.com/fallback.png" {
		t.Fatalf("素材链失败应退回外链: ref=%q ok=%v", ref, ok)
	}
	if len(seam.gotBase64) != 1 {
		t.Fatalf("应恰好回退一次外链上传，实际 %d", len(seam.gotBase64))
	}
}

// TestResolveLocalImageMediaUpgradesShortLivedCDNResult 非短签名图床但外链仍是短签名形态：
// 用同一份字节换素材引用，不再把短链写进 markdown。
func TestResolveLocalImageMediaUpgradesShortLivedCDNResult(t *testing.T) {
	const shortURL = "https://t1.chatglm.cn/file/x.png?expired_at=1&sign=a"
	content := slPNG()
	mock, _ := newMediaStoreOK(t, content)
	withShortLivedOSSProvider(t, false)
	seam := &slUploadSeam{wantURL: shortURL}
	withShortLivedUploadSeam(t, seam)

	ref, ok := resolveLocalImageMedia(mock, content)
	if !ok || ref != "qqbot-media://image/v1/token-1" {
		t.Fatalf("短签名外链应升级为素材引用: ref=%q ok=%v", ref, ok)
	}
	if len(seam.gotBase64) != 1 {
		t.Fatalf("应恰好一次外链上传后升级，实际 %d", len(seam.gotBase64))
	}
}

// TestResolveMarkdownImagesLocalImageUsesMediaStore Markdown 本地图片端到端：
// 文件路径进 markdown，出来的必须是 qqbot-media:// 引用，且图床 provider 零调用。
func TestResolveMarkdownImagesLocalImageUsesMediaStore(t *testing.T) {
	content := slPNG()
	mock, _ := newMediaStoreOK(t, content)
	withShortLivedOSSProvider(t, true)
	seam := &slUploadSeam{wantURL: "https://cdn.example.com/should-not-be-used.png"}
	withShortLivedUploadSeam(t, seam)

	path := filepath.Join(t.TempDir(), "local.png")
	if err := os.WriteFile(path, content, 0o600); err != nil {
		t.Fatalf("写入临时图片失败: %v", err)
	}

	out := ResolveMarkdownImages("看图 ![本地]("+filepath.ToSlash(path)+") 完毕", mock)
	want := "看图 ![本地](qqbot-media://image/v1/token-1) 完毕"
	if out != want {
		t.Fatalf("Markdown 本地图片应改写成素材引用:\n got: %s\nwant: %s", out, want)
	}
	if len(seam.gotBase64) != 0 {
		t.Fatalf("短签名图床不得调用图床 provider，实际 %d 次", len(seam.gotBase64))
	}
}

// TestSelfFetchToStableImageURLUsesMediaStore 短签名外链自取路径（Markdown/keyboard）：
// 自取字节优先走素材链，产出 qqbot-media:// 引用，不再触达外链上传链。
func TestSelfFetchToStableImageURLUsesMediaStore(t *testing.T) {
	const shortURL = "https://t1.chatglm.cn/file/x.png?expired_at=1&sign=a"
	content := slPNG()
	mock, _ := newMediaStoreOK(t, content)
	withShortLivedStaticBytes(t, shortURL, content)
	seam := &slUploadSeam{wantURL: "https://cdn.example.com/should-not-be-used.png"}
	withShortLivedUploadSeam(t, seam)

	ref, ok := selfFetchToStableImageURL(mock, shortURL)
	if !ok || ref != "qqbot-media://image/v1/token-1" {
		t.Fatalf("自取路径应产出素材引用: ref=%q ok=%v", ref, ok)
	}
	if len(seam.gotBase64) != 0 {
		t.Fatalf("素材链成功时不得触达外链上传链，实际 %d 次", len(seam.gotBase64))
	}
}

// TestResolveMarkdownImagesKeepsMediaReference qqbot-media:// 引用原样透传：
// 既不当本地路径去读文件，也不被二次解析改写。
func TestResolveMarkdownImagesKeepsMediaReference(t *testing.T) {
	const ref = "qqbot-media://image/v1/token-abc"
	withShortLivedOSSProvider(t, true)
	seam := &slUploadSeam{wantURL: "https://cdn.example.com/should-not-be-used.png"}
	withShortLivedUploadSeam(t, seam)

	in := "![图 #200px #200px](" + ref + ")"
	if out := ResolveMarkdownImages(in, &mediaStoreMock{}); out != in {
		t.Fatalf("素材引用必须原样透传: got %q", out)
	}
	if len(seam.gotBase64) != 0 {
		t.Fatal("素材引用不得触达上传链")
	}
}

// TestMediaImageUploadFlowMapsOfficialError 官方 400350xx 错误码还原为中文说明并保留原码。
func TestMediaImageUploadFlowMapsOfficialError(t *testing.T) {
	withShortLivedOSSProvider(t, true)
	seam := &slUploadSeam{wantURL: "https://cdn.example.com/x.png"}
	withShortLivedUploadSeam(t, seam)
	mock := &mediaStoreMock{prepareErr: errs.New(40035006, "internal error")}

	_, err := mediaImageUploadFlow(context.Background(), mock, slPNG(), "markdown.png", "image/png", "test")
	if err == nil {
		t.Fatal("prepare 失败必须返回错误")
	}
	if !strings.Contains(err.Error(), "40035006") || !strings.Contains(err.Error(), "服务端内部错误") {
		t.Fatalf("错误码应还原为中文说明: %v", err)
	}
}

// TestMediaImageUploadFlowRejectsNonImageBytes 非 png/jpeg/gif/webp 字节不进素材链（防被污染的外链）。
func TestMediaImageUploadFlowRejectsNonImageBytes(t *testing.T) {
	withShortLivedOSSProvider(t, true)
	seam := &slUploadSeam{wantURL: "https://cdn.example.com/fallback.png"}
	withShortLivedUploadSeam(t, seam)
	mock := &mediaStoreMock{}

	if _, ok := uploadImageBytesToMediaStore(mock, []byte("plain-text"), "test"); ok {
		t.Fatal("非图片字节不应进素材链")
	}
	if mock.prepareReq != nil {
		t.Fatal("格式嗅探失败时不得调用 prepare")
	}
}

// TestMediaImageUploadFlowMissingETagUsesLocalMD5 预签名 PUT 未回 ETag（COS 可配置不返回）：
// complete 必须回退本地计算的 md5，不能带空串。
func TestMediaImageUploadFlowMissingETagUsesLocalMD5(t *testing.T) {
	content := slPNG()
	putSrv := newMediaPutServer(t, "image/png", content, "")
	t.Cleanup(putSrv.Close)
	mock := newMediaStoreWithPutServer(putSrv.URL)

	ref, err := mediaImageUploadFlow(context.Background(), mock, content, "markdown.png", "image/png", "test")
	if err != nil {
		t.Fatalf("缺 ETag 不应失败: %v", err)
	}
	if ref != "qqbot-media://image/v1/token-1" {
		t.Fatalf("应回素材引用: %q", ref)
	}
	if mock.completeReq == nil || mock.completeReq.MD5 != computeMD5(content) {
		t.Fatalf("缺 ETag 应回退本地 md5: %+v", mock.completeReq)
	}
}

// TestMediaImageUploadFlowETagOverridesLocalMD5 ETag 与本地 md5 不一致（COS 侧内容被改写等）：
// complete 以 ETag 为准，而不是本地 md5。
func TestMediaImageUploadFlowETagOverridesLocalMD5(t *testing.T) {
	content := slPNG()
	remoteMD5 := computeMD5([]byte("cos-side-content-changed"))
	if remoteMD5 == computeMD5(content) {
		t.Fatal("用例前提不成立：远端 md5 与本地相同")
	}
	putSrv := newMediaPutServer(t, "image/png", content, remoteMD5)
	t.Cleanup(putSrv.Close)
	mock := newMediaStoreWithPutServer(putSrv.URL)

	if _, err := mediaImageUploadFlow(context.Background(), mock, content, "markdown.png", "image/png", "test"); err != nil {
		t.Fatalf("ETag 不一致不应失败: %v", err)
	}
	if mock.completeReq == nil || mock.completeReq.MD5 != remoteMD5 {
		t.Fatalf("complete 应以 ETag 为准: got %+v want md5=%s", mock.completeReq, remoteMD5)
	}
}

// TestMediaImageUploadFlowPutNon2xx COS 回非 2xx：必须报错并带上状态码，绝不进 complete。
func TestMediaImageUploadFlowPutNon2xx(t *testing.T) {
	putSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		http.Error(w, "boom", http.StatusInternalServerError)
	}))
	t.Cleanup(putSrv.Close)
	mock := newMediaStoreWithPutServer(putSrv.URL)

	_, err := mediaImageUploadFlow(context.Background(), mock, slPNG(), "markdown.png", "image/png", "test")
	if err == nil {
		t.Fatal("PUT 非 2xx 必须返回错误")
	}
	if !strings.Contains(err.Error(), "500") || !strings.Contains(err.Error(), "boom") {
		t.Fatalf("错误应带状态码与响应体: %v", err)
	}
	if mock.completeReq != nil {
		t.Fatal("直传失败不得调用 complete")
	}
}

// TestMediaImageUploadFlowSkipsOversizedContent 超过 10MB 上限：本地判定即跳过，不触 prepare。
func TestMediaImageUploadFlowSkipsOversizedContent(t *testing.T) {
	oversized := append(slPNG(), make([]byte, maxMediaImageBytes)...)
	if int64(len(oversized)) <= maxMediaImageBytes {
		t.Fatalf("用例前提不成立：%d 字节未超上限", len(oversized))
	}
	mock := &mediaStoreMock{}

	if _, ok := uploadImageBytesToMediaStore(mock, oversized, "test"); ok {
		t.Fatal("超过 10MB 不应进素材链")
	}
	if mock.prepareReq != nil {
		t.Fatal("体积超限时不得调用 prepare")
	}
}

// TestSelfFetchToStableImageURLSelfFetchFailureFallsBackToDirect 自取失败不再直接放弃：
// 先退直链解析（只读响应头，成本低），成功即采纳直链、不触达既有外链上传链。
func TestSelfFetchToStableImageURLSelfFetchFailureFallsBackToDirect(t *testing.T) {
	const shortURL = "https://t1.chatglm.cn/file/x.png?expired_at=1&sign=a"
	const directURL = "https://pfiles.chatglm.cn/file/x.png?x-oss-expires=899&sign=oss"
	oldMatch, oldFetch, oldSSRF := isShortLivedImageURL, selfFetchBytes, selfFetchSSRFBlocked
	isShortLivedImageURL = slMatcher(shortURL)
	selfFetchBytes = func(string) ([]byte, error) { return nil, errors.New("自取失败") }
	selfFetchSSRFBlocked = func(string) bool { return false }
	t.Cleanup(func() {
		isShortLivedImageURL, selfFetchBytes, selfFetchSSRFBlocked = oldMatch, oldFetch, oldSSRF
	})
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
		t.Fatalf("自取失败应退回直链: url=%q ok=%v", got, ok)
	}
	if len(seam.gotBase64) != 0 {
		t.Fatalf("直链兜底成功不得触达上传链，实际 %d 次", len(seam.gotBase64))
	}
}

package handlers

// 素材管理链（media-images）回归：本地图片字节 → prepare → 预签名 PUT → complete →
// qqbot-media:// 引用；oss_type=7 直接走素材链且图床 provider 零调用；素材链失败或
// 40035007 配额超限回退既有外链；短签名外链自取路径同样产出素材引用。
// 全部离线（httptest 模拟预签名 PUT），不触真实网络。
// 约定：handlers 套件用例禁用 t.Parallel()（注入缝与 config、echo 都是包级全局态）。

import (
	"bytes"
	"context"
	"crypto/sha1"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/errs"
	"github.com/tencent-connect/botgo/openapi"
)

// mediaStoreMock 仅覆写素材链相关方法，其余方法由嵌入的 nil 接口兜底。
type mediaStoreMock struct {
	openapi.OpenAPI

	prepareReq   *dto.MediaImagePrepareRequest
	prepareResp  *dto.MediaImagePrepareResponse
	prepareErr   error
	prepareErrs  []error // 按调用顺序消费，用于「配额超限 → 清理 → 重试成功」用例
	prepareCalls int

	completeReq  *dto.MediaImageCompleteRequest
	completeResp *dto.MediaImageCompleteResponse
	completeErr  error

	listResp   *dto.MediaImageList
	listCalls  int
	deletedIDs []string
}

func (m *mediaStoreMock) PrepareMediaImage(_ context.Context, req *dto.MediaImagePrepareRequest) (*dto.MediaImagePrepareResponse, error) {
	m.prepareReq = req
	m.prepareCalls++
	if len(m.prepareErrs) > 0 {
		err := m.prepareErrs[0]
		m.prepareErrs = m.prepareErrs[1:]
		if err != nil {
			return nil, err
		}
	}
	return m.prepareResp, m.prepareErr
}

func (m *mediaStoreMock) ListMediaImages(_ context.Context, _ string) (*dto.MediaImageList, error) {
	m.listCalls++
	return m.listResp, nil
}

func (m *mediaStoreMock) DeleteMediaImage(_ context.Context, imageID string) error {
	m.deletedIDs = append(m.deletedIDs, imageID)
	return nil
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

// TestResolveLocalImageMediaQuotaExceededFallsBackToCDN oss_type=7 + 40035007（100 张上限）：
// 素材链不重试，直接走既有外链兜底，发送不被阻断。
func TestResolveLocalImageMediaQuotaExceededFallsBackToCDN(t *testing.T) {
	withShortLivedOSSProvider(t, true)
	seam := &slUploadSeam{wantURL: "https://cdn.example.com/quota-fallback.png"}
	withShortLivedUploadSeam(t, seam)
	mock := &mediaStoreMock{prepareErr: errs.New(40035007, "image count limit exceeded")}

	ref, ok := resolveLocalImageMedia(mock, slPNG())
	if !ok || ref != "https://cdn.example.com/quota-fallback.png" {
		t.Fatalf("配额超限应退回外链: ref=%q ok=%v", ref, ok)
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

// TestMediaImageUploadFlowQuotaExceededReturnsSentinel 40035007（单机器人 100 张上限）：
// 必须还原成可判定的配额哨兵，上传入口据此降级而不是重试。
func TestMediaImageUploadFlowQuotaExceededReturnsSentinel(t *testing.T) {
	mock := &mediaStoreMock{prepareErr: errs.New(40035007, "image count limit exceeded")}

	_, err := mediaImageUploadFlow(context.Background(), mock, slPNG(), "markdown.png", "image/png", "test")
	if err == nil {
		t.Fatal("配额超限必须返回错误")
	}
	if !errors.Is(err, errMediaImageQuotaExceeded) {
		t.Fatalf("40035007 应返回配额哨兵: %v", err)
	}
	if !strings.Contains(err.Error(), "40035007") || !strings.Contains(err.Error(), "100 张") {
		t.Fatalf("配额错误应带官方码与中文说明: %v", err)
	}
}

// TestMediaImageUploadFlowQuotaPurgeDeletesOldestAndRetries 40035007：
// 拉列表 → 只删 24 小时前的旧素材（一次最多 20 张，最旧优先）→ 重试一次成功。
func TestMediaImageUploadFlowQuotaPurgeDeletesOldestAndRetries(t *testing.T) {
	content := slPNG()
	mock, _ := newMediaStoreOK(t, content)
	mock.prepareErrs = []error{errs.New(40035007, "image count limit exceeded")}

	now := time.Now()
	// 25 张旧素材（48~72 小时前创建，列表顺序故意打乱）＋ 1 张 1 小时前的新素材
	images := []*dto.MediaImage{{ImageID: "recent", CreatedAt: now.Add(-time.Hour).Unix()}}
	for i := 24; i >= 0; i-- {
		images = append(images, &dto.MediaImage{
			ImageID:   fmt.Sprintf("old-%02d", i),
			CreatedAt: now.Add(-time.Duration(48+i) * time.Hour).Unix(),
		})
	}
	mock.listResp = &dto.MediaImageList{Images: images}

	ref, err := mediaImageUploadFlow(context.Background(), mock, content, "markdown.png", "image/png", "test")
	if err != nil {
		t.Fatalf("清理后重试应成功: %v", err)
	}
	if ref != "qqbot-media://image/v1/token-1" {
		t.Fatalf("应回素材引用: %q", ref)
	}
	if mock.prepareCalls != 2 {
		t.Fatalf("配额超限应重试一次 prepare，实际调用 %d 次", mock.prepareCalls)
	}
	if mock.listCalls != 1 {
		t.Fatalf("应拉取一次素材列表，实际 %d 次", mock.listCalls)
	}
	if len(mock.deletedIDs) != mediaQuotaPurgeCount {
		t.Fatalf("应清理 %d 张旧素材，实际 %d: %v", mediaQuotaPurgeCount, len(mock.deletedIDs), mock.deletedIDs)
	}
	for _, id := range mock.deletedIDs {
		if id == "recent" {
			t.Fatal("最近 24 小时内创建的素材不得删除")
		}
	}
	if mock.deletedIDs[0] != "old-24" {
		t.Fatalf("应从最旧开始删，首个删除项 %q", mock.deletedIDs[0])
	}
}

// TestResolveLocalImageMediaQuotaPurgeProtectsRecentFallsBack 40035007 且列表里只有
// 24 小时内创建的素材：一张都不删，直接降级既有外链，发送不被阻断。
func TestResolveLocalImageMediaQuotaPurgeProtectsRecentFallsBack(t *testing.T) {
	withShortLivedOSSProvider(t, true)
	seam := &slUploadSeam{wantURL: "https://cdn.example.com/quota-protected.png"}
	withShortLivedUploadSeam(t, seam)
	mock := &mediaStoreMock{
		prepareErr: errs.New(40035007, "image count limit exceeded"),
		listResp: &dto.MediaImageList{Images: []*dto.MediaImage{
			{ImageID: "recent-1", CreatedAt: time.Now().Add(-time.Hour).Unix()},
		}},
	}

	ref, ok := resolveLocalImageMedia(mock, slPNG())
	if !ok || ref != "https://cdn.example.com/quota-protected.png" {
		t.Fatalf("无可清理旧素材应降级外链: ref=%q ok=%v", ref, ok)
	}
	if len(mock.deletedIDs) != 0 {
		t.Fatalf("24 小时内的素材不得删除: %v", mock.deletedIDs)
	}
	if mock.prepareCalls != 1 {
		t.Fatalf("没有删到素材不应重试 prepare，实际 %d 次", mock.prepareCalls)
	}
	if len(seam.gotBase64) != 1 {
		t.Fatalf("应恰好回退一次外链上传，实际 %d", len(seam.gotBase64))
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

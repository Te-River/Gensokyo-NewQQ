package handlers

// 素材管理（media-images）回归保护：prepare 参数与校验、10MB 边界、
// 格式/扩展名一致性、ETag 去引号、官方错误码映射、qqbot-media 透传。

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/errs"
	"github.com/tencent-connect/botgo/openapi"
)

// miMockAPI 仅覆写素材管理四方法的 mock，其余方法由嵌入的 nil 接口兜底
type miMockAPI struct {
	openapi.OpenAPI

	prepareReq   *dto.MediaImagePrepareRequest
	prepareResp  *dto.MediaImagePrepareResponse
	prepareErr   error
	completeReq  *dto.MediaImageCompleteRequest
	completeResp *dto.MediaImageCompleteResponse
	completeErr  error
	listResp     *dto.MediaImageList
	listErr      error
	deleteErr    error
	deletedIDs   []string
}

func (m *miMockAPI) PrepareMediaImage(_ context.Context, req *dto.MediaImagePrepareRequest) (*dto.MediaImagePrepareResponse, error) {
	m.prepareReq = req
	return m.prepareResp, m.prepareErr
}

func (m *miMockAPI) CompleteMediaImage(_ context.Context, req *dto.MediaImageCompleteRequest) (*dto.MediaImageCompleteResponse, error) {
	m.completeReq = req
	return m.completeResp, m.completeErr
}

func (m *miMockAPI) ListMediaImages(_ context.Context, _ string) (*dto.MediaImageList, error) {
	return m.listResp, m.listErr
}

func (m *miMockAPI) DeleteMediaImage(_ context.Context, imageID string) error {
	m.deletedIDs = append(m.deletedIDs, imageID)
	return m.deleteErr
}

// miClient 只记录最后一次回包
type miClient struct{ last map[string]interface{} }

func (c *miClient) SendMessage(message map[string]interface{}) error {
	c.last = message
	return nil
}

// miResponse OneBot 回包解析
type miResponse struct {
	Data    json.RawMessage `json:"data"`
	Message string          `json:"message"`
	RetCode int             `json:"retcode"`
	Status  string          `json:"status"`
}

func parseMIMessage(t *testing.T, raw string) miResponse {
	t.Helper()
	var resp miResponse
	if err := json.Unmarshal([]byte(raw), &resp); err != nil {
		t.Fatalf("回包解析失败: %v (%s)", err, raw)
	}
	return resp
}

// newMIPutServer 预签名 URL 的 mock：校验方法、Content-Type 与不携带 Authorization，回带引号的 ETag
func newMIPutServer(t *testing.T, wantContentType, wantMD5 string) *httptest.Server {
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

// TestMediaImage_UploadPrepareParams (1)(4) prepare 请求体字段名与取值、ETag 去引号后传给 complete
func TestMediaImage_UploadPrepareParams(t *testing.T) {
	content := []byte("\x89PNG\r\n\x1a\nfake-image-body")
	md5Hex := computeMD5(content)
	putSrv := newMIPutServer(t, "image/png", md5Hex)
	defer putSrv.Close()

	mock := &miMockAPI{
		prepareResp: &dto.MediaImagePrepareResponse{UploadID: "up-1", PresignedURL: putSrv.URL, ExpiresIn: 3600},
		completeResp: &dto.MediaImageCompleteResponse{
			ImageID:    "img-1",
			URL:        "qqbot-media://image/v1/token-1",
			TTL:        7776000,
			PreviewURL: "https://cos.example/preview",
		},
	}

	client := &miClient{}
	out, err := UploadMediaImage(client, nil, mock, callapi.ActionMessage{
		Action: "upload_media_image",
		Params: callapi.ParamsContent{
			File:     "base64://" + base64.StdEncoding.EncodeToString(content),
			FileName: "kline.png",
		},
	})
	if err != nil {
		t.Fatalf("handler 错误: %v", err)
	}

	if mock.prepareReq == nil {
		t.Fatal("prepare 未被调用")
	}
	if mock.prepareReq.FileName != "kline.png" || mock.prepareReq.ContentType != "image/png" {
		t.Fatalf("prepare 文件名/类型: %+v", mock.prepareReq)
	}
	if mock.prepareReq.FileSize != uint64(len(content)) {
		t.Fatalf("prepare file_size: got %d, want %d", mock.prepareReq.FileSize, len(content))
	}
	if mock.prepareReq.MD5 != md5Hex {
		t.Fatalf("prepare md5: got %s, want %s", mock.prepareReq.MD5, md5Hex)
	}
	if len(mock.prepareReq.SHA1) != 40 {
		t.Fatalf("prepare sha1 长度应为 40: %s", mock.prepareReq.SHA1)
	}
	if mock.prepareReq.SHA1 != computeMediaImageSHA1(content) {
		t.Fatalf("prepare sha1 与本地计算不一致")
	}

	// (4) ETag 带引号 → complete 收到去引号值
	if mock.completeReq == nil || mock.completeReq.MD5 != md5Hex {
		t.Fatalf("complete md5 未去引号: %+v", mock.completeReq)
	}
	if mock.completeReq.UploadID != "up-1" {
		t.Fatalf("complete upload_id: %s", mock.completeReq.UploadID)
	}

	resp := parseMIMessage(t, out)
	if resp.RetCode != 0 || resp.Status != "ok" {
		t.Fatalf("回包状态: %+v", resp)
	}
	var data MediaImageUploadResponse
	if err := json.Unmarshal(resp.Data, &data); err != nil {
		t.Fatalf("data 解析失败: %v", err)
	}
	if data.ImageID != "img-1" || data.URL != "qqbot-media://image/v1/token-1" ||
		data.TTL != 7776000 || data.PreviewURL != "https://cos.example/preview" ||
		data.FileName != "kline.png" || data.ExpiresIn != 3600 {
		t.Fatalf("data 字段未原样回传: %+v", data)
	}
	if client.last == nil {
		t.Fatal("回包未通过 client 发送")
	}
}

// TestMediaImage_SizeBoundary (2) 10,000,000 通过、10,000,001 拒且不发网络
func TestMediaImage_SizeBoundary(t *testing.T) {
	if err := checkMediaImageSize(10_000_000); err != nil {
		t.Fatalf("10MB 应通过: %v", err)
	}
	if err := checkMediaImageSize(10_000_001); err == nil {
		t.Fatal("超过 10MB 应拒绝")
	}

	// 稀疏文件：stat 就超过上限，不应读取内容也不应发起 prepare
	path := filepath.Join(t.TempDir(), "big.png")
	f, err := os.Create(path)
	if err != nil {
		t.Fatalf("创建临时文件失败: %v", err)
	}
	if err := f.Truncate(10_000_001); err != nil {
		t.Fatalf("truncate 失败: %v", err)
	}
	f.Close()

	mock := &miMockAPI{}
	out, err := UploadMediaImage(&miClient{}, nil, mock, callapi.ActionMessage{
		Action: "upload_media_image",
		Params: callapi.ParamsContent{File: path},
	})
	if err != nil {
		t.Fatalf("handler 错误: %v", err)
	}
	resp := parseMIMessage(t, out)
	if resp.RetCode != 1400 {
		t.Fatalf("超限应回 1400: %+v", resp)
	}
	if mock.prepareReq != nil {
		t.Fatal("超限不得发起 prepare 网络请求")
	}
}

// TestMediaImage_FormatValidation (3) content_type 非四种、扩展名与 content_type 不一致
func TestMediaImage_FormatValidation(t *testing.T) {
	cases := []struct {
		name        string
		fileName    string
		contentType string
		wantErr     string
	}{
		{"bmp 类型拒绝", "a.png", "image/bmp", "不支持"},
		{"扩展名与类型不一致", "a.png", "image/jpeg", "不一致"},
		{"未知扩展名且未给类型", "a.bmp", "", "无法从文件名"},
		{"扩展名不在白名单", "a.bmp", "image/png", "不支持"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			mock := &miMockAPI{}
			out, err := UploadMediaImage(&miClient{}, nil, mock, callapi.ActionMessage{
				Action: "upload_media_image",
				Params: callapi.ParamsContent{
					File:        "base64://" + base64.StdEncoding.EncodeToString([]byte("x")),
					FileName:    tc.fileName,
					ContentType: tc.contentType,
				},
			})
			if err != nil {
				t.Fatalf("handler 错误: %v", err)
			}
			resp := parseMIMessage(t, out)
			if resp.RetCode != 1400 {
				t.Fatalf("应回 1400: %+v", resp)
			}
			if !strings.Contains(resp.Message, tc.wantErr) {
				t.Fatalf("message 应含 %q: %s", tc.wantErr, resp.Message)
			}
			if mock.prepareReq != nil {
				t.Fatal("校验失败不得发起 prepare")
			}
		})
	}

	// jpg 与 jpeg 视作同一族
	if name, ct, err := normalizeMediaImageMeta("a.jpeg", "image/jpeg"); err != nil || ct != "image/jpeg" || name != "a.jpeg" {
		t.Fatalf("jpeg 应同族通过: %s %s %v", name, ct, err)
	}
	// 未给 content_type 时按扩展名推导
	if _, ct, err := normalizeMediaImageMeta("a.webp", ""); err != nil || ct != "image/webp" {
		t.Fatalf("应按扩展名推导 content_type: %s %v", ct, err)
	}
}

// TestMediaImage_ErrorCodeMapping (5)(7) 官方 400350xx 映射为中文可读信息并保留原码
func TestMediaImage_ErrorCodeMapping(t *testing.T) {
	codes := []int{40035001, 40035002, 40035003, 40035007, 40035008, 40035009, 40035010, 40035006}
	for _, code := range codes {
		err := errs.New(http.StatusBadRequest, fmt.Sprintf(`{"code":%d,"message":"upstream"}`, code), "trace")
		got := describeMediaImageError(err)
		if !strings.Contains(got, strconv.Itoa(code)) {
			t.Fatalf("message 应保留官方码 %d: %s", code, got)
		}
		if !strings.Contains(got, "素材管理失败") || !strings.Contains(got, "：") {
			t.Fatalf("应带中文说明: %s", got)
		}
	}

	// complete 回 40035003 → retcode 1401 且 message 含 40035003
	content := []byte("payload")
	putSrv := newMIPutServer(t, "image/png", computeMD5(content))
	defer putSrv.Close()
	mock := &miMockAPI{
		prepareResp: &dto.MediaImagePrepareResponse{UploadID: "up-2", PresignedURL: putSrv.URL},
		completeErr: errs.New(http.StatusBadRequest, `{"code":40035003,"message":"upload expired"}`, ""),
	}
	out, _ := UploadMediaImage(&miClient{}, nil, mock, callapi.ActionMessage{
		Action: "upload_media_image",
		Params: callapi.ParamsContent{
			File:     "base64://" + base64.StdEncoding.EncodeToString(content),
			FileName: "a.png",
		},
	})
	resp := parseMIMessage(t, out)
	if resp.RetCode != 1401 || !strings.Contains(resp.Message, "40035003") {
		t.Fatalf("complete 40035003 应转 1401 且含原码: %+v", resp)
	}

	// delete 回 40035008 → 1401
	delMock := &miMockAPI{deleteErr: errs.New(http.StatusBadRequest, `{"code":40035008,"message":"not found"}`, "")}
	out, _ = DeleteMediaImage(&miClient{}, nil, delMock, callapi.ActionMessage{
		Action: "delete_media_image",
		Params: callapi.ParamsContent{ImageID: "img-gone"},
	})
	resp = parseMIMessage(t, out)
	if resp.RetCode != 1401 || !strings.Contains(resp.Message, "40035008") {
		t.Fatalf("delete 40035008 应转 1401 且含原码: %+v", resp)
	}

	// image_id 为空 → 1400
	out, _ = DeleteMediaImage(&miClient{}, nil, &miMockAPI{}, callapi.ActionMessage{Action: "delete_media_image"})
	if resp = parseMIMessage(t, out); resp.RetCode != 1400 {
		t.Fatalf("空 image_id 应回 1400: %+v", resp)
	}
}

// TestMediaImage_ListEmpty (6) 列表空返回不 panic，images 为 [] 而非 null
func TestMediaImage_ListEmpty(t *testing.T) {
	mock := &miMockAPI{listResp: &dto.MediaImageList{}}
	out, err := GetMediaImageList(&miClient{}, nil, mock, callapi.ActionMessage{Action: "get_media_image_list"})
	if err != nil {
		t.Fatalf("handler 错误: %v", err)
	}
	resp := parseMIMessage(t, out)
	if resp.RetCode != 0 {
		t.Fatalf("空列表应成功: %+v", resp)
	}
	var data MediaImageListResponse
	if err := json.Unmarshal(resp.Data, &data); err != nil {
		t.Fatalf("data 解析失败: %v", err)
	}
	if data.Total != 0 || data.NextCursor != "" || data.Next {
		t.Fatalf("空列表分页字段: %+v", data)
	}
	if strings.Contains(string(resp.Data), `"images":null`) {
		t.Fatalf("images 应为空数组: %s", string(resp.Data))
	}
	if len(data.Images) != 0 {
		t.Fatalf("images 应为空: %+v", data.Images)
	}

	// nil 列表同样不 panic
	out, _ = GetMediaImageList(&miClient{}, nil, &miMockAPI{}, callapi.ActionMessage{Action: "get_media_image_list"})
	if resp = parseMIMessage(t, out); resp.RetCode != 0 {
		t.Fatalf("nil 列表应成功: %+v", resp)
	}
}

// TestMediaImage_DeleteSuccess 删除成功回 retcode 0 与空 data
func TestMediaImage_DeleteSuccess(t *testing.T) {
	mock := &miMockAPI{}
	out, err := DeleteMediaImage(&miClient{}, nil, mock, callapi.ActionMessage{
		Action: "delete_media_image",
		Params: callapi.ParamsContent{ImageID: "img-1"},
	})
	if err != nil {
		t.Fatalf("handler 错误: %v", err)
	}
	resp := parseMIMessage(t, out)
	if resp.RetCode != 0 || string(resp.Data) != "{}" {
		t.Fatalf("删除回包: %+v data=%s", resp, string(resp.Data))
	}
	if len(mock.deletedIDs) != 1 || mock.deletedIDs[0] != "img-1" {
		t.Fatalf("删除入参: %v", mock.deletedIDs)
	}
}

// TestMediaImage_MarkdownQQBotMediaPassthrough (8) qqbot-media 引用不被本地路径分支吃掉
// 传 nil apiv2：一旦走本地读取→上传支路必然 panic，透传则原样返回
func TestMediaImage_MarkdownQQBotMediaPassthrough(t *testing.T) {
	const content = "![说明 #100px #100px](qqbot-media://image/v1/token-abc)\n[链接](https://example.com/a.png)"
	got := ResolveMarkdownImages(content, nil)
	if got != content {
		t.Fatalf("qqbot-media 引用被改写:\n got: %s\n want: %s", got, content)
	}

	// HTML img 标签同理
	const html = `<img src="qqbot-media://image/v1/token-def">`
	if got := ResolveMarkdownImages(html, nil); got != html {
		t.Fatalf("html img 中的 qqbot-media 被改写: %s", got)
	}
}

// TestMediaImage_ETagNormalize (4) ETag 归一：去引号、去 W/ 前缀、转小写
func TestMediaImage_ETagNormalize(t *testing.T) {
	cases := map[string]string{
		`"d41d8cd98f00b204e9800998ecf8427e"`:     "d41d8cd98f00b204e9800998ecf8427e",
		`W/"d41d8cd98f00b204e9800998ecf8427e"`:   "d41d8cd98f00b204e9800998ecf8427e",
		`  "D41D8CD98F00B204E9800998ECF8427E"  `: "d41d8cd98f00b204e9800998ecf8427e",
		"":                                       "",
	}
	for etag, want := range cases {
		if got := normalizeMediaImageETag(etag); got != want {
			t.Fatalf("ETag %q: got %q, want %q", etag, got, want)
		}
	}
}

// TestMediaImage_FileSourceRejected http(s) 入参直接拒绝（prepare 无法代表远端内容）
func TestMediaImage_FileSourceRejected(t *testing.T) {
	mock := &miMockAPI{}
	out, _ := UploadMediaImage(&miClient{}, nil, mock, callapi.ActionMessage{
		Action: "upload_media_image",
		Params: callapi.ParamsContent{File: "https://example.com/a.png"},
	})
	resp := parseMIMessage(t, out)
	if resp.RetCode != 1400 || !strings.Contains(resp.Message, "http") {
		t.Fatalf("http 入参应拒: %+v", resp)
	}
	if mock.prepareReq != nil {
		t.Fatal("http 入参不得发起 prepare")
	}

	// 本地路径读不到 → 1404
	out, _ = UploadMediaImage(&miClient{}, nil, mock, callapi.ActionMessage{
		Action: "upload_media_image",
		Params: callapi.ParamsContent{File: filepath.Join(t.TempDir(), "missing.png")},
	})
	if resp = parseMIMessage(t, out); resp.RetCode != 1404 {
		t.Fatalf("读不到文件应回 1404: %+v", resp)
	}
}

// shadow 预演：素材三个 action 都不得触达上游（上传会占配额、删除不可回滚）。
func TestMediaImage_ShadowDryRunSkipsUpstream(t *testing.T) {
	mock := &miMockAPI{}
	client := &miClient{}

	cases := []struct {
		action string
		params callapi.ParamsContent
		callFn func(callapi.Client, openapi.OpenAPI, openapi.OpenAPI, callapi.ActionMessage) (string, error)
	}{
		{
			"upload_media_image",
			callapi.ParamsContent{File: "base64://" + base64.StdEncoding.EncodeToString([]byte("x"))},
			UploadMediaImage,
		},
		{"get_media_image_list", callapi.ParamsContent{}, GetMediaImageList},
		{"delete_media_image", callapi.ParamsContent{ImageID: "img-1"}, DeleteMediaImage},
	}

	for _, tc := range cases {
		t.Run(tc.action, func(t *testing.T) {
			out, err := tc.callFn(client, nil, mock, callapi.ActionMessage{
				Action:       tc.action,
				Params:       tc.params,
				ShadowDryRun: true,
			})
			if err != nil {
				t.Fatalf("dry-run 返回错误: %v", err)
			}
			resp := parseMIMessage(t, out)
			if resp.RetCode != 0 {
				t.Fatalf("dry-run 应回 retcode=0, got %+v", resp)
			}
			if !strings.Contains(string(resp.Data), `"dry_run":true`) {
				t.Fatalf("dry-run 应回占位 data, got %s", resp.Data)
			}
		})
	}

	if mock.prepareReq != nil || mock.completeReq != nil {
		t.Fatalf("dry-run 不得调用 prepare/complete")
	}
	if len(mock.deletedIDs) != 0 {
		t.Fatalf("dry-run 不得真删素材: %v", mock.deletedIDs)
	}
}

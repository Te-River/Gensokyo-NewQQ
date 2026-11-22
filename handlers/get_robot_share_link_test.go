package handlers

// 回归（v016）：get_robot_share_link 曾只推伪造 notice、不补 action 回执，
// 客户端收不到 `{}` 响应只能等到超时。

import (
	"context"
	"errors"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

// mockShareLinkOpenAPI 捕获 GenerateURLLink 入参并按预设返回结果。
type mockShareLinkOpenAPI struct {
	openapi.OpenAPI

	url          string
	err          error
	calls        int
	lastCallback string
}

func (m *mockShareLinkOpenAPI) GenerateURLLink(ctx context.Context, params *dto.GenerateURLLinkToCreate) (*dto.GenerateURLLink, error) {
	m.calls++
	m.lastCallback = params.CallbackData
	if m.err != nil {
		return nil, m.err
	}
	return &dto.GenerateURLLink{URL: m.url}, nil
}

// shareLinkTestClient 按序记录 handler 发出的每一条消息。
type shareLinkTestClient struct {
	msgs []map[string]interface{}
}

func (c *shareLinkTestClient) SendMessage(message map[string]interface{}) error {
	c.msgs = append(c.msgs, message)
	return nil
}

// TestGetRobotShareLinkSendsNoticeThenReceipt 成功路径：先 notice（带上游 URL），再补 `{}` 回执。
func TestGetRobotShareLinkSendsNoticeThenReceipt(t *testing.T) {
	const wantURL = "https://bot.q.qq.com/s/mockShareLink"
	client := &shareLinkTestClient{}
	mock := &mockShareLinkOpenAPI{url: wantURL}
	msg := callapi.ActionMessage{
		Action: "get_robot_share_link",
		Params: callapi.ParamsContent{CallbackData: "cb-share"},
		Echo:   "echo-share",
	}

	raw, err := GetRobotShareLink(client, nil, mock, msg)
	if err != nil {
		t.Fatalf("GetRobotShareLink 返回错误: %v", err)
	}
	if raw != "{}" {
		t.Errorf("返回值 = %q, want %q", raw, "{}")
	}
	if mock.calls != 1 || mock.lastCallback != "cb-share" {
		t.Fatalf("GenerateURLLink 调用 %d 次、callback_data = %q, want 1 次 / cb-share", mock.calls, mock.lastCallback)
	}
	if len(client.msgs) != 2 {
		t.Fatalf("客户端应收到 2 条消息（notice + 回执），实际 %d 条: %v", len(client.msgs), client.msgs)
	}

	notice := client.msgs[0]
	if notice["post_type"] != "notice" || notice["notice_type"] != "share_link_generated" {
		t.Errorf("第 1 条应为 share_link_generated notice，实际: %v", notice)
	}
	if notice["url"] != wantURL {
		t.Errorf("notice.url = %v, want %s", notice["url"], wantURL)
	}

	if len(client.msgs[1]) != 0 {
		t.Errorf("第 2 条应为空对象回执，实际: %v", client.msgs[1])
	}
}

// TestGetRobotShareLinkUpstreamErrorSendsOnlyFailReceipt 失败路径：只回 1500 失败回执，不推 notice。
func TestGetRobotShareLinkUpstreamErrorSendsOnlyFailReceipt(t *testing.T) {
	client := &shareLinkTestClient{}
	mock := &mockShareLinkOpenAPI{err: errors.New("upstream boom")}
	msg := callapi.ActionMessage{
		Action: "get_robot_share_link",
		Params: callapi.ParamsContent{CallbackData: "cb-fail"},
		Echo:   "echo-share-fail",
	}

	raw, err := GetRobotShareLink(client, nil, mock, msg)
	if err != nil {
		t.Fatalf("GetRobotShareLink 返回错误: %v", err)
	}
	if len(client.msgs) != 1 {
		t.Fatalf("失败路径应只发 1 条回执，实际 %d 条: %v", len(client.msgs), client.msgs)
	}

	receipt := client.msgs[0]
	// structToMap 经 JSON 往返，数字一律是 float64。
	retCode, _ := receipt["retcode"].(float64)
	if retCode != float64(sendFailRetCode) || receipt["status"] != "failed" {
		t.Errorf("失败回执 retcode/status = %v/%v, want %d/failed", receipt["retcode"], receipt["status"], sendFailRetCode)
	}
	if receipt["echo"] != "echo-share-fail" {
		t.Errorf("失败回执 echo = %v, want echo-share-fail", receipt["echo"])
	}
	if _, ok := receipt["notice_type"]; ok {
		t.Errorf("失败路径不应推出 notice: %v", receipt)
	}
	got := decodeActionReceipt(t, raw)
	if got["status"] != "failed" {
		t.Errorf("返回体 status = %v, want failed", got["status"])
	}
}

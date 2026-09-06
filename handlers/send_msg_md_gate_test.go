package handlers

import (
	"context"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

// mockMDSendOpenAPI 捕获 PostGroupMessage / PostC2CMessage，
// 其余方法走嵌入接口兜底 panic（与 group_member_test.go 同款桩风格）。
type mockMDSendOpenAPI struct {
	openapi.OpenAPI

	groupCalls  int
	lastGroupID string
	lastGroup   dto.APIMessage

	c2cCalls  int
	lastC2CID string
	lastC2C   dto.APIMessage
}

func (m *mockMDSendOpenAPI) PostGroupMessage(ctx context.Context, groupID string, msg dto.APIMessage) (*dto.GroupMessageResponse, error) {
	m.groupCalls++
	m.lastGroupID = groupID
	m.lastGroup = msg
	return &dto.GroupMessageResponse{Message: &dto.Message{ID: "mock-group-msg-id"}}, nil
}

func (m *mockMDSendOpenAPI) PostC2CMessage(ctx context.Context, userID string, msg dto.APIMessage) (*dto.C2CMessageResponse, error) {
	m.c2cCalls++
	m.lastC2CID = userID
	m.lastC2C = msg
	return &dto.C2CMessageResponse{Message: &dto.Message{ID: "mock-c2c-msg-id"}}, nil
}

// mdGateTestClient 记录 handler 发出的回执 payload
type mdGateTestClient struct {
	response map[string]interface{}
}

func (c *mdGateTestClient) SendMessage(message map[string]interface{}) error {
	c.response = message
	return nil
}

// mdSegment 构造下游常见的段数组嵌套格式：{"type":"markdown","data":{"data":{"markdown":{"content":...}}}}
func mdSegment(content string) map[string]interface{} {
	return map[string]interface{}{
		"type": "markdown",
		"data": map[string]interface{}{
			"data": map[string]interface{}{
				"markdown": map[string]interface{}{
					"content": content,
				},
			},
		},
	}
}

// textSegment 构造 {"type":"text","data":{"text":...}}
func textSegment(text string) map[string]interface{} {
	return map[string]interface{}{
		"type": "text",
		"data": map[string]interface{}{"text": text},
	}
}

// keyboardSegment 构造 {"type":"keyboard","data":{"data":{"id":...}}}（按钮模板形态）
func keyboardSegment(id string) map[string]interface{} {
	return map[string]interface{}{
		"type": "keyboard",
		"data": map[string]interface{}{
			"data": map[string]interface{}{"id": id},
		},
	}
}

// TestSendGroupMsgMarkdownOnlySegment md-only 段数组群消息：
// 门条件修复前 messageText 为空 → md/keyboard 处理块被跳过，消息静默丢弃（无 PostGroupMessage）。
// 修复后应走 MsgType=2 发送，Markdown.Content 无前导换行。
func TestSendGroupMsgMarkdownOnlySegment(t *testing.T) {
	mock := &mockMDSendOpenAPI{}
	client := &mdGateTestClient{}

	msg := callapi.ActionMessage{
		Action: "send_group_msg",
		Params: callapi.ParamsContent{
			GroupID: openID32('g'),
			UserID:  openID32('u'), // legacy 解析管道对 nil UserID 会断言 panic，测试显式带上
			Message: []interface{}{mdSegment("### AT 检测结果")},
		},
		Echo: "md-gate-group-mdonly",
	}

	if _, err := HandleSendGroupMsg(client, nil, mock, msg); err != nil {
		t.Fatalf("HandleSendGroupMsg 返回错误: %v", err)
	}
	if mock.groupCalls != 1 {
		t.Fatalf("md-only 群消息应调用一次 PostGroupMessage, got %d", mock.groupCalls)
	}
	mtc, ok := mock.lastGroup.(*dto.MessageToCreate)
	if !ok {
		t.Fatalf("应发送 MessageToCreate, got %T", mock.lastGroup)
	}
	if mtc.MsgType != 2 {
		t.Errorf("MsgType = %d, want 2", mtc.MsgType)
	}
	if mtc.Markdown == nil || mtc.Markdown.Content != "### AT 检测结果" {
		t.Errorf("Markdown.Content = %+v, want 原文（无前导换行）", mtc.Markdown)
	}
	if mtc.Content != "" {
		t.Errorf("Content = %q, want 空串", mtc.Content)
	}
}

// TestSendGroupMsgMarkdownWithTextSegment md+text 段数组群消息：
// text 行应拼接到 markdown 内容头部（拼接行为不回归）。
func TestSendGroupMsgMarkdownWithTextSegment(t *testing.T) {
	mock := &mockMDSendOpenAPI{}
	client := &mdGateTestClient{}

	msg := callapi.ActionMessage{
		Action: "send_group_msg",
		Params: callapi.ParamsContent{
			GroupID: openID32('g'),
			UserID:  openID32('u'),
			Message: []interface{}{textSegment("text行"), mdSegment("### md内容")},
		},
		Echo: "md-gate-group-mdtext",
	}

	if _, err := HandleSendGroupMsg(client, nil, mock, msg); err != nil {
		t.Fatalf("HandleSendGroupMsg 返回错误: %v", err)
	}
	if mock.groupCalls != 1 {
		t.Fatalf("md+text 群消息应调用一次 PostGroupMessage, got %d", mock.groupCalls)
	}
	mtc, ok := mock.lastGroup.(*dto.MessageToCreate)
	if !ok {
		t.Fatalf("应发送 MessageToCreate, got %T", mock.lastGroup)
	}
	if mtc.MsgType != 2 {
		t.Errorf("MsgType = %d, want 2", mtc.MsgType)
	}
	want := "text行\n### md内容"
	if mtc.Markdown == nil || mtc.Markdown.Content != want {
		t.Errorf("Markdown.Content = %+v, want %q", mtc.Markdown, want)
	}
}

// TestSendPrivateMsgMarkdownOnlySegment md-only 段数组私聊消息：
// 门条件修复前被 strings.TrimSpace(messageText) 拦住，静默丢弃。
// 修复后应走 MsgType=2 PostC2CMessage（私聊 md 分支不拼接 messageText）。
func TestSendPrivateMsgMarkdownOnlySegment(t *testing.T) {
	mock := &mockMDSendOpenAPI{}
	client := &mdGateTestClient{}

	msg := callapi.ActionMessage{
		Action: "send_private_msg",
		Params: callapi.ParamsContent{
			UserID:  openID32('u'),
			Message: []interface{}{mdSegment("### AT 检测结果")},
		},
		Echo: "md-gate-private-mdonly",
	}

	if _, err := HandleSendPrivateMsg(client, nil, mock, msg); err != nil {
		t.Fatalf("HandleSendPrivateMsg 返回错误: %v", err)
	}
	if mock.c2cCalls != 1 {
		t.Fatalf("md-only 私聊消息应调用一次 PostC2CMessage, got %d", mock.c2cCalls)
	}
	mtc, ok := mock.lastC2C.(*dto.MessageToCreate)
	if !ok {
		t.Fatalf("应发送 MessageToCreate, got %T", mock.lastC2C)
	}
	if mtc.MsgType != 2 {
		t.Errorf("MsgType = %d, want 2", mtc.MsgType)
	}
	if mtc.Markdown == nil || mtc.Markdown.Content != "### AT 检测结果" {
		t.Errorf("Markdown.Content = %+v, want 原文", mtc.Markdown)
	}
}

// TestSendGroupMsgKeyboardOnlySegment kb-only 段数组群消息：
// 门条件修复前静默丢弃，修复后应有发送尝试且 Keyboard 非 nil。
func TestSendGroupMsgKeyboardOnlySegment(t *testing.T) {
	mock := &mockMDSendOpenAPI{}
	client := &mdGateTestClient{}

	msg := callapi.ActionMessage{
		Action: "send_group_msg",
		Params: callapi.ParamsContent{
			GroupID: openID32('g'),
			UserID:  openID32('u'),
			Message: []interface{}{keyboardSegment("tpl_1")},
		},
		Echo: "md-gate-group-kbonly",
	}

	if _, err := HandleSendGroupMsg(client, nil, mock, msg); err != nil {
		t.Fatalf("HandleSendGroupMsg 返回错误: %v", err)
	}
	if mock.groupCalls != 1 {
		t.Fatalf("kb-only 群消息应调用一次 PostGroupMessage, got %d", mock.groupCalls)
	}
	mtc, ok := mock.lastGroup.(*dto.MessageToCreate)
	if !ok {
		t.Fatalf("应发送 MessageToCreate, got %T", mock.lastGroup)
	}
	if mtc.Keyboard == nil {
		t.Fatal("Keyboard 应非 nil")
	}
	if mtc.Keyboard.ID != "tpl_1" {
		t.Errorf("Keyboard.ID = %q, want tpl_1", mtc.Keyboard.ID)
	}
}

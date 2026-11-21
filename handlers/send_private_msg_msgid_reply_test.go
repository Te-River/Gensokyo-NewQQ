package handlers

// 回归（QQ 2026-11-17：群 msg_id 可直接用于私聊被动回复）：
// 钉住 send_private_msg 把「被动回复凭据」写进出站 MessageToCreate.MsgID、且 EventID 为空。
// 断言分三层，便于定位缺口落在哪一环：
//  1. 构造层：generatePrivateMessage 必须原样带 MsgID、EventID 为空（当前已成立）；
//  2. 凭据来源层：echo 缓存命中时，真正发给 OpenAPI 的 MessageToCreate 带该 msg_id；
//  3. 入参层：显式 params.message_id 必须被当作被动回复凭据（当前缺失，见测试内注释）。

import (
	"testing"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/echo"
	"github.com/tencent-connect/botgo/dto"
)

const (
	explicitGroupMsgID = "ROBOT1.0_FROM_GROUP"
	echoCacheMsgID     = "ROBOT1.0_FROM_ECHO_CACHE"
)

// assertPrivateMsgID 断言最近一次 PostC2CMessage 的凭据字段。
func assertPrivateMsgID(t *testing.T, mock *mockMDSendOpenAPI, wantMsgID string) {
	t.Helper()
	if mock.c2cCalls != 1 {
		t.Fatalf("应调用一次 PostC2CMessage, got %d", mock.c2cCalls)
	}
	mtc, ok := mock.lastC2C.(*dto.MessageToCreate)
	if !ok {
		t.Fatalf("应发送 MessageToCreate, got %T", mock.lastC2C)
	}
	if mtc.MsgID != wantMsgID {
		t.Errorf("MessageToCreate.MsgID = %q, want %q", mtc.MsgID, wantMsgID)
	}
	if mtc.EventID != "" {
		t.Errorf("被动回复时 MessageToCreate.EventID 必须为空, got %q", mtc.EventID)
	}
}

// TestSendPrivateMsgExplicitMsgIDBuildLayer 构造层：显式 msg_id 原样进入 MessageToCreate。
func TestSendPrivateMsgExplicitMsgIDBuildLayer(t *testing.T) {
	mock := &mockMDSendOpenAPI{}

	got := generatePrivateMessage(explicitGroupMsgID, "", nil, "hi", 1, mock, openID32('m'))
	mtc, ok := got.(*dto.MessageToCreate)
	if !ok {
		t.Fatalf("文本消息应构造 *dto.MessageToCreate, got %T", got)
	}
	if mtc.MsgID != explicitGroupMsgID {
		t.Errorf("MsgID = %q, want %q", mtc.MsgID, explicitGroupMsgID)
	}
	if mtc.EventID != "" {
		t.Errorf("EventID 应为空, got %q", mtc.EventID)
	}
	if mtc.Content != "hi" {
		t.Errorf("Content = %q, want hi", mtc.Content)
	}
}

// TestSendPrivateMsgEchoCachedMsgIDForwarded 凭据来源层：echo 缓存命中 → 出站 MsgID 生效。
// send_private_msg.go:329-333 用原始 echo 字符串查 echo.GetMsgIDByKey，而 AddMsgIDv3 的 key
// 固定为 "appid_s"；取 appid 为空的 "_s" 形态即可让两者相等（不改产品代码）。
func TestSendPrivateMsgEchoCachedMsgIDForwarded(t *testing.T) {
	const echoKey = "_gsk-msgid-reply-echo"
	echo.AddMsgIDv3("", "gsk-msgid-reply-echo", echoCacheMsgID)

	mock := &mockMDSendOpenAPI{}
	client := &mdGateTestClient{}
	msg := callapi.ActionMessage{
		Action: "send_private_msg",
		Params: callapi.ParamsContent{
			UserID:  openID32('m'),
			Message: "hi",
		},
		Echo: echoKey,
	}

	if _, err := HandleSendPrivateMsg(client, nil, mock, msg); err != nil {
		t.Fatalf("HandleSendPrivateMsg 返回错误: %v", err)
	}
	assertPrivateMsgID(t, mock, echoCacheMsgID)
}

// TestSendPrivateMsgExplicitMsgIDForwarded 入参层（本次需求）：params.message_id 作为被动回复凭据。
// 现状：send_private_msg.go 只用 lazy 池 / echo 缓存 / GetMessageIDByUseridOrGroupid 取 msg_id，
// 从不读取 message.Params.MessageID（全仓仅 delete_msg.go / send_group_msg_raw.go 读取），
// 因此显式传入的群 msg_id 会被丢弃 → 本用例失败即为 PRODUCT_BUG（凭据未接线，非构造层问题）。
func TestSendPrivateMsgExplicitMsgIDForwarded(t *testing.T) {
	mock := &mockMDSendOpenAPI{}
	client := &mdGateTestClient{}
	msg := callapi.ActionMessage{
		Action: "send_private_msg",
		Params: callapi.ParamsContent{
			UserID:    openID32('n'),
			MessageID: explicitGroupMsgID,
			Message:   "hi",
		},
		Echo: "gsk-msgid-reply-explicit",
	}

	if _, err := HandleSendPrivateMsg(client, nil, mock, msg); err != nil {
		t.Fatalf("HandleSendPrivateMsg 返回错误: %v", err)
	}
	assertPrivateMsgID(t, mock, explicitGroupMsgID)
}

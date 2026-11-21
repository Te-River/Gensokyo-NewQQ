package handlers

// 回归（v016）：消息已发出但虚拟 message_id 写缓存失败时，曾裸 return "", nil，
// 客户端既收不到 SendMessage 回执也收不到 JSON，只能等超时；而消息其实已发出，
// 回 failed 又会误导插件重发，故统一回成功回执 + data.message_id=0。

import (
	"errors"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/tencent-connect/botgo/dto"
)

// failCacheStore 让写缓存必然失败，用例结束后恢复默认实现（包级注入缝，用例禁止 t.Parallel()）。
func failCacheStore(t *testing.T) {
	t.Helper()
	old := storeVirtualMessageID
	storeVirtualMessageID = func(string) (int64, error) {
		return 0, errors.New("mock 缓存写入失败")
	}
	t.Cleanup(func() { storeVirtualMessageID = old })
}

// assertOKReceipt 断言客户端确实收到成功回执，且 data.message_id 为 want。
func assertOKReceipt(t *testing.T, client *contractTestClient, raw string, wantMessageID float64, wantEcho string) {
	t.Helper()
	if raw == "" {
		t.Fatal("写缓存失败后返回空串：客户端拿不到 JSON 回执")
	}
	if client.last == nil {
		t.Fatal("客户端未收到 SendMessage 回执（静默无响应）")
	}
	if client.last["retcode"] != float64(0) {
		t.Errorf("回执 retcode = %v, want 0", client.last["retcode"])
	}
	if client.last["status"] != "ok" {
		t.Errorf("回执 status = %v, want ok", client.last["status"])
	}
	if client.last["message"] != "" {
		t.Errorf("回执 message = %v, want 空串", client.last["message"])
	}
	if client.last["echo"] != wantEcho {
		t.Errorf("回执 echo = %v, want %s", client.last["echo"], wantEcho)
	}
	got := decodeActionReceipt(t, raw)
	data, ok := got["data"].(map[string]interface{})
	if !ok {
		t.Fatalf("data 不是对象: %v", got["data"])
	}
	if data["message_id"] != wantMessageID {
		t.Errorf("data.message_id = %v, want %v", data["message_id"], wantMessageID)
	}
	if got["retcode"] != float64(0) || got["status"] != "ok" || got["echo"] != wantEcho {
		t.Errorf("返回体与回执不一致: %v", got)
	}
}

// TestSendResponseCacheFailureRepliesSuccess 群消息：缓存失败仍回 retcode 0 / message_id 0。
func TestSendResponseCacheFailureRepliesSuccess(t *testing.T) {
	failCacheStore(t)
	client := &contractTestClient{}
	msg := callapi.ActionMessage{
		Action: "send_group_msg",
		Params: callapi.ParamsContent{GroupID: "9600000001", UserID: "9600000002"},
		Echo:   "echo-cache-group",
	}

	raw, err := SendResponse(client, nil, &msg, &dto.GroupMessageResponse{Message: &dto.Message{ID: "real-msg-1"}}, nil, nil)
	if err != nil {
		t.Fatalf("SendResponse 返回错误: %v", err)
	}
	assertOKReceipt(t, client, raw, 0, "echo-cache-group")
}

// TestSendGuildResponseCacheFailureRepliesSuccess 频道消息：缓存失败仍回 retcode 0 / message_id 0。
func TestSendGuildResponseCacheFailureRepliesSuccess(t *testing.T) {
	failCacheStore(t)
	client := &contractTestClient{}
	msg := callapi.ActionMessage{
		Action: "send_guild_msg",
		Params: callapi.ParamsContent{ChannelID: "9600000003", GuildID: "9600000004", UserID: "9600000005"},
		Echo:   "echo-cache-guild",
	}

	raw, err := SendGuildResponse(client, nil, &msg, &dto.Message{ID: "real-msg-2"})
	if err != nil {
		t.Fatalf("SendGuildResponse 返回错误: %v", err)
	}
	assertOKReceipt(t, client, raw, 0, "echo-cache-guild")
}

// TestSendC2CResponseCacheFailureRepliesSuccess 单聊消息：缓存失败仍回 retcode 0 / message_id 0。
func TestSendC2CResponseCacheFailureRepliesSuccess(t *testing.T) {
	failCacheStore(t)
	client := &contractTestClient{}
	msg := callapi.ActionMessage{
		Action: "send_private_msg",
		Params: callapi.ParamsContent{UserID: "9600000006"},
		Echo:   "echo-cache-c2c",
	}

	raw, err := SendC2CResponse(client, nil, &msg, &dto.C2CMessageResponse{Message: &dto.Message{ID: "real-msg-3"}})
	if err != nil {
		t.Fatalf("SendC2CResponse 返回错误: %v", err)
	}
	assertOKReceipt(t, client, raw, 0, "echo-cache-c2c")
}

// TestSendGuildPrivateResponseCacheFailureRepliesSuccess 频道私信：缓存失败仍回 retcode 0 / message_id 0。
func TestSendGuildPrivateResponseCacheFailureRepliesSuccess(t *testing.T) {
	failCacheStore(t)
	client := &contractTestClient{}
	msg := callapi.ActionMessage{
		Action: "send_guild_private_msg",
		Params: callapi.ParamsContent{UserID: "9600000007", GuildID: "9600000008"},
		Echo:   "echo-cache-guild-private",
	}

	raw, err := SendGuildPrivateResponse(client, nil, &msg, &dto.Message{ID: "real-msg-4"}, "9600000008")
	if err != nil {
		t.Fatalf("SendGuildPrivateResponse 返回错误: %v", err)
	}
	assertOKReceipt(t, client, raw, 0, "echo-cache-guild-private")
}

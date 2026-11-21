package handlers

// 回归（v016）：idmap 未映射的 user_id / group_id 曾被 handler 裸 return "", nil，
// 反向 WS 客户端收不到任何回执只能等到超时断开。

import (
	"encoding/json"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/echo"
)

// decodeActionReceipt 解析回执 JSON 为 map，便于按字段断言。
func decodeActionReceipt(t *testing.T, raw string) map[string]interface{} {
	t.Helper()
	var got map[string]interface{}
	if err := json.Unmarshal([]byte(raw), &got); err != nil {
		t.Fatalf("回执不是合法 JSON: %v (%s)", err, raw)
	}
	return got
}

// assertFailedReceipt 断言「客户端确实收到了一次失败回执」：retcode/status/echo 三项对齐。
func assertFailedReceipt(t *testing.T, client *contractTestClient, raw string, wantRetCode int, wantEcho string) {
	t.Helper()
	if client.last == nil {
		t.Fatalf("客户端未收到任何 SendMessage 回执（静默无响应）: raw=%s", raw)
	}
	if client.last["retcode"] != float64(wantRetCode) {
		t.Errorf("SendMessage 回执 retcode = %v, want %d", client.last["retcode"], wantRetCode)
	}
	if client.last["status"] != "failed" {
		t.Errorf("SendMessage 回执 status = %v, want failed", client.last["status"])
	}
	if client.last["echo"] != wantEcho {
		t.Errorf("SendMessage 回执 echo = %v, want %s", client.last["echo"], wantEcho)
	}
	got := decodeActionReceipt(t, raw)
	if got["status"] != "failed" || got["echo"] != wantEcho {
		t.Errorf("返回体与回执不一致: %v", got)
	}
	if data, ok := got["data"]; !ok || data != nil {
		t.Errorf("data 应为 null: %v", data)
	}
}

// TestSendPrivateMsgUnmappedUserIDReplies 未映射的 user_id → 回执 1400，不再静默返回。
func TestSendPrivateMsgUnmappedUserIDReplies(t *testing.T) {
	client := &contractTestClient{}
	msg := callapi.ActionMessage{
		Action: "send_private_msg",
		Params: callapi.ParamsContent{
			UserID:  "9400000001",
			Message: "hi",
		},
		Echo: "echo-unmapped-private",
	}

	raw, err := HandleSendPrivateMsg(client, nil, nil, msg)
	if err != nil {
		t.Fatalf("HandleSendPrivateMsg 返回错误: %v", err)
	}
	assertFailedReceipt(t, client, raw, sendParamRetCode, "echo-unmapped-private")
}

// TestSendGroupMsgUnmappedGroupIDReplies 未映射的 group_id → 回执 1400，且不调用上游。
func TestSendGroupMsgUnmappedGroupIDReplies(t *testing.T) {
	// 预置消息类型为 group，跳过 handler 内的类型枚举递归，直达 group 分支的反查。
	idInt64, err := ConvertToInt64("9500000001")
	if err != nil {
		t.Fatalf("ConvertToInt64 失败: %v", err)
	}
	echo.AddMsgType(config.GetAppIDStr(), idInt64, "group")

	client := &contractTestClient{}
	mock := &mockMDSendOpenAPI{}
	msg := callapi.ActionMessage{
		Action: "send_group_msg",
		Params: callapi.ParamsContent{
			GroupID: "9500000001",
			UserID:  "9500000002",
			Message: "hi",
		},
		Echo: "echo-unmapped-group",
	}

	raw, err := HandleSendGroupMsg(client, nil, mock, msg)
	if err != nil {
		t.Fatalf("HandleSendGroupMsg 返回错误: %v", err)
	}
	if mock.groupCalls != 0 {
		t.Errorf("反查失败时不应调用 PostGroupMessage，实际调用 %d 次", mock.groupCalls)
	}
	assertFailedReceipt(t, client, raw, sendParamRetCode, "echo-unmapped-group")
}

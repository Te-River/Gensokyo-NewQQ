package handlers

// 回归：内部代码（CQ 解析、事件转 action、webhook 转发）会直接构造 callapi.ActionMessage，
// 此时 Params.UserID / GroupID / MessageID 可能为 nil。旧实现对这些字段直接做 .(string)
// 断言，命中 nil 时 panic——RecoverPanic 只兜住不至于崩溃，却不回执，客户端只能等到超时。
// 本文件钉住：JSON 之外直接构造的缺 id action 一律回 1400，且全程不 panic。

import (
	"encoding/json"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/callapi"
)

// assertParamErrorReceipt 断言 handler 既返回 1400 失败回执，也把同一回执推给了客户端。
func assertParamErrorReceipt(t *testing.T, client *contractTestClient, raw string, err error) {
	t.Helper()
	if err != nil {
		t.Fatalf("handler 返回错误: %v", err)
	}
	var got struct {
		RetCode int    `json:"retcode"`
		Status  string `json:"status"`
	}
	if err := json.Unmarshal([]byte(raw), &got); err != nil {
		t.Fatalf("回执不是合法 JSON: %v (%s)", err, raw)
	}
	if got.RetCode != sendParamRetCode {
		t.Fatalf("retcode = %d, want %d (%s)", got.RetCode, sendParamRetCode, raw)
	}
	if got.Status != "failed" {
		t.Errorf("status = %q, want failed (%s)", got.Status, raw)
	}
	if code, ok := client.last["retcode"]; !ok || code != float64(sendParamRetCode) {
		t.Fatalf("客户端收到的回执 retcode = %v, want %d", code, sendParamRetCode)
	}
}

func TestParamsIDSafeOnNonString(t *testing.T) {
	cases := []struct {
		name string
		in   interface{}
		want string
	}{
		{"nil", nil, ""},
		{"string", "12345", "12345"},
		{"empty", "", ""},
		{"int", 12345, ""},
		{"float64", float64(12345), ""},
	}
	for _, tc := range cases {
		if got := paramsID(tc.in); got != tc.want {
			t.Errorf("paramsID(%v) = %q, want %q", tc.in, got, tc.want)
		}
	}
}

func TestSendPrivateMsgNilUserIDRepliesParamError(t *testing.T) {
	client := &contractTestClient{}
	message := callapi.ActionMessage{Action: "send_private_msg", Echo: "nil-user-id"}
	message.Params.UserID = nil

	raw, err := HandleSendPrivateMsg(client, nil, nil, message)
	assertParamErrorReceipt(t, client, raw, err)
}

func TestSendGroupMsgNilGroupIDRepliesParamError(t *testing.T) {
	client := &contractTestClient{}
	message := callapi.ActionMessage{Action: "send_group_msg", Echo: "nil-group-id"}
	message.Params.GroupID = nil

	raw, err := HandleSendGroupMsg(client, nil, nil, message)
	assertParamErrorReceipt(t, client, raw, err)
}

// Params 为零值（只有 Action/Echo）：两个发送入口都不得 panic，且都回 1400。
func TestSendHandlersZeroValueParamsReplyParamError(t *testing.T) {
	client := &contractTestClient{}

	raw, err := HandleSendPrivateMsg(client, nil, nil, callapi.ActionMessage{Action: "send_private_msg", Echo: "zero-params"})
	assertParamErrorReceipt(t, client, raw, err)

	raw, err = HandleSendGroupMsg(client, nil, nil, callapi.ActionMessage{Action: "send_group_msg", Echo: "zero-params"})
	assertParamErrorReceipt(t, client, raw, err)
}

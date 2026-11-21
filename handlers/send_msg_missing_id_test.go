package handlers

// 回归（v016）：缺失 / 空值的 group_id、user_id 曾绕过参数守卫，走进「注入 msgType 后自调用」
// 的递归分支。注入键是 appID_0，而探测读的是 appID_ 空串，永远读不回 → 无限自调用 →
// fatal error: stack overflow（Go 无法 recover，整个进程退出）。
// 现在递归前必须可推进，递归一次后仍无消息类型则回 1400。

import (
	"encoding/json"
	"testing"
	"time"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/echo"
)

// decodeActionMessage 走线上同一条 JSON 路径构造 ActionMessage：
// ParamsContent.UnmarshalJSON 会把缺失字段归一为 ""（非 nil），这是缺陷复现的前提。
func decodeActionMessage(t *testing.T, raw string) callapi.ActionMessage {
	t.Helper()
	var msg callapi.ActionMessage
	if err := json.Unmarshal([]byte(raw), &msg); err != nil {
		t.Fatalf("解析 action JSON 失败: %v (%s)", err, raw)
	}
	return msg
}

// assertNoRecursion 断言递归块没有真的启动：id 为 0 时一旦递归就会写入 AddMapping(0, 2)，
// 且无限递归会把测试进程的栈打爆而非返回。递归计数是包级共享状态，调用前需用
// resetRecursionProbe 清零，其余用例可能已在 id 0 上留下尾巴（尾块递减计数）。
func resetRecursionProbe() { echo.AddMapping(0, 0) }

func assertNoRecursion(t *testing.T, elapsed time.Duration) {
	t.Helper()
	if elapsed > time.Second {
		t.Fatalf("handler 耗时 %v，递归未收敛", elapsed)
	}
	if got := echo.GetMapping(0); got != 0 {
		t.Fatalf("id 为 0 时写入了递归计数 %d，说明递归块被执行", got)
	}
}

func TestSendGroupMsgMissingIDReplies1400(t *testing.T) {
	cases := []struct {
		name string
		raw  string
	}{
		{"no_id", `{"action":"send_group_msg","params":{"message":"x"},"echo":"g-group_missing"}`},
		{"empty_group_id", `{"action":"send_group_msg","params":{"group_id":"","message":"x"},"echo":"g-group_empty"}`},
		{"zero_group_id", `{"action":"send_group_msg","params":{"group_id":0,"message":"x"},"echo":"g-group_zero"}`},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			msg := decodeActionMessage(t, tc.raw)
			client := &contractTestClient{}
			resetRecursionProbe()
			start := time.Now()
			raw, err := HandleSendGroupMsg(client, nil, nil, msg)
			elapsed := time.Since(start)
			if err != nil {
				t.Fatalf("HandleSendGroupMsg 返回错误: %v", err)
			}
			assertNoRecursion(t, elapsed)
			wantEcho, _ := msg.Echo.(string)
			assertFailedReceipt(t, client, raw, sendParamRetCode, wantEcho)
		})
	}
}

func TestSendPrivateMsgMissingIDReplies1400(t *testing.T) {
	cases := []struct {
		name string
		raw  string
	}{
		{"no_id", `{"action":"send_private_msg","params":{"message":"x"},"echo":"p-private_missing"}`},
		{"empty_user_id", `{"action":"send_private_msg","params":{"user_id":"","message":"x"},"echo":"p-private_empty"}`},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			msg := decodeActionMessage(t, tc.raw)
			client := &contractTestClient{}
			resetRecursionProbe()
			start := time.Now()
			raw, err := HandleSendPrivateMsg(client, nil, nil, msg)
			elapsed := time.Since(start)
			if err != nil {
				t.Fatalf("HandleSendPrivateMsg 返回错误: %v", err)
			}
			assertNoRecursion(t, elapsed)
			wantEcho, _ := msg.Echo.(string)
			assertFailedReceipt(t, client, raw, sendParamRetCode, wantEcho)
		})
	}
}

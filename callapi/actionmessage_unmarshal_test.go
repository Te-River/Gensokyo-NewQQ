package callapi

import (
	"encoding/json"
	"testing"
)

// post_type 放宽只针对 put_interaction：数字与字符串等价，其它 action 仍严格 string。
// 这些用例锁住「放宽的边界」，避免放宽外溢成全局宽松或把 put_interaction 的兼容收回。

// TestActionMessageUnmarshalPostTypePutInteractionAcceptsNumberAndString 断言两种写法
// 都能通过解析（不再落到收包层的 1400），归一结果一致。
func TestActionMessageUnmarshalPostTypePutInteractionAcceptsNumberAndString(t *testing.T) {
	cases := []struct {
		name     string
		payload  string
		wantType string
	}{
		{"数字 0 归一为字符串", `{"action":"put_interaction","post_type":0}`, "0"},
		{"字符串 \"0\" 保持原样", `{"action":"put_interaction","post_type":"0"}`, "0"},
		{"数字 1 归一为字符串", `{"action":"put_interaction","post_type":1}`, "1"},
		{"缺省 post_type 不报错", `{"action":"put_interaction"}`, ""},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			var message ActionMessage
			if err := json.Unmarshal([]byte(tc.payload), &message); err != nil {
				t.Fatalf("put_interaction 应能解析 %s, got error: %v", tc.payload, err)
			}
			if message.Action != "put_interaction" {
				t.Errorf("action = %q, want put_interaction", message.Action)
			}
			if message.PostType != tc.wantType {
				t.Errorf("post_type = %q, want %q", message.PostType, tc.wantType)
			}
		})
	}
}

// TestActionMessageUnmarshalPostTypePutInteractionRejectsNonScalar 断言对象/布尔仍按解析失败，
// 由收包层回 1400——放宽不覆盖非标量类型。
func TestActionMessageUnmarshalPostTypePutInteractionRejectsNonScalar(t *testing.T) {
	payloads := []struct {
		name    string
		payload string
	}{
		{"对象", `{"action":"put_interaction","post_type":{}}`},
		{"布尔", `{"action":"put_interaction","post_type":true}`},
		{"数组", `{"action":"put_interaction","post_type":[0]}`},
	}

	for _, tc := range payloads {
		t.Run(tc.name, func(t *testing.T) {
			var message ActionMessage
			if err := json.Unmarshal([]byte(tc.payload), &message); err == nil {
				t.Fatalf("post_type 为%s时应解析失败: %s", tc.name, tc.payload)
			}
		})
	}
}

// TestActionMessageUnmarshalPostTypeStrictForOtherActions 断言其它 action 的 post_type
// 仍是严格 string：数字失败、字符串成功（放宽没有外溢）。
func TestActionMessageUnmarshalPostTypeStrictForOtherActions(t *testing.T) {
	rejects := []struct {
		name    string
		payload string
	}{
		{"send_group_msg 数字", `{"action":"send_group_msg","post_type":0}`},
		{"send_private_msg 数字", `{"action":"send_private_msg","post_type":0}`},
		{"set_group_add_request 数字", `{"action":"set_group_add_request","post_type":0}`},
	}
	for _, tc := range rejects {
		t.Run(tc.name, func(t *testing.T) {
			var message ActionMessage
			if err := json.Unmarshal([]byte(tc.payload), &message); err == nil {
				t.Fatalf("非 put_interaction 的数字 post_type 应解析失败: %s", tc.payload)
			}
		})
	}

	var message ActionMessage
	if err := json.Unmarshal([]byte(`{"action":"send_group_msg","post_type":"0"}`), &message); err != nil {
		t.Fatalf("非 put_interaction 的字符串 post_type 应保持可解析: %v", err)
	}
	if message.PostType != "0" {
		t.Errorf("post_type = %q, want %q", message.PostType, "0")
	}
}

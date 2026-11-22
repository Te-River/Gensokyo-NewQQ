package callapi

import (
	"encoding/json"
	"strings"
	"testing"
)

// 整包反序列化失败时也必须回执（1400 / failed / data:null），否则客户端只能等到超时。
// echo 从原始报文二次提取，因此回执里的 echo 必须逐字节保留（大整数不能被 float64 改写）。
// 本文件断言 echo 用 RawMessage，才能看到原始字节而不是解码后的值。

// rawReceipt 与 receipt 同形状，但 echo/data 保留原始字节，便于断言精度未被改写。
type rawReceipt struct {
	Data    json.RawMessage `json:"data"`
	Message string          `json:"message"`
	RetCode int             `json:"retcode"`
	Status  string          `json:"status"`
	Echo    json.RawMessage `json:"echo"`
}

func decodeRawReceipt(t *testing.T, raw []byte) rawReceipt {
	t.Helper()
	var r rawReceipt
	if err := json.Unmarshal(raw, &r); err != nil {
		t.Fatalf("回执不是合法 JSON: %v (%s)", err, raw)
	}
	return r
}

// assertUnparseableReceipt 断言一份 1400 解析失败回执，wantEcho 为 echo 的原始 JSON 文本。
func assertUnparseableReceipt(t *testing.T, raw []byte, wantEcho string) {
	t.Helper()
	r := decodeRawReceipt(t, raw)
	if r.RetCode != 1400 {
		t.Errorf("retcode = %d, want 1400 (%s)", r.RetCode, raw)
	}
	if r.Status != "failed" {
		t.Errorf("status = %q, want %q", r.Status, "failed")
	}
	if string(r.Data) != "null" {
		t.Errorf("data = %s, want null", r.Data)
	}
	if !strings.Contains(r.Message, "参数解析失败") {
		t.Errorf("message = %q, want 含「参数解析失败」", r.Message)
	}
	if string(r.Echo) != wantEcho {
		t.Errorf("echo = %s, want %s (%s)", r.Echo, wantEcho, raw)
	}
}

// TestReplyUnparseableMessageReceipt 覆盖回执内容与 echo 保真：调用方若重复回执，
// SendMessage 次数会多于 1；echo 若被 float64 过一遍，大整数会失真。
func TestReplyUnparseableMessageReceipt(t *testing.T) {
	cases := []struct {
		name     string
		payload  string
		wantEcho string
	}{
		{
			name:     "echo 字符串原样回显",
			payload:  `{"echo":"pi-1","action":"put_interaction","post_type":{}}`,
			wantEcho: `"pi-1"`,
		},
		{
			name:     "echo 大整数不被 float64 改写",
			payload:  `{"echo":1234567890123456789,"action":"send_group_msg","post_type":0}`,
			wantEcho: `1234567890123456789`,
		},
		{
			name:     "无 echo 时 echo 为 null",
			payload:  `{"action":"put_interaction","post_type":{}}`,
			wantEcho: `null`,
		},
		{
			name:     "报文不是 JSON 对象时 echo 为 null",
			payload:  `not-json`,
			wantEcho: `null`,
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			var message ActionMessage
			err := json.Unmarshal([]byte(tc.payload), &message)
			if err == nil {
				t.Fatalf("payload 应反序列化失败，实际成功: %s", tc.payload)
			}

			client := &mockClient{}
			result := ReplyUnparseableMessage(client, []byte(tc.payload), err)

			assertUnparseableReceipt(t, []byte(result), tc.wantEcho)

			msgs := client.sent()
			if len(msgs) != 1 {
				t.Fatalf("SendMessage 调用次数 = %d, want 1（解析失败必须回执恰好一次）", len(msgs))
			}
			sentRaw, err := MarshalNoEscape(msgs[0])
			if err != nil {
				t.Fatalf("下发回执无法序列化: %v", err)
			}
			assertUnparseableReceipt(t, sentRaw, tc.wantEcho)
		})
	}
}

// 没有可用的 client 时仍要返回回执字符串，且不能 panic（新链/shadow 可能没注入 client）。
func TestReplyUnparseableMessageNilClient(t *testing.T) {
	raw := []byte(`{"echo":"no-client","action":"put_interaction","post_type":true}`)
	var message ActionMessage
	err := json.Unmarshal(raw, &message)
	if err == nil {
		t.Fatalf("payload 应反序列化失败: %s", raw)
	}

	result := ReplyUnparseableMessage(nil, raw, err)
	assertUnparseableReceipt(t, []byte(result), `"no-client"`)
}

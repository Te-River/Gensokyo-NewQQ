package callapi

import (
	"strings"
	"testing"

	"github.com/tencent-connect/botgo/openapi"
)

// panicHandler 注册一个必定 panic 的临时 action，用于验证分发层的 panic 兜底会回执。
func panicHandler(action string, panicValue interface{}) {
	RegisterHandler(action, func(Client, openapi.OpenAPI, openapi.OpenAPI, ActionMessage) (string, error) {
		panic(panicValue)
	})
}

// assertSinglePanicReceipt 断言收到恰好一份 1500/failed 回执，message 带 panic 值与 action 名。
func assertSinglePanicReceipt(t *testing.T, client *mockClient, result string, action string, panicValue string, echo interface{}) {
	t.Helper()

	msgs := client.sent()
	if len(msgs) != 1 {
		t.Fatalf("SendMessage 调用次数 = %d, want 1（panic 只能回一份回执）", len(msgs))
	}
	sentRaw, err := MarshalNoEscape(msgs[0])
	if err != nil {
		t.Fatalf("下发回执无法序列化: %v", err)
	}

	for _, raw := range [][]byte{[]byte(result), sentRaw} {
		r := decodeReceipt(t, raw)
		if r.RetCode != 1500 {
			t.Errorf("retcode = %d, want 1500 (%s)", r.RetCode, raw)
		}
		if r.Status != "failed" {
			t.Errorf("status = %q, want %q", r.Status, "failed")
		}
		if string(r.Data) != "null" {
			t.Errorf("data = %s, want null", r.Data)
		}
		if !strings.Contains(r.Message, panicValue) || !strings.Contains(r.Message, action) {
			t.Errorf("message = %q, want 同时包含 panic 值 %q 与 action %q", r.Message, panicValue, action)
		}
		if r.Echo != echo {
			t.Errorf("echo = %#v, want %#v", r.Echo, echo)
		}
	}
}

func TestLegacyHandlerPanicSendsReceipt(t *testing.T) {
	const action = "test_legacy_handler_panic"
	panicHandler(action, "legacy boom")

	client := &mockClient{}
	result := callAPIFromDictLegacy(client, nil, nil, ActionMessage{Action: action, Echo: "e-panic-legacy"})

	assertSinglePanicReceipt(t, client, result, action, "legacy boom", "e-panic-legacy")
}

func TestNewHandlerPanicSendsReceipt(t *testing.T) {
	const action = "test_new_handler_panic"
	panicHandler(action, "new boom")

	client := &mockClient{}
	result := callAPIFromDictNew(client, nil, nil, ActionMessage{Action: action, Echo: "e-panic-new"}, false)

	assertSinglePanicReceipt(t, client, result, action, "new boom", "e-panic-new")
}

// shadow 的 new 对比执行（dryRun）panic 时只构造回执、不下发，避免与 legacy 双发。
func TestNewHandlerPanicDryRunDoesNotSend(t *testing.T) {
	const action = "test_new_handler_panic_dry_run"
	panicHandler(action, "dry-run boom")

	client := &mockClient{}
	result := callAPIFromDictNew(client, nil, nil, ActionMessage{Action: action, Echo: 1}, true)

	if msgs := client.sent(); len(msgs) != 0 {
		t.Fatalf("dry-run 下 SendMessage 调用次数 = %d, want 0", len(msgs))
	}
	r := decodeReceipt(t, []byte(result))
	if r.RetCode != 1500 || !strings.Contains(r.Message, action) {
		t.Errorf("dry-run 回执 = %s, want retcode 1500 且 message 含 action", result)
	}
}

// 经唯一分发入口调用：panic 被内层接住后，外层不会再补发，客户端只收到一份回执。
func TestCallAPIFromDictHandlerPanicSendsSingleReceipt(t *testing.T) {
	const action = "test_callapi_handler_panic"
	panicHandler(action, "entry boom")

	client := &mockClient{}
	result := CallAPIFromDict(client, nil, nil, ActionMessage{Action: action, Echo: "e-panic-entry"})

	assertSinglePanicReceipt(t, client, result, action, "entry boom", "e-panic-entry")
}

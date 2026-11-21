package callapi

import (
	"encoding/json"
	"errors"
	"sync"
	"testing"

	"github.com/tencent-connect/botgo/openapi"
)

// mockClient 记录 SendMessage 调用，用于断言分发层确实下发了回执（不再静默无响应）。
type mockClient struct {
	mu       sync.Mutex
	messages []map[string]interface{}
}

func (m *mockClient) SendMessage(message map[string]interface{}) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.messages = append(m.messages, message)
	return nil
}

func (m *mockClient) sent() []map[string]interface{} {
	m.mu.Lock()
	defer m.mu.Unlock()
	return append([]map[string]interface{}(nil), m.messages...)
}

// receipt 用于解析回执：data 用 RawMessage 以区分 null 与缺失。
type receipt struct {
	Data    json.RawMessage `json:"data"`
	Message string          `json:"message"`
	RetCode int             `json:"retcode"`
	Status  string          `json:"status"`
	Echo    interface{}     `json:"echo"`
}

func decodeReceipt(t *testing.T, raw []byte) receipt {
	t.Helper()
	var r receipt
	if err := json.Unmarshal(raw, &r); err != nil {
		t.Fatalf("回执不是合法 JSON: %v (%s)", err, raw)
	}
	return r
}

func assertFailureReceipt(t *testing.T, raw []byte, wantCode int, wantMsg string, wantEcho interface{}) {
	t.Helper()
	r := decodeReceipt(t, raw)
	if r.RetCode != wantCode {
		t.Errorf("retcode = %d, want %d (%s)", r.RetCode, wantCode, raw)
	}
	if r.Status != "failed" {
		t.Errorf("status = %q, want %q", r.Status, "failed")
	}
	if string(r.Data) != "null" {
		t.Errorf("data = %s, want null", r.Data)
	}
	if r.Message != wantMsg {
		t.Errorf("message = %q, want %q", r.Message, wantMsg)
	}
	if r.Echo != wantEcho {
		t.Errorf("echo = %#v, want %#v", r.Echo, wantEcho)
	}
}

func TestLegacyUnknownActionSendsReceipt(t *testing.T) {
	client := &mockClient{}
	result := callAPIFromDictLegacy(client, nil, nil, ActionMessage{Action: "no_such_action", Echo: 42})

	assertFailureReceipt(t, []byte(result), 1404, "未知 action: no_such_action", float64(42))
	msgs := client.sent()
	if len(msgs) != 1 {
		t.Fatalf("SendMessage 调用次数 = %d, want 1", len(msgs))
	}
	raw, err := MarshalNoEscape(msgs[0])
	if err != nil {
		t.Fatalf("下发回执无法序列化: %v", err)
	}
	assertFailureReceipt(t, raw, 1404, "未知 action: no_such_action", float64(42))
}

func TestLegacyHandlerErrorSendsReceipt(t *testing.T) {
	const action = "test_legacy_handler_error"
	RegisterHandler(action, func(Client, openapi.OpenAPI, openapi.OpenAPI, ActionMessage) (string, error) {
		return "", errors.New("boom")
	})

	client := &mockClient{}
	result := callAPIFromDictLegacy(client, nil, nil, ActionMessage{Action: action, Echo: "e1"})

	assertFailureReceipt(t, []byte(result), 1500, "boom", "e1")
	if msgs := client.sent(); len(msgs) != 1 {
		t.Fatalf("SendMessage 调用次数 = %d, want 1", len(msgs))
	}
}

func TestNewUnknownActionSendsReceipt(t *testing.T) {
	client := &mockClient{}
	result := callAPIFromDictNew(client, nil, nil, ActionMessage{Action: "no_such_action", Echo: 7}, false)

	assertFailureReceipt(t, []byte(result), 1404, "未知 action: no_such_action", float64(7))
	if msgs := client.sent(); len(msgs) != 1 {
		t.Fatalf("SendMessage 调用次数 = %d, want 1", len(msgs))
	}
}

// shadow 的 new 对比执行只构造回执、不发送，避免与 legacy 双发。
func TestNewUnknownActionDryRunDoesNotSend(t *testing.T) {
	client := &mockClient{}
	result := callAPIFromDictNew(client, nil, nil, ActionMessage{Action: "no_such_action", Echo: 7}, true)

	assertFailureReceipt(t, []byte(result), 1404, "未知 action: no_such_action", float64(7))
	if msgs := client.sent(); len(msgs) != 0 {
		t.Fatalf("dry-run 下 SendMessage 调用次数 = %d, want 0", len(msgs))
	}
}

func TestNewHandlerErrorSendsReceipt(t *testing.T) {
	const action = "test_new_handler_error"
	RegisterHandler(action, func(Client, openapi.OpenAPI, openapi.OpenAPI, ActionMessage) (string, error) {
		return "", errors.New("boom")
	})

	client := &mockClient{}
	result := callAPIFromDictNew(client, nil, nil, ActionMessage{Action: action, Echo: "e2"}, false)

	assertFailureReceipt(t, []byte(result), 1500, "boom", "e2")
	if msgs := client.sent(); len(msgs) != 1 {
		t.Fatalf("SendMessage 调用次数 = %d, want 1", len(msgs))
	}
}

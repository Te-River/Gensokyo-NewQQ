package wsclient

// 反向 WS 的解析失败分支回归网：ws.go recvMessage 在整包反序列化失败时必须回 1400 回执，
// 且 echo 从原始报文二次提取后逐字节保留（大整数不能被 float64 改写）。
// 这里走真实读循环 handleIncomingMessages，覆盖「服务端投递报文 → recvMessage → 回写回执」全链。

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/gorilla/websocket"
)

// wsReceipt 用 RawMessage 保存 data/echo，才能断言 echo 的原始字节。
type wsReceipt struct {
	Data    json.RawMessage `json:"data"`
	Message string          `json:"message"`
	RetCode int             `json:"retcode"`
	Status  string          `json:"status"`
	Echo    json.RawMessage `json:"echo"`
}

// newRecvPathClient 建一对本地 WS 连接，返回服务端 conn（测试用它投递 action 报文、读回执），
// 客户端 side 交给真实读循环消费。绕开 NewWebSocketClient 的 lifecycle/心跳，
// 保证回执通道里只有被测报文产生的帧。
func newRecvPathClient(t *testing.T) *websocket.Conn {
	t.Helper()

	up := websocket.Upgrader{}
	serverCh := make(chan *websocket.Conn, 1)
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		c, err := up.Upgrade(w, r, nil)
		if err != nil {
			return
		}
		// Upgrader 已 hijack 底层连接，handler 返回不会关闭它；此 conn 交给测试使用。
		serverCh <- c
	}))
	t.Cleanup(srv.Close)

	clientConn, _, err := websocket.DefaultDialer.Dial("ws"+strings.TrimPrefix(srv.URL, "http"), nil)
	if err != nil {
		t.Fatalf("连接本地 WS 失败: %v", err)
	}
	serverConn := <-serverCh

	client := &WebSocketClient{
		conn:         clientConn,
		sendFailures: []map[string]interface{}{},
		writeCh:      make(chan writeRequest, 16),
		closeCh:      make(chan struct{}),
	}
	go client.startWriter()
	_, cancel := context.WithCancel(context.Background())
	go client.handleIncomingMessages(cancel)

	t.Cleanup(func() { _ = client.Close() })
	t.Cleanup(func() { _ = serverConn.Close() })
	return serverConn
}

// readReceiptFrame 读一帧回执并解析；超时即失败（解析失败分支若静默，这里会卡住）。
func readReceiptFrame(t *testing.T, server *websocket.Conn) wsReceipt {
	t.Helper()
	if err := server.SetReadDeadline(time.Now().Add(3 * time.Second)); err != nil {
		t.Fatalf("设置读超时失败: %v", err)
	}
	_, raw, err := server.ReadMessage()
	if err != nil {
		t.Fatalf("未收到 1400 回执（解析失败被静默）: %v", err)
	}
	var r wsReceipt
	if err := json.Unmarshal(raw, &r); err != nil {
		t.Fatalf("回执不是合法 JSON: %v (%s)", err, raw)
	}
	return r
}

func assertWSUnparseableReceipt(t *testing.T, r wsReceipt, wantEcho string) {
	t.Helper()
	if r.RetCode != 1400 {
		t.Errorf("retcode = %d, want 1400", r.RetCode)
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
		t.Errorf("echo = %s, want %s", r.Echo, wantEcho)
	}
}

// TestRecvMessageUnparseableReplies1400Once 断言每被投递一份坏报文，恰好回一份 1400 回执。
func TestRecvMessageUnparseableReplies1400Once(t *testing.T) {
	cases := []struct {
		name     string
		payload  string
		wantEcho string
	}{
		{
			name:     "put_interaction post_type 为对象",
			payload:  `{"echo":"pi-1","action":"put_interaction","post_type":{}}`,
			wantEcho: `"pi-1"`,
		},
		{
			name:     "大整数 echo 不被 float64 改写",
			payload:  `{"echo":1234567890123456789,"action":"send_group_msg","post_type":0}`,
			wantEcho: `1234567890123456789`,
		},
		{
			name:     "无 echo 时仍回执一次且 echo 为 null",
			payload:  `{"action":"put_interaction","post_type":{}}`,
			wantEcho: `null`,
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			server := newRecvPathClient(t)

			if err := server.WriteMessage(websocket.TextMessage, []byte(tc.payload)); err != nil {
				t.Fatalf("投递报文失败: %v", err)
			}

			assertWSUnparseableReceipt(t, readReceiptFrame(t, server), tc.wantEcho)

			// 只回一份：再读一次应超时（本测试未启动心跳，回执通道里不会有其它帧）。
			if err := server.SetReadDeadline(time.Now().Add(300 * time.Millisecond)); err != nil {
				t.Fatalf("设置读超时失败: %v", err)
			}
			if _, extra, err := server.ReadMessage(); err == nil {
				t.Fatalf("解析失败回了不止一份回执，多余帧: %s", extra)
			}
		})
	}
}

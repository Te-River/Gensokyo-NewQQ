package server

// 正向 WS 的解析失败分支回归网：wsserver.go processWSMessage 在整包反序列化失败时必须
// 回 1400 回执。这里把真实 server 侧 conn 直接交给 processWSMessage，客户端 conn 读回执，
// 覆盖的正是分支所在的函数本体；wsHandler 的读循环只负责转调，未在本文件覆盖。

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/gorilla/websocket"
)

// serverWSReceipt 用 RawMessage 保存 data/echo，才能断言 echo 的原始字节。
type serverWSReceipt struct {
	Data    json.RawMessage `json:"data"`
	Message string          `json:"message"`
	RetCode int             `json:"retcode"`
	Status  string          `json:"status"`
	Echo    json.RawMessage `json:"echo"`
}

// newServerWSPair 建一对本地 WS 连接，返回 (服务端 conn, 客户端 conn)。
func newServerWSPair(t *testing.T) (serverConn, clientConn *websocket.Conn) {
	t.Helper()

	up := websocket.Upgrader{}
	serverCh := make(chan *websocket.Conn, 1)
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		c, err := up.Upgrade(w, r, nil)
		if err != nil {
			return
		}
		// Upgrader 已 hijack 底层连接，handler 返回不会关闭它。
		serverCh <- c
	}))
	t.Cleanup(srv.Close)

	clientConn, _, err := websocket.DefaultDialer.Dial("ws"+strings.TrimPrefix(srv.URL, "http"), nil)
	if err != nil {
		t.Fatalf("连接本地 WS 失败: %v", err)
	}
	serverConn = <-serverCh
	t.Cleanup(func() { _ = serverConn.Close() })
	t.Cleanup(func() { _ = clientConn.Close() })
	return serverConn, clientConn
}

// TestProcessWSMessageUnparseableReplies1400 断言解析失败时 processWSMessage 同步回一份
// 1400 / failed / data:null 回执，echo 从原始报文保真回显。
func TestProcessWSMessageUnparseableReplies1400(t *testing.T) {
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
			name:     "无 echo 时仍回执且 echo 为 null",
			payload:  `{"action":"put_interaction","post_type":{}}`,
			wantEcho: `null`,
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			serverConn, clientConn := newServerWSPair(t)
			client := &WebSocketServerClient{Conn: serverConn}

			processWSMessage(client, []byte(tc.payload))

			if err := clientConn.SetReadDeadline(time.Now().Add(3 * time.Second)); err != nil {
				t.Fatalf("设置读超时失败: %v", err)
			}
			_, raw, err := clientConn.ReadMessage()
			if err != nil {
				t.Fatalf("未收到 1400 回执（解析失败被静默）: %v", err)
			}

			var r serverWSReceipt
			if err := json.Unmarshal(raw, &r); err != nil {
				t.Fatalf("回执不是合法 JSON: %v (%s)", err, raw)
			}
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
			if string(r.Echo) != tc.wantEcho {
				t.Errorf("echo = %s, want %s", r.Echo, tc.wantEcho)
			}
		})
	}
}

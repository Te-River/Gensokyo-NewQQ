package wsclient

// 回归网：Close 与 SendMessage/Reconnect 的竞态。
// 修复前 Close 会关闭 writeCh，关闭后的 SendMessage（以及 Reconnect 里复用
// client 的 SendMessage）在已关闭 channel 上发送会 panic: send on closed channel。

import (
	"errors"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/gorilla/websocket"
)

// newTestWSClient 起一个本地 httptest WS 服务端，并返回已连接的 client。
func newTestWSClient(t *testing.T) *WebSocketClient {
	t.Helper()

	up := websocket.Upgrader{}
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		c, err := up.Upgrade(w, r, nil)
		if err != nil {
			return
		}
		defer c.Close()
		for {
			if _, _, err := c.ReadMessage(); err != nil {
				return
			}
		}
	}))
	t.Cleanup(srv.Close)

	wsURL := "ws" + strings.TrimPrefix(srv.URL, "http")
	client, err := NewWebSocketClient(wsURL, 1, nil, nil, 0)
	if err != nil {
		t.Fatalf("连接本地 WS 失败: %v", err)
	}
	// LIFO：先关 client，srv.Close 才不会等待未断开的连接
	t.Cleanup(func() { _ = client.Close() })
	return client
}

// TestCloseIdempotentAndSendAfterClose 重复 Close 不 panic；关闭后 SendMessage
// 返回哨兵错误而不是「send on closed channel」panic。
func TestCloseIdempotentAndSendAfterClose(t *testing.T) {
	client := newTestWSClient(t)

	if err := client.Close(); err != nil {
		t.Fatalf("首次 Close 失败: %v", err)
	}
	if err := client.Close(); err != nil {
		t.Fatalf("重复 Close 应幂等且返回 nil: %v", err)
	}

	if err := client.SendMessage(map[string]interface{}{"post_type": "meta_event"}); !errors.Is(err, errWebSocketClosed) {
		t.Fatalf("关闭后 SendMessage 应返回 errWebSocketClosed, got %v", err)
	}
}

// TestReconnectAfterCloseDoesNotPanic 关闭后 Reconnect 应在关闭检查处快速返回，
// 不再建连、不再发送元事件。
func TestReconnectAfterCloseDoesNotPanic(t *testing.T) {
	client := newTestWSClient(t)

	if err := client.Close(); err != nil {
		t.Fatalf("Close 失败: %v", err)
	}

	done := make(chan struct{})
	go func() {
		defer close(done)
		client.Reconnect()
	}()

	select {
	case <-done:
	case <-time.After(2 * time.Second):
		t.Fatal("关闭后 Reconnect 未及时返回")
	}
}

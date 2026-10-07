package Processor

// 本文件是 tester 对入站 parity（修复包 1）的独立复验网：
//   - new 入站 payload 与 legacy 上报 map 为同一实例（逐字节透传）；
//   - new 入站送达全部通道：反向 WS(Wsclient) / 正向 WS(WsServerClients) / HTTP POST(post_url)；
//   - 非 bot 的 <@OpenID> 经 idmap 转为 [CQ:at,qq=虚拟ID]。
//
// 文件名 zzzz 前缀：在 zzz_arch_bridge_test.go（已注入 config 单例）之后运行。

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"reflect"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/gorilla/websocket"
	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/internal/domain/message"
	"github.com/hoshinonyaruko/gensokyo/structs"
	"github.com/hoshinonyaruko/gensokyo/template"
	"github.com/hoshinonyaruko/gensokyo/wsclient"
)

// parityLoadConfig 注入 arch_mode / post_url / convert_other_at 后加载 config 单例。
func parityLoadConfig(t *testing.T, mode, postURL string, convertOtherAt bool) {
	t.Helper()
	yml := strings.Replace(template.ConfigTemplate, "arch_mode : new", "arch_mode : "+mode, 1)
	if postURL != "" {
		yml = strings.Replace(yml, `post_url: [""]`, `post_url: ["`+postURL+`"]`, 1)
	}
	if convertOtherAt {
		yml = strings.Replace(yml, "convert_other_at : false", "convert_other_at : true", 1)
	}
	path := filepath.Join(t.TempDir(), "config.yml")
	if err := os.WriteFile(path, []byte(yml), 0600); err != nil {
		t.Fatalf("写入测试配置失败: %v", err)
	}
	if _, err := config.LoadConfig(path, false); err != nil {
		t.Fatalf("加载测试配置失败: %v", err)
	}
}

// TestDispatchInboundPayloadSameInstance 钉死 parity：new 入站送达的 payload 就是
// legacy 构造的上报 map 本身（同一实例 → 逐字节一致，无字段漂移）。
func TestDispatchInboundPayloadSameInstance(t *testing.T) {
	parityLoadConfig(t, "new", "", false)
	p, f := newArchProcessors()
	SetInbound(p)

	payload := map[string]interface{}{
		"post_type":    "message",
		"message_type": "group",
		"group_id":     int64(1),
		"self_id":      int64(2),
		"message_id":   int64(3),
		"raw_message":  "hi",
		"time":         int64(5),
	}
	p.dispatchInbound(archGroupMsg(), payload, func() { t.Fatal("new 不应走 legacy 广播") })

	select {
	case <-f.ch:
	case <-time.After(2 * time.Second):
		t.Fatal("new 路径未发布")
	}
	f.mu.Lock()
	got := f.msgs[0]
	f.mu.Unlock()
	if reflect.ValueOf(got).Pointer() != reflect.ValueOf(payload).Pointer() {
		t.Fatal("new 入站 payload 应为 legacy 上报 map 的同一实例（逐字节透传）")
	}
}

// TestBroadcastInboundAllChannels 钉死通道覆盖：new 入站经 BroadcastInbound 送达
// 反向 WS(Wsclient) / 正向 WS(WsServerClients) / HTTP POST(post_url) 三条通道。
func TestBroadcastInboundAllChannels(t *testing.T) {
	postCh := make(chan map[string]interface{}, 1)
	postSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var m map[string]interface{}
		_ = json.NewDecoder(r.Body).Decode(&m)
		select {
		case postCh <- m:
		default:
		}
		w.WriteHeader(http.StatusOK)
	}))
	defer postSrv.Close()

	wsCh := make(chan map[string]interface{}, 8)
	up := websocket.Upgrader{}
	wsSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		c, err := up.Upgrade(w, r, nil)
		if err != nil {
			return
		}
		defer c.Close()
		for {
			_, data, err := c.ReadMessage()
			if err != nil {
				return
			}
			var m map[string]interface{}
			if json.Unmarshal(data, &m) == nil && m["post_type"] == "message" {
				select {
				case wsCh <- m:
				default:
				}
			}
		}
	}))
	defer wsSrv.Close()

	parityLoadConfig(t, "new", postSrv.URL, false)

	wsURL := "ws" + strings.TrimPrefix(wsSrv.URL, "http")
	wsc, err := wsclient.NewWebSocketClient(wsURL, 1, nil, nil, 0)
	if err != nil {
		t.Fatalf("连接本地 WS 失败: %v", err)
	}
	defer wsc.Close()

	fake := &fakeServerClient{ch: make(chan struct{}, 4)}
	p := &Processors{
		Wsclient:        []*wsclient.WebSocketClient{wsc},
		WsServerClients: []callapi.WebSocketServerClienter{fake},
		Settings:        &structs.Settings{},
	}

	payload := map[string]interface{}{
		"post_type":    "message",
		"message_type": "group",
		"group_id":     int64(7),
		"self_id":      int64(8),
		"message_id":   int64(9),
		"raw_message":  "parity",
		"time":         int64(10),
	}
	if err := p.BroadcastInbound(payload); err != nil {
		t.Fatalf("BroadcastInbound 失败: %v", err)
	}

	select {
	case <-fake.ch:
	case <-time.After(2 * time.Second):
		t.Fatal("正向 WS(WsServerClients) 未收到 payload")
	}
	select {
	case got := <-wsCh:
		if got["raw_message"] != "parity" {
			t.Fatalf("反向 WS payload 不一致: %#v", got)
		}
	case <-time.After(3 * time.Second):
		t.Fatal("反向 WS(Wsclient) 未收到 payload")
	}
	select {
	case got := <-postCh:
		if got["raw_message"] != "parity" {
			t.Fatalf("HTTP POST payload 不一致: %#v", got)
		}
	case <-time.After(3 * time.Second):
		t.Fatal("HTTP POST(post_url) 未收到 payload")
	}
}

// TestNormalizeInboundAtOtherOpenID 钉死 <@OpenID>→[CQ:at,qq=虚拟ID]：
// 非 bot 的 OpenID 经 idmap 反查虚拟 ID（convert_other_at=true）。
func TestNormalizeInboundAtOtherOpenID(t *testing.T) {
	parityLoadConfig(t, "new", "", true)
	p, _ := newArchProcessors()
	SetInbound(p)

	const realOpenID = "0123456789abcdef0123456789abcdef"
	virtual, err := idmap.StoreIDv2(realOpenID)
	if err != nil {
		t.Fatalf("StoreIDv2 失败: %v", err)
	}

	m := archGroupMsg()
	m.Content = "<@" + realOpenID + "> hi"
	ev, err := p.normalizeInbound(m)
	if err != nil {
		t.Fatalf("Normalize 失败: %v", err)
	}
	tp, ok := ev.Message.Parts[0].(message.TextPart)
	if !ok {
		t.Fatalf("Part 类型异常: %#v", ev.Message.Parts[0])
	}
	want := "[CQ:at,qq=" + strconv.FormatInt(virtual, 10) + "] hi"
	if tp.Text != want {
		t.Fatalf("非 bot <@OpenID> 应转虚拟 ID: got %q want %q", tp.Text, want)
	}
}

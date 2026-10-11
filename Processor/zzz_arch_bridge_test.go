package Processor

// 本文件是 tester 对入站接缝（B 包）的独立验证网：
//   - legacy：dispatchInbound 只走 legacyBroadcast，不触发 new 发布；
//   - shadow：legacyBroadcast 保留，并打印 "[arch] shadow inbound" 标记，不发布；
//   - new：不走 legacyBroadcast，经 BroadcastSender 发布到 forward-WS 客户端；
//   - B 语义：Normalize 产出的 Message 为单一 TextPart。
//
// 文件名 zzz 前缀：本包 processor_test.go 依赖 config 单例为 nil（GetMePrefix 默认），
// 而本文件经 LoadConfig 注入单例，故必须最后运行（Go 按文件名字母序执行）。

import (
	"bytes"
	"io"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/handlers"
	"github.com/hoshinonyaruko/gensokyo/internal/domain/message"
	"github.com/hoshinonyaruko/gensokyo/structs"
	"github.com/hoshinonyaruko/gensokyo/template"
	"github.com/tencent-connect/botgo/dto"
)

// archLoadConfig 以完整模板为基底注入指定 arch_mode 并加载 config 单例。
func archLoadConfig(t *testing.T, mode string) {
	t.Helper()
	yml := strings.Replace(template.ConfigTemplate, "arch_mode : new", "arch_mode : "+mode, 1)
	path := filepath.Join(t.TempDir(), "config.yml")
	if err := os.WriteFile(path, []byte(yml), 0600); err != nil {
		t.Fatalf("写入测试配置失败: %v", err)
	}
	if _, err := config.LoadConfig(path, false); err != nil {
		t.Fatalf("加载测试配置失败: %v", err)
	}
}

// fakeServerClient 记录 new 路径经 BroadcastSender 下发的 payload。
type fakeServerClient struct {
	mu   sync.Mutex
	msgs []map[string]interface{}
	ch   chan struct{}
}

func (f *fakeServerClient) SendMessage(m map[string]interface{}) error {
	f.mu.Lock()
	f.msgs = append(f.msgs, m)
	f.mu.Unlock()
	select {
	case f.ch <- struct{}{}:
	default:
	}
	return nil
}

func (f *fakeServerClient) Close() error { return nil }

func (f *fakeServerClient) count() int {
	f.mu.Lock()
	defer f.mu.Unlock()
	return len(f.msgs)
}

func newArchProcessors() (*Processors, *fakeServerClient) {
	f := &fakeServerClient{ch: make(chan struct{}, 4)}
	p := &Processors{WsServerClients: []callapi.WebSocketServerClienter{f}, Settings: &structs.Settings{}}
	return p, f
}

func archGroupMsg() *dto.WSGroupMessageData {
	return &dto.WSGroupMessageData{
		ID:      "m1",
		Content: "hi",
		GroupID: "g1",
		Author:  &dto.User{ID: "u1"},
	}
}

// captureStdout 捕获 fn 执行期间写入 os.Stdout 的内容（mylog 走 fmt.Println）。
func captureStdout(fn func()) string {
	old := os.Stdout
	r, w, err := os.Pipe()
	if err != nil {
		panic(err)
	}
	os.Stdout = w
	done := make(chan string, 1)
	go func() {
		var b bytes.Buffer
		_, _ = io.Copy(&b, r)
		done <- b.String()
	}()
	fn()
	_ = w.Close()
	os.Stdout = old
	return <-done
}

// TestDispatchInboundLegacy 验证 legacy 分支：只走 legacyBroadcast，不触发 new 发布。
func TestDispatchInboundLegacy(t *testing.T) {
	archLoadConfig(t, "legacy")
	p, f := newArchProcessors()
	SetInbound(p)

	called := 0
	p.dispatchInbound(archGroupMsg(), nil, func() { called++ })

	if called != 1 {
		t.Fatalf("legacy 应调用 legacyBroadcast 恰好一次: got %d", called)
	}
	time.Sleep(50 * time.Millisecond)
	if n := f.count(); n != 0 {
		t.Fatalf("legacy 不应走 new 发布: got %d 条", n)
	}
}

// TestDispatchInboundShadow 验证 shadow 分支：保留 legacy 广播 + 打印标记，不发布。
func TestDispatchInboundShadow(t *testing.T) {
	archLoadConfig(t, "shadow")
	p, f := newArchProcessors()
	SetInbound(p)

	called := 0
	out := captureStdout(func() {
		p.dispatchInbound(archGroupMsg(), nil, func() { called++ })
	})

	if called != 1 {
		t.Fatalf("shadow 应保留 legacyBroadcast: got %d", called)
	}
	if !strings.Contains(out, "[arch] shadow inbound") {
		t.Fatalf("shadow 应打印标记 [arch] shadow inbound: got %q", out)
	}
	if n := f.count(); n != 0 {
		t.Fatalf("shadow 入站不应发布: got %d 条", n)
	}
}

// TestDispatchInboundNew 验证 new 分支：不走 legacy 广播，把 legacy payload 逐字节
// 透传到 forward-WS 客户端（parity：字段集与 legacy 一致）。
func TestDispatchInboundNew(t *testing.T) {
	archLoadConfig(t, "new")
	p, f := newArchProcessors()
	SetInbound(p)

	// legacy 构造的上报 map（含 group_id/self_id/message_id/raw_message/sender/sub_type）。
	payload := map[string]interface{}{
		"post_type":    "message",
		"message_type": "group",
		"group_id":     int64(1),
		"self_id":      int64(2),
		"message_id":   int64(3),
		"raw_message":  "hi",
		"sender":       map[string]interface{}{"user_id": int64(4)},
		"sub_type":     "normal",
		"time":         int64(5),
	}

	called := 0
	p.dispatchInbound(archGroupMsg(), payload, func() { called++ })

	if called != 0 {
		t.Fatalf("new 不应走 legacy 广播: got %d", called)
	}
	select {
	case <-f.ch:
	case <-time.After(2 * time.Second):
		t.Fatalf("new 路径未发布到 forward-WS 客户端")
	}
	f.mu.Lock()
	got := f.msgs[0]
	f.mu.Unlock()
	// parity：legacy payload 的每个字段都必须原样送达。
	for k, want := range payload {
		if !reflect.DeepEqual(got[k], want) {
			t.Fatalf("new payload 字段 %q 未逐字节透传: got=%#v want=%#v", k, got[k], want)
		}
	}
}

// TestNormalizeInboundSingleTextPart 钉死 B 语义：Normalize 产出单一 TextPart，
// 且 <@OpenID> 经 CQ 转换（parity：与 legacy 入站一致）。
func TestNormalizeInboundSingleTextPart(t *testing.T) {
	archLoadConfig(t, "new")
	p, _ := newArchProcessors()
	SetInbound(p)

	ev, err := p.normalizeInbound(archGroupMsg())
	if err != nil {
		t.Fatalf("Normalize 失败: %v", err)
	}
	if len(ev.Message.Parts) != 1 {
		t.Fatalf("Message 应为单一 Part: got %d", len(ev.Message.Parts))
	}
	tp, ok := ev.Message.Parts[0].(message.TextPart)
	if !ok || tp.Text != "hi" {
		t.Fatalf("Part 应为 TextPart{Text:hi}: got %#v", ev.Message.Parts[0])
	}

	// @bot 的 <@OpenID> 应转为 [CQ:at,qq=自身ID]（复用 handlers 的 self-at 判定）。
	handlers.RememberSelfAtID("abc")
	atMsg := archGroupMsg()
	atMsg.Content = "<@abc> hi"
	ev2, err := p.normalizeInbound(atMsg)
	if err != nil {
		t.Fatalf("Normalize(@) 失败: %v", err)
	}
	tp2, ok := ev2.Message.Parts[0].(message.TextPart)
	if !ok || !strings.Contains(tp2.Text, "[CQ:at,qq=") {
		t.Fatalf("入站 <@OpenID> 应转为 [CQ:at,qq=...]: got %#v", ev2.Message.Parts[0])
	}
}

// TestNormalizeInboundUnsupportedFallsBack 验证未覆盖事件回退 legacy 广播。
func TestNormalizeInboundUnsupportedFallsBack(t *testing.T) {
	archLoadConfig(t, "new")
	p, _ := newArchProcessors()
	SetInbound(p)

	called := 0
	// *dto.WSInteractionData（inline search）不在 Normalizer 覆盖范围。
	p.dispatchInbound(&dto.WSInteractionData{}, nil, func() { called++ })
	if called != 1 {
		t.Fatalf("未覆盖事件应回退 legacyBroadcast: got %d", called)
	}
}

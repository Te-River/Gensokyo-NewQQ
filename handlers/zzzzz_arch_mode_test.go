package handlers

// 本文件是 tester 对 handlers 侧接缝（A/D 包）的独立验证网：
//   - handlers.ArchMode() 逐请求反映 config.GetArchMode()（legacy|shadow|new）；
//   - D 包 state 接缝：SetStateRepositories 注入后 getter 可读回，且仓储可用；
//   - A 包回执：new 路径回执为最小 {"status":"ok",...}，非 legacy ServerResponse 形态。
//
// 文件名 zzzzz 前缀：本包 zzzz_inbound_at_order_test.go 已注入 config 单例，
// 本文件在其后运行（Go 按文件名字母序执行）。

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"

	staterepo "github.com/hoshinonyaruko/gensokyo/adapter/state"
	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/internal/application/outbound"
	"github.com/hoshinonyaruko/gensokyo/internal/domain/identity"
	"github.com/hoshinonyaruko/gensokyo/template"
)

// archModeLoadConfig 以完整模板为基底注入指定 arch_mode 并加载 config 单例。
func archModeLoadConfig(t *testing.T, mode string) {
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

// TestHandlersArchModeReflectsConfig 验证 handlers.ArchMode() 逐请求反映配置。
func TestHandlersArchModeReflectsConfig(t *testing.T) {
	for _, mode := range []string{"legacy", "shadow", "new"} {
		archModeLoadConfig(t, mode)
		if got := ArchMode(); got != mode {
			t.Fatalf("ArchMode() 应反映 arch_mode=%q: got %q", mode, got)
		}
	}
}

// TestStateSeamInjectedAndReadable 验证 D 包接缝：注入后 getter 可读回且仓储可用。
func TestStateSeamInjectedAndReadable(t *testing.T) {
	seq := staterepo.NewEchoSequenceRepository()
	ctxRepo := staterepo.NewEchoContextRepository()
	SetStateRepositories(seq, ctxRepo)

	if SequenceRepository() == nil {
		t.Fatalf("SequenceRepository() 注入后应非 nil")
	}
	if MessageContextRepository() == nil {
		t.Fatalf("MessageContextRepository() 注入后应非 nil")
	}
	if SequenceRepository() != seq || MessageContextRepository() != ctxRepo {
		t.Fatalf("getter 应读回注入的同一实例")
	}

	// 仓储可用：Next 原子递增；Set/Get 往返。
	if _, err := SequenceRepository().Next(context.Background(), "k"); err != nil {
		t.Fatalf("SequenceRepository.Next 失败: %v", err)
	}
	if err := MessageContextRepository().Set(context.Background(), "owner", "key", "val", 0); err != nil {
		t.Fatalf("MessageContextRepository.Set 失败: %v", err)
	}
	got, err := MessageContextRepository().Get(context.Background(), "owner", "key")
	if err != nil || got != "val" {
		t.Fatalf("MessageContextRepository.Get 往返失败: got=%q err=%v", got, err)
	}
}

// fakeReceiptClient 记录 legacy 发送核心经 client.SendMessage 回发的回执。
type fakeReceiptClient struct {
	mu   sync.Mutex
	msgs []map[string]interface{}
}

func (f *fakeReceiptClient) SendMessage(m map[string]interface{}) error {
	f.mu.Lock()
	f.msgs = append(f.msgs, m)
	f.mu.Unlock()
	return nil
}

// TestNewOutboundReceiptLegacyShape 钉死 A 语义：new 路径回执复用 legacy 发送核心
// （SendResponse），含 echo/group_id/虚拟 message_id，并经 client.SendMessage 主动回发。
func TestNewOutboundReceiptLegacyShape(t *testing.T) {
	archModeLoadConfig(t, "new")
	f := &fakeReceiptClient{}
	target := identity.ResolvedTarget{
		Kind:  identity.TargetGroup,
		Group: &identity.ResolvedGroup{OpenID: "OPENID_GROUP", VirtualGroupID: "12345"},
	}
	msg := callapi.ActionMessage{Action: "send_group_msg", Params: callapi.ParamsContent{GroupID: "12345"}, Echo: "e1"}
	_ = sendOutboundReceipt(f, nil, nil, msg, target, "REAL_MSG_ID", nil)

	if len(f.msgs) != 1 {
		t.Fatalf("回执应经 client.SendMessage 主动回发: got %d", len(f.msgs))
	}
	m := f.msgs[0]
	if m["status"] != "ok" || m["retcode"].(float64) != 0 {
		t.Fatalf("回执 status/retcode 异常: %+v", m)
	}
	if m["echo"] != "e1" {
		t.Fatalf("回执应含 echo: %+v", m)
	}
	if _, ok := m["group_id"]; !ok {
		t.Fatalf("回执应含 group_id: %+v", m)
	}
	data, ok := m["data"].(map[string]interface{})
	if !ok {
		t.Fatalf("回执 data 异常: %+v", m)
	}
	if _, ok := data["message_id"]; !ok {
		t.Fatalf("回执应含 data.message_id: %+v", m)
	}
}

// fakeQQSender 记录 new 出站服务的发送次数。
type fakeQQSender struct {
	mu sync.Mutex
	n  int
}

func (f *fakeQQSender) Send(_ context.Context, _ identity.ResolvedTarget, _ outbound.QQMessage) (outbound.QQSendResult, error) {
	f.mu.Lock()
	f.n++
	f.mu.Unlock()
	return outbound.QQSendResult{MessageID: "1"}, nil
}

// TestShadowOutboundNoDoubleSend 钉死 C 语义：shadow 下 TryOutbound 只 diff、不发送，
// 且返回 handled=false（legacy 继续发送）→ 不双发。
func TestShadowOutboundNoDoubleSend(t *testing.T) {
	archModeLoadConfig(t, "shadow")
	f := &fakeQQSender{}
	SetOutboundService(outbound.NewService(f, outbound.RetryPolicy{}))
	defer SetOutboundService(nil)

	msg := callapi.ActionMessage{Params: callapi.ParamsContent{GroupID: "12345"}}
	handled, _ := TryOutbound(nil, nil, nil, msg, "hi", map[string][]string{}, identity.TargetGroup, false)

	if handled {
		t.Fatalf("shadow 应返回 handled=false（legacy 继续）")
	}
	if f.n != 0 {
		t.Fatalf("shadow 不应经 new 路径发送: got %d", f.n)
	}
}

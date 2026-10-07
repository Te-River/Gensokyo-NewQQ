package handlers_test

// 第二轮修复复验（tester 独立网，外部测试包）：
//   - C1：arch_mode=new + cq_parse_mode=legacy 下动作型 CQ 码在 TryOutbound 前执行、
//     正文清理、cqPreExecuted 守卫不重复执行；arch_mode=legacy 行为不变；
//     member 的 event_id / 跨群路由 parity；
//   - M4：shadow dry-run 为 per-request（ActionMessage.ShadowDryRun 经 ctx 透传），
//     并发下 legacy 发送不被跳过。

import (
	"context"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/adapter/qq"
	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/echo"
	"github.com/hoshinonyaruko/gensokyo/handlers"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/internal/application/outbound"
	"github.com/hoshinonyaruko/gensokyo/internal/domain/identity"
	"github.com/hoshinonyaruko/gensokyo/template"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

// round2E2EOpenAPI 记录群发送与禁言设置调用。
type round2E2EOpenAPI struct {
	openapi.OpenAPI
	groupCalls    int
	lastGroupID   string
	lastGroup     dto.APIMessage
	restrictCalls int
	setCalls      int
}

func (m *round2E2EOpenAPI) PostGroupMessage(_ context.Context, groupID string, msg dto.APIMessage) (*dto.GroupMessageResponse, error) {
	m.groupCalls++
	m.lastGroupID = groupID
	m.lastGroup = msg
	return &dto.GroupMessageResponse{Message: &dto.Message{ID: "round2-group-1"}}, nil
}

func (m *round2E2EOpenAPI) RestrictChatSetting(_ context.Context, _ string) (*dto.RestrictChatSetting, error) {
	m.restrictCalls++
	return &dto.RestrictChatSetting{}, nil
}

func (m *round2E2EOpenAPI) SetRestrictChatSetting(_ context.Context, _ string, _ *dto.RestrictChatSetting) error {
	m.setCalls++
	return nil
}

const (
	round2GroupA = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
	round2GroupB = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
	round2UserA  = "cccccccccccccccccccccccccccccccc"
)

func round2SetGroupMessage(groupID string) string {
	return "hello [CQ:set_group,action=whole_ban,group_id=" + groupID + ",enable=true]"
}

// TestC1NewModeSetGroupPreExecutedAndTextCleaned 钉死 C1：
// arch_mode=new + cq_parse_mode=legacy 下 [CQ:set_group] 在 TryOutbound 前执行一次，
// 正文清理后发送（cqPreExecuted 守卫不重复执行）。
func TestC1NewModeSetGroupPreExecutedAndTextCleaned(t *testing.T) {
	adapterLoadConfig(t, "new")
	mock := &round2E2EOpenAPI{}
	handlers.SetOutboundService(outbound.NewService(qq.NewQQSender(nil, mock), outbound.RetryPolicy{}))
	defer handlers.SetOutboundService(nil)
	client := &adapterTestClient{}

	msg := callapi.ActionMessage{
		Action: "send_group_msg",
		Params: callapi.ParamsContent{GroupID: round2GroupA, UserID: round2UserA, Message: round2SetGroupMessage(round2GroupA)},
		Echo:   "e-c1-new",
	}
	if _, err := handlers.HandleSendGroupMsg(client, nil, mock, msg); err != nil {
		t.Fatalf("HandleSendGroupMsg 返回错误: %v", err)
	}
	if mock.setCalls != 1 {
		t.Fatalf("set_group 应在 TryOutbound 前执行且只执行一次: got %d", mock.setCalls)
	}
	if mock.groupCalls != 1 {
		t.Fatalf("应发送 1 条群消息: got %d", mock.groupCalls)
	}
	gm, ok := mock.lastGroup.(*dto.MessageToCreate)
	if !ok {
		t.Fatalf("应发送 MessageToCreate: got %T", mock.lastGroup)
	}
	if strings.Contains(gm.Content, "[CQ:set_group") {
		t.Fatalf("正文应清理动作 CQ 码: got %q", gm.Content)
	}
	if strings.TrimSpace(gm.Content) != "hello" {
		t.Fatalf("正文应为 hello: got %q", gm.Content)
	}
}

// TestC1LegacyModeSetGroupUnchanged 钉死 C1 回归面：arch_mode=legacy 行为不变
// （legacy 段执行动作码一次并清理正文）。
func TestC1LegacyModeSetGroupUnchanged(t *testing.T) {
	adapterLoadConfig(t, "legacy")
	mock := &round2E2EOpenAPI{}
	client := &adapterTestClient{}

	msg := callapi.ActionMessage{
		Action: "send_group_msg",
		Params: callapi.ParamsContent{GroupID: round2GroupA, UserID: round2UserA, Message: round2SetGroupMessage(round2GroupA)},
		Echo:   "e-c1-legacy",
	}
	if _, err := handlers.HandleSendGroupMsg(client, nil, mock, msg); err != nil {
		t.Fatalf("HandleSendGroupMsg 返回错误: %v", err)
	}
	if mock.setCalls != 1 {
		t.Fatalf("legacy 应执行 set_group 一次: got %d", mock.setCalls)
	}
	if mock.groupCalls != 1 {
		t.Fatalf("legacy 应发送 1 条群消息: got %d", mock.groupCalls)
	}
	gm, ok := mock.lastGroup.(*dto.MessageToCreate)
	if !ok {
		t.Fatalf("应发送 MessageToCreate: got %T", mock.lastGroup)
	}
	if strings.TrimSpace(gm.Content) != "hello" {
		t.Fatalf("legacy 正文应为 hello: got %q", gm.Content)
	}
}

// TestC1NewModeMemberAddKeepsEventID 钉死 C1 的 member 语义：
// [CQ:member,type=add] 预执行取到的 event_id 必须随发送保留（legacy parity）。
func TestC1NewModeMemberAddKeepsEventID(t *testing.T) {
	adapterLoadConfig(t, "new")
	mock := &round2E2EOpenAPI{}
	handlers.SetOutboundService(outbound.NewService(qq.NewQQSender(nil, mock), outbound.RetryPolicy{}))
	defer handlers.SetOutboundService(nil)
	client := &adapterTestClient{}

	echo.AddEvnetIDv2(config.GetAppIDStr(), round2GroupA, "EV-ROUND2")
	msg := callapi.ActionMessage{
		Action: "send_group_msg",
		Params: callapi.ParamsContent{GroupID: round2GroupA, UserID: round2UserA,
			Message: "hello [CQ:member,type=add,group_id=" + round2GroupA + ",user_id=" + round2UserA + "]"},
		Echo: "e-c1-member",
	}
	if _, err := handlers.HandleSendGroupMsg(client, nil, mock, msg); err != nil {
		t.Fatalf("HandleSendGroupMsg 返回错误: %v", err)
	}
	if mock.groupCalls != 1 {
		t.Fatalf("应发送 1 条群消息: got %d", mock.groupCalls)
	}
	gm, ok := mock.lastGroup.(*dto.MessageToCreate)
	if !ok {
		t.Fatalf("应发送 MessageToCreate: got %T", mock.lastGroup)
	}
	if gm.EventID != "EV-ROUND2" {
		t.Fatalf("member add 的 event_id 应随发送保留（legacy parity）: got %q", gm.EventID)
	}
}

// TestC1LegacyModeMemberAddKeepsEventID event_id 保留的 legacy 基线
// （证明期望值来自 legacy 语义，而非测试臆造）。
func TestC1LegacyModeMemberAddKeepsEventID(t *testing.T) {
	adapterLoadConfig(t, "legacy")
	mock := &round2E2EOpenAPI{}
	client := &adapterTestClient{}

	echo.AddEvnetIDv2(config.GetAppIDStr(), round2GroupA, "EV-ROUND2")
	msg := callapi.ActionMessage{
		Action: "send_group_msg",
		Params: callapi.ParamsContent{GroupID: round2GroupA, UserID: round2UserA,
			Message: "hello [CQ:member,type=add,group_id=" + round2GroupA + ",user_id=" + round2UserA + "]"},
		Echo: "e-c1-member-legacy",
	}
	if _, err := handlers.HandleSendGroupMsg(client, nil, mock, msg); err != nil {
		t.Fatalf("HandleSendGroupMsg 返回错误: %v", err)
	}
	if mock.groupCalls != 1 {
		t.Fatalf("legacy 应发送 1 条群消息: got %d", mock.groupCalls)
	}
	gm, ok := mock.lastGroup.(*dto.MessageToCreate)
	if !ok {
		t.Fatalf("应发送 MessageToCreate: got %T", mock.lastGroup)
	}
	if gm.EventID != "EV-ROUND2" {
		t.Fatalf("legacy member add 的 event_id 基线异常: got %q", gm.EventID)
	}
}

// TestC1NewModeMemberCrossGroupRouting 钉死 C1 的跨群路由：
// [CQ:member] 的 group_id 应作为目标群（legacy send_group_msg.go:546-550）。
func TestC1NewModeMemberCrossGroupRouting(t *testing.T) {
	adapterLoadConfig(t, "new")
	mock := &round2E2EOpenAPI{}
	handlers.SetOutboundService(outbound.NewService(qq.NewQQSender(nil, mock), outbound.RetryPolicy{}))
	defer handlers.SetOutboundService(nil)
	client := &adapterTestClient{}

	msg := callapi.ActionMessage{
		Action: "send_group_msg",
		Params: callapi.ParamsContent{GroupID: round2GroupA, UserID: round2UserA,
			Message: "hello [CQ:member,type=remove,group_id=" + round2GroupB + ",user_id=" + round2UserA + "]"},
		Echo: "e-c1-route",
	}
	if _, err := handlers.HandleSendGroupMsg(client, nil, mock, msg); err != nil {
		t.Fatalf("HandleSendGroupMsg 返回错误: %v", err)
	}
	if mock.groupCalls != 1 {
		t.Fatalf("应发送 1 条群消息: got %d", mock.groupCalls)
	}
	if mock.lastGroupID != round2GroupB {
		t.Fatalf("member 的 group_id 应作为目标群（legacy 跨群路由）: got %q want %q", mock.lastGroupID, round2GroupB)
	}
}

// TestC1LegacyModeMemberCrossGroupRouting 跨群路由的 legacy 基线（证明期望值来自 legacy 语义）。
func TestC1LegacyModeMemberCrossGroupRouting(t *testing.T) {
	adapterLoadConfig(t, "legacy")
	mock := &round2E2EOpenAPI{}
	client := &adapterTestClient{}

	msg := callapi.ActionMessage{
		Action: "send_group_msg",
		Params: callapi.ParamsContent{GroupID: round2GroupA, UserID: round2UserA,
			Message: "hello [CQ:member,type=remove,group_id=" + round2GroupB + ",user_id=" + round2UserA + "]"},
		Echo: "e-c1-route-legacy",
	}
	if _, err := handlers.HandleSendGroupMsg(client, nil, mock, msg); err != nil {
		t.Fatalf("HandleSendGroupMsg 返回错误: %v", err)
	}
	if mock.groupCalls != 1 {
		t.Fatalf("legacy 应发送 1 条群消息: got %d", mock.groupCalls)
	}
	if mock.lastGroupID != round2GroupB {
		t.Fatalf("legacy 跨群路由目标异常: got %q want %q", mock.lastGroupID, round2GroupB)
	}
}

// TestC1NewModeMemberAddCrossGroupEventID 钉死 C1 的 add+跨群组合：
// 请求群=A、[CQ:member,type=add,group_id=B] 时，目标群为 B 且 event_id 取 B 的键。
func TestC1NewModeMemberAddCrossGroupEventID(t *testing.T) {
	adapterLoadConfig(t, "new")
	mock := &round2E2EOpenAPI{}
	handlers.SetOutboundService(outbound.NewService(qq.NewQQSender(nil, mock), outbound.RetryPolicy{}))
	defer handlers.SetOutboundService(nil)
	client := &adapterTestClient{}

	echo.AddEvnetIDv2(config.GetAppIDStr(), round2GroupB, "EV-CROSS-B")
	msg := callapi.ActionMessage{
		Action: "send_group_msg",
		Params: callapi.ParamsContent{GroupID: round2GroupA, UserID: round2UserA,
			Message: "hello [CQ:member,type=add,group_id=" + round2GroupB + ",user_id=" + round2UserA + "]"},
		Echo: "e-c1-member-cross",
	}
	if _, err := handlers.HandleSendGroupMsg(client, nil, mock, msg); err != nil {
		t.Fatalf("HandleSendGroupMsg 返回错误: %v", err)
	}
	if mock.groupCalls != 1 {
		t.Fatalf("应发送 1 条群消息: got %d", mock.groupCalls)
	}
	if mock.lastGroupID != round2GroupB {
		t.Fatalf("add 的 group_id 应作为目标群: got %q want %q", mock.lastGroupID, round2GroupB)
	}
	gm, ok := mock.lastGroup.(*dto.MessageToCreate)
	if !ok {
		t.Fatalf("应发送 MessageToCreate: got %T", mock.lastGroup)
	}
	if gm.EventID != "EV-CROSS-B" {
		t.Fatalf("add 的 event_id 应取 CQ 目标群的键: got %q want %q", gm.EventID, "EV-CROSS-B")
	}
}

// TestC1LegacyModeMemberAddCrossGroupEventID add+跨群的 legacy 基线。
func TestC1LegacyModeMemberAddCrossGroupEventID(t *testing.T) {
	adapterLoadConfig(t, "legacy")
	mock := &round2E2EOpenAPI{}
	client := &adapterTestClient{}

	echo.AddEvnetIDv2(config.GetAppIDStr(), round2GroupB, "EV-CROSS-B")
	msg := callapi.ActionMessage{
		Action: "send_group_msg",
		Params: callapi.ParamsContent{GroupID: round2GroupA, UserID: round2UserA,
			Message: "hello [CQ:member,type=add,group_id=" + round2GroupB + ",user_id=" + round2UserA + "]"},
		Echo: "e-c1-member-cross-legacy",
	}
	if _, err := handlers.HandleSendGroupMsg(client, nil, mock, msg); err != nil {
		t.Fatalf("HandleSendGroupMsg 返回错误: %v", err)
	}
	if mock.groupCalls != 1 {
		t.Fatalf("legacy 应发送 1 条群消息: got %d", mock.groupCalls)
	}
	if mock.lastGroupID != round2GroupB {
		t.Fatalf("legacy add 跨群目标异常: got %q want %q", mock.lastGroupID, round2GroupB)
	}
	gm, ok := mock.lastGroup.(*dto.MessageToCreate)
	if !ok {
		t.Fatalf("应发送 MessageToCreate: got %T", mock.lastGroup)
	}
	if gm.EventID != "EV-CROSS-B" {
		t.Fatalf("legacy add 跨群 event_id 基线异常: got %q want %q", gm.EventID, "EV-CROSS-B")
	}
}

// round2LoadConfigNoRetMsg 注入 arch_mode=new + no_ret_msg=true。
func round2LoadConfigNoRetMsg(t *testing.T) {
	t.Helper()
	yml := strings.Replace(template.ConfigTemplate, "no_ret_msg : false", "no_ret_msg : true", 1)
	path := filepath.Join(t.TempDir(), "config.yml")
	if err := os.WriteFile(path, []byte(yml), 0600); err != nil {
		t.Fatalf("写入测试配置失败: %v", err)
	}
	if _, err := config.LoadConfig(path, false); err != nil {
		t.Fatalf("加载测试配置失败: %v", err)
	}
}

// TestM3GroupLatestBotMsgRecordedWhenNoRetMsg 钉死 M3 边界：
// no_ret_msg=true 时回执早退，但"机器人最新消息"副作用仍须记录（delete_group_msg 依赖）。
func TestM3GroupLatestBotMsgRecordedWhenNoRetMsg(t *testing.T) {
	round2LoadConfigNoRetMsg(t)
	const realGroupOpenID = "dddddddddddddddddddddddddddddddd"
	virtual, err := idmap.StoreIDv2(realGroupOpenID)
	if err != nil {
		t.Fatalf("StoreIDv2 失败: %v", err)
	}
	const realID = "REAL-QQ-MSG-ID-NORET"
	handlers.SetOutboundService(outbound.NewService(&fixedIDSender{id: realID}, outbound.RetryPolicy{}))
	defer handlers.SetOutboundService(nil)
	client := &adapterTestClient{}

	msg := callapi.ActionMessage{Action: "send_group_msg", Params: callapi.ParamsContent{GroupID: strconv.FormatInt(virtual, 10)}, Echo: "e-m3"}
	handled, _ := handlers.TryOutbound(client, nil, nil, msg, "hi", map[string][]string{}, identity.TargetGroup, false)
	if !handled {
		t.Fatal("new 群聊应 handled")
	}
	if client.last != nil {
		t.Fatalf("no_ret_msg=true 不应回发回执: %#v", client.last)
	}
	got, err := idmap.GetLatestBotMsgID(realGroupOpenID)
	if err != nil || got != realID {
		t.Fatalf("no_ret_msg=true 仍应记录机器人最新消息: got=%q err=%v want=%q", got, err, realID)
	}
}

// round2CountingSender 记录 new 出站发送次数。
type round2CountingSender struct {
	mu sync.Mutex
	n  int
}

func (f *round2CountingSender) Send(_ context.Context, _ identity.ResolvedTarget, _ outbound.QQMessage) (outbound.QQSendResult, error) {
	f.mu.Lock()
	f.n++
	f.mu.Unlock()
	return outbound.QQSendResult{MessageID: "1"}, nil
}

// TestM4ShadowDryRunPerRequestConcurrent 钉死 M4：shadow 的 dry-run 为 per-request，
// 并发下 legacy 发送不被跳过（旧进程级标志会串读），且 dry-run 标记确实经 ctx 到达 handler。
func TestM4ShadowDryRunPerRequestConcurrent(t *testing.T) {
	adapterLoadConfig(t, "shadow")
	f := &round2CountingSender{}
	handlers.SetOutboundService(outbound.NewService(f, outbound.RetryPolicy{}))
	defer handlers.SetOutboundService(nil)

	const action = "tester_round2_shadow_probe"
	var legacyCalls, dryRunCalls atomic.Int64
	callapi.RegisterHandler(action, func(client callapi.Client, api, apiv2 openapi.OpenAPI, msg callapi.ActionMessage) (string, error) {
		if msg.ShadowDryRun {
			dryRunCalls.Add(1)
		} else {
			legacyCalls.Add(1)
		}
		if handled, ret := handlers.TryOutbound(client, api, apiv2, msg, "hi", map[string][]string{}, identity.TargetGroup, false); handled {
			return ret, nil
		}
		return "legacy-sent", nil
	})

	const goroutines, iterations = 4, 100
	bad := make([]string, goroutines)
	var wg sync.WaitGroup
	for g := 0; g < goroutines; g++ {
		wg.Add(1)
		go func(g int) {
			defer wg.Done()
			client := &adapterTestClient{}
			for i := 0; i < iterations; i++ {
				got := callapi.CallAPIFromDict(client, nil, nil, callapi.ActionMessage{Action: action, Params: callapi.ParamsContent{GroupID: "12345"}})
				if got != "legacy-sent" {
					bad[g] = got
					return
				}
			}
		}(g)
	}
	wg.Wait()
	for g, got := range bad {
		if got != "" {
			t.Fatalf("goroutine %d: shadow 并发下 legacy 发送被跳过: got %q", g, got)
		}
	}
	const want = int64(goroutines * iterations)
	if got := legacyCalls.Load(); got != want {
		t.Fatalf("legacy 执行次数异常（dry-run 标记串读?）: got %d want %d", got, want)
	}
	if got := dryRunCalls.Load(); got != want {
		t.Fatalf("dry-run 标记未经 ctx 到达 handler: got %d want %d", got, want)
	}
	if f.n != 0 {
		t.Fatalf("shadow dry-run 不应经 new 路径发送: got %d", f.n)
	}
}

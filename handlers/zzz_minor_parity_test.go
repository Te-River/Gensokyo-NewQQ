package handlers_test

// 收尾修复轮的行为分叉复验网（外部测试包 handlers_test）：
//   - 修复 1：CQ 跨群路由时 msg_id/event_id 查缓存与 SSM 补发必须用「原群」，
//     发送目标才是 CQ 的 group_id（legacy send_group_msg.go:283/290/299/322 为原群）。
//   - 修复 2：[CQ:member,type=remove] 显式清空 eventID 后，messageID=="2000" 分支
//     不得把缓存 event_id 回填（legacy 中 CQ 执行晚于 2000 分支，清空即最终值）。
// 每条分叉用例都配一条 legacy 基线（同样输入跑 arch_mode=legacy），
// 期望值取自 legacy 实跑结果而非测试臆造。

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/hoshinonyaruko/gensokyo/adapter/qq"
	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/echo"
	"github.com/hoshinonyaruko/gensokyo/handlers"
	"github.com/hoshinonyaruko/gensokyo/internal/application/outbound"
	"github.com/hoshinonyaruko/gensokyo/template"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

// parityGroupA/B：原群与 CQ 目标群（32 位 OpenID，同时用作请求 group_id，
// 使 legacy 的虚拟键与 new 的真实 OpenID 键落到同一条 echo 记录，便于对比）。
const (
	parityGroupA = "11111111111111111111111111111111"
	parityGroupB = "22222222222222222222222222222222"
	parityUserA  = "33333333333333333333333333333333"
	parityGroupC = "44444444444444444444444444444444"
	parityGroupD = "55555555555555555555555555555555"
	parityUserC  = "66666666666666666666666666666666"
	parityUserD  = "77777777777777777777777777777777"
)

// parityGroupCall 记录一次群发送（目标群 + 消息体）。
type parityGroupCall struct {
	groupID string
	msg     dto.APIMessage
}

// parityOpenAPI 按顺序记录群发送，SSM 补发与正文发送因此可分别断言。
type parityOpenAPI struct {
	openapi.OpenAPI
	calls []parityGroupCall
}

func (m *parityOpenAPI) PostGroupMessage(_ context.Context, groupID string, msg dto.APIMessage) (*dto.GroupMessageResponse, error) {
	m.calls = append(m.calls, parityGroupCall{groupID: groupID, msg: msg})
	return &dto.GroupMessageResponse{Message: &dto.Message{ID: "parity-group-1"}}, nil
}

// parityLoadConfig 注入 arch_mode + lazy_message_id=true（修复 1/2 的 2000 与 SSM 分支需要）。
func parityLoadConfig(t *testing.T, mode string) {
	t.Helper()
	yml := strings.Replace(template.ConfigTemplate, "arch_mode : new", "arch_mode : "+mode, 1)
	yml = strings.Replace(yml, "lazy_message_id : false", "lazy_message_id : true", 1)
	path := filepath.Join(t.TempDir(), "config.yml")
	if err := os.WriteFile(path, []byte(yml), 0600); err != nil {
		t.Fatalf("写入测试配置失败: %v", err)
	}
	if _, err := config.LoadConfig(path, false); err != nil {
		t.Fatalf("加载测试配置失败: %v", err)
	}
}

// paritySentEventID 取最后一次群发送的 event_id。
func paritySentEventID(t *testing.T, mock *parityOpenAPI) string {
	t.Helper()
	if len(mock.calls) == 0 {
		t.Fatalf("没有群发送记录")
	}
	gm, ok := mock.calls[len(mock.calls)-1].msg.(*dto.MessageToCreate)
	if !ok {
		t.Fatalf("应发送 MessageToCreate: got %T", mock.calls[len(mock.calls)-1].msg)
	}
	return gm.EventID
}

// parityPushStack 把一条待补发消息压入全局 SSM 栈（属原群），并注册清理。
func parityPushStack(t *testing.T, groupID, content string) {
	t.Helper()
	correlationID := "parity-ssm-" + groupID
	echo.PushGlobalStack(echo.MessageGroupPair{
		Group:         groupID,
		GroupMessage:  &dto.MessageToCreate{Content: content},
		CorrelationID: correlationID,
	})
	t.Cleanup(func() {
		for _, count := range []int{200} {
			pairs := echo.PopGlobalStackMulti(count)
			for i := len(pairs) - 1; i >= 0; i-- {
				if pairs[i].CorrelationID == correlationID {
					echo.RemoveFromGlobalStack(i)
				}
			}
		}
	})
}

// ---- 修复 1：跨群 CQ 时 msg_id/event_id 查缓存用「原群」 ----

// TestParity1NewCrossGroupEventIDLookupUsesOrigGroup 钉死修复 1（event_id 查缓存）：
// 请求群=A（缓存了 event_id）、CQ 目标群=B（B 无缓存）、add 未命中、
// messageID=="2000" 触发回填 → 回填必须查原群 A，拿到 A 的 event_id。
// 若查目标群 B 则回填为空（分叉）。
func TestParity1NewCrossGroupEventIDLookupUsesOrigGroup(t *testing.T) {
	parityLoadConfig(t, "new")
	mock := &parityOpenAPI{}
	handlers.SetOutboundService(outbound.NewService(qq.NewQQSender(nil, mock), outbound.RetryPolicy{}))
	defer handlers.SetOutboundService(nil)
	client := &adapterTestClient{}

	echo.AddEvnetIDv2(config.GetAppIDStr(), parityGroupA, "EV-ORIG")
	echo.AddLazyMessageIdv2(parityGroupA, parityUserA, "2000", time.Now())
	msg := callapi.ActionMessage{
		Action: "send_group_msg",
		Params: callapi.ParamsContent{GroupID: parityGroupA, UserID: parityUserA,
			Message: "hello [CQ:member,type=add,group_id=" + parityGroupB + ",user_id=" + parityUserA + "]"},
		Echo: "parity-1-new",
	}
	if _, err := handlers.HandleSendGroupMsg(client, nil, mock, msg); err != nil {
		t.Fatalf("HandleSendGroupMsg 返回错误: %v", err)
	}
	if len(mock.calls) != 1 {
		t.Fatalf("应发送 1 条群消息: got %d", len(mock.calls))
	}
	if got := mock.calls[0].groupID; got != parityGroupB {
		t.Fatalf("发送目标应为 CQ 目标群: got %q want %q", got, parityGroupB)
	}
	if got := paritySentEventID(t, mock); got != "EV-ORIG" {
		t.Fatalf("event_id 回填应查原群（legacy 语义）: got %q want %q", got, "EV-ORIG")
	}
}

// TestParity1LegacyCrossGroupEventIDLookupUsesOrigGroup 修复 1 的 legacy 基线
// （证明 EV-ORIG 来自 legacy 语义，而非测试臆造）。
func TestParity1LegacyCrossGroupEventIDLookupUsesOrigGroup(t *testing.T) {
	parityLoadConfig(t, "legacy")
	mock := &parityOpenAPI{}
	client := &adapterTestClient{}

	echo.AddEvnetIDv2(config.GetAppIDStr(), parityGroupA, "EV-ORIG")
	echo.AddLazyMessageIdv2(parityGroupA, parityUserA, "2000", time.Now())
	msg := callapi.ActionMessage{
		Action: "send_group_msg",
		Params: callapi.ParamsContent{GroupID: parityGroupA, UserID: parityUserA,
			Message: "hello [CQ:member,type=add,group_id=" + parityGroupB + ",user_id=" + parityUserA + "]"},
		Echo: "parity-1-legacy",
	}
	if _, err := handlers.HandleSendGroupMsg(client, nil, mock, msg); err != nil {
		t.Fatalf("HandleSendGroupMsg 返回错误: %v", err)
	}
	if got := mock.calls[len(mock.calls)-1].groupID; got != parityGroupB {
		t.Fatalf("legacy 跨群路由目标异常: got %q want %q", got, parityGroupB)
	}
	if got := paritySentEventID(t, mock); got != "EV-ORIG" {
		t.Fatalf("legacy event_id 回填基线异常: got %q want %q", got, "EV-ORIG")
	}
}

// TestParity1NewCrossGroupSSMUsesOrigGroup 钉死修复 1（SSM 补发）：
// 原群 A 的栈内失败消息应在跨群路由（目标群 B）时仍补发到 A
// （legacy send_group_msg.go:283 用 message.Params.GroupID）。
func TestParity1NewCrossGroupSSMUsesOrigGroup(t *testing.T) {
	parityLoadConfig(t, "new")
	mock := &parityOpenAPI{}
	handlers.SetOutboundService(outbound.NewService(qq.NewQQSender(nil, mock), outbound.RetryPolicy{}))
	defer handlers.SetOutboundService(nil)
	client := &adapterTestClient{}

	echo.AddLazyMessageIdv2(parityGroupA, parityUserA, "MSG-ORIG", time.Now())
	parityPushStack(t, parityGroupA, "queued-in-A")
	msg := callapi.ActionMessage{
		Action: "send_group_msg",
		Params: callapi.ParamsContent{GroupID: parityGroupA, UserID: parityUserA,
			Message: "hello [CQ:member,type=remove,group_id=" + parityGroupB + ",user_id=" + parityUserA + "]"},
		Echo: "parity-1-ssm",
	}
	if _, err := handlers.HandleSendGroupMsg(client, nil, mock, msg); err != nil {
		t.Fatalf("HandleSendGroupMsg 返回错误: %v", err)
	}
	if len(mock.calls) != 2 {
		t.Fatalf("应补发 1 条 + 正文 1 条: got %d", len(mock.calls))
	}
	if mock.calls[0].groupID != parityGroupA {
		t.Fatalf("SSM 补发应落在原群: got %q want %q", mock.calls[0].groupID, parityGroupA)
	}
	if mock.calls[1].groupID != parityGroupB {
		t.Fatalf("正文应发往 CQ 目标群: got %q want %q", mock.calls[1].groupID, parityGroupB)
	}
}

// TestParity1LegacyCrossGroupSSMUsesOrigGroup SSM 补发群的 legacy 基线。
func TestParity1LegacyCrossGroupSSMUsesOrigGroup(t *testing.T) {
	parityLoadConfig(t, "legacy")
	mock := &parityOpenAPI{}
	client := &adapterTestClient{}

	echo.AddLazyMessageIdv2(parityGroupA, parityUserA, "MSG-ORIG", time.Now())
	parityPushStack(t, parityGroupA, "queued-in-A")
	msg := callapi.ActionMessage{
		Action: "send_group_msg",
		Params: callapi.ParamsContent{GroupID: parityGroupA, UserID: parityUserA,
			Message: "hello [CQ:member,type=remove,group_id=" + parityGroupB + ",user_id=" + parityUserA + "]"},
		Echo: "parity-1-ssm-legacy",
	}
	if _, err := handlers.HandleSendGroupMsg(client, nil, mock, msg); err != nil {
		t.Fatalf("HandleSendGroupMsg 返回错误: %v", err)
	}
	if len(mock.calls) != 2 {
		t.Fatalf("legacy 应补发 1 条 + 正文 1 条: got %d", len(mock.calls))
	}
	if mock.calls[0].groupID != parityGroupA {
		t.Fatalf("legacy SSM 补发群基线异常: got %q want %q", mock.calls[0].groupID, parityGroupA)
	}
	if mock.calls[1].groupID != parityGroupB {
		t.Fatalf("legacy 正文目标群基线异常: got %q want %q", mock.calls[1].groupID, parityGroupB)
	}
}

// ---- 修复 2：[CQ:member,type=remove] 显式清空 eventID ----

// TestParity2NewMemberRemoveNoBackfill 钉死修复 2：
// remove 显式清空 eventID + messageID=="2000" + 群内有缓存 event_id
// → 最终 event_id 必须为空（不得回填缓存）。
func TestParity2NewMemberRemoveNoBackfill(t *testing.T) {
	parityLoadConfig(t, "new")
	mock := &parityOpenAPI{}
	handlers.SetOutboundService(outbound.NewService(qq.NewQQSender(nil, mock), outbound.RetryPolicy{}))
	defer handlers.SetOutboundService(nil)
	client := &adapterTestClient{}

	echo.AddEvnetIDv2(config.GetAppIDStr(), parityGroupC, "EV-CACHED-C")
	echo.AddLazyMessageIdv2(parityGroupC, parityUserC, "2000", time.Now())
	msg := callapi.ActionMessage{
		Action: "send_group_msg",
		Params: callapi.ParamsContent{GroupID: parityGroupC, UserID: parityUserC,
			Message: "bye [CQ:member,type=remove,user_id=" + parityUserC + "]"},
		Echo: "parity-2-remove",
	}
	if _, err := handlers.HandleSendGroupMsg(client, nil, mock, msg); err != nil {
		t.Fatalf("HandleSendGroupMsg 返回错误: %v", err)
	}
	if got := paritySentEventID(t, mock); got != "" {
		t.Fatalf("remove 显式清空后不得回填缓存 event_id: got %q", got)
	}
}

// TestParity2LegacyMemberRemoveNoBackfill 修复 2 的 legacy 基线（最终 event_id 为空）。
func TestParity2LegacyMemberRemoveNoBackfill(t *testing.T) {
	parityLoadConfig(t, "legacy")
	mock := &parityOpenAPI{}
	client := &adapterTestClient{}

	echo.AddEvnetIDv2(config.GetAppIDStr(), parityGroupC, "EV-CACHED-C")
	echo.AddLazyMessageIdv2(parityGroupC, parityUserC, "2000", time.Now())
	msg := callapi.ActionMessage{
		Action: "send_group_msg",
		Params: callapi.ParamsContent{GroupID: parityGroupC, UserID: parityUserC,
			Message: "bye [CQ:member,type=remove,user_id=" + parityUserC + "]"},
		Echo: "parity-2-remove-legacy",
	}
	if _, err := handlers.HandleSendGroupMsg(client, nil, mock, msg); err != nil {
		t.Fatalf("HandleSendGroupMsg 返回错误: %v", err)
	}
	if got := paritySentEventID(t, mock); got != "" {
		t.Fatalf("legacy remove 最终 event_id 基线异常: got %q", got)
	}
}

// TestParity2NewMemberAddKeepsInjectedEventID 回归面：add 命中缓存时注入值保留
// （不因 hasEventID 改造而丢失）。
func TestParity2NewMemberAddKeepsInjectedEventID(t *testing.T) {
	parityLoadConfig(t, "new")
	mock := &parityOpenAPI{}
	handlers.SetOutboundService(outbound.NewService(qq.NewQQSender(nil, mock), outbound.RetryPolicy{}))
	defer handlers.SetOutboundService(nil)
	client := &adapterTestClient{}

	echo.AddEvnetIDv2(config.GetAppIDStr(), parityGroupD, "EV-ADD-HIT")
	echo.AddLazyMessageIdv2(parityGroupD, parityUserD, "2000", time.Now())
	msg := callapi.ActionMessage{
		Action: "send_group_msg",
		Params: callapi.ParamsContent{GroupID: parityGroupD, UserID: parityUserD,
			Message: "hi [CQ:member,type=add,user_id=" + parityUserD + "]"},
		Echo: "parity-2-add",
	}
	if _, err := handlers.HandleSendGroupMsg(client, nil, mock, msg); err != nil {
		t.Fatalf("HandleSendGroupMsg 返回错误: %v", err)
	}
	if got := paritySentEventID(t, mock); got != "EV-ADD-HIT" {
		t.Fatalf("add 命中的 event_id 应保留: got %q want %q", got, "EV-ADD-HIT")
	}
}

// TestParity2NewNoCQKeepsBackfill 回归面：无动作型 CQ 码时
// messageID=="2000" 的缓存 event_id 回填行为不变。
func TestParity2NewNoCQKeepsBackfill(t *testing.T) {
	parityLoadConfig(t, "new")
	mock := &parityOpenAPI{}
	handlers.SetOutboundService(outbound.NewService(qq.NewQQSender(nil, mock), outbound.RetryPolicy{}))
	defer handlers.SetOutboundService(nil)
	client := &adapterTestClient{}

	echo.AddEvnetIDv2(config.GetAppIDStr(), parityGroupC, "EV-CACHED-C")
	echo.AddLazyMessageIdv2(parityGroupC, parityUserC, "2000", time.Now())
	msg := callapi.ActionMessage{
		Action: "send_group_msg",
		Params: callapi.ParamsContent{GroupID: parityGroupC, UserID: parityUserC, Message: "hello"},
		Echo:   "parity-2-nocq",
	}
	if _, err := handlers.HandleSendGroupMsg(client, nil, mock, msg); err != nil {
		t.Fatalf("HandleSendGroupMsg 返回错误: %v", err)
	}
	if got := paritySentEventID(t, mock); got != "EV-CACHED-C" {
		t.Fatalf("无 CQ 时应照常回填缓存 event_id: got %q want %q", got, "EV-CACHED-C")
	}
}

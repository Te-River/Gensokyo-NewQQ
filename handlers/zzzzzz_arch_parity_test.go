package handlers

// 本文件是 tester 对出站 parity（修复包 2/3）的独立复验网：
//   - shadow 经 CallAPIFromDict 的 dry-run 分支：只 diff、不发送；
//   - active/active_type/active_sub_type/input_notify/stream 随 Parts 往返保留；
//   - 22009 入 SSM 补发栈（非 22009 不入栈）。
//
// 文件名 zzzzzz 前缀：在 zzzzz_arch_mode_test.go（已注入 config 单例）之后运行。
// 经真实 adapter/qq.QQSender 的私聊语义测试见 zzzzzzz_arch_adapter_test.go
// （外部测试包，避免 handlers→adapter/qq→handlers 的测试导入环）。

import (
	"errors"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/echo"
	"github.com/hoshinonyaruko/gensokyo/internal/application/outbound"
	"github.com/hoshinonyaruko/gensokyo/internal/domain/identity"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

// TestShadowDispatchDryRunNoDoubleSend 钉死 shadow 的 dry-run 分支：
// CallAPIFromDict 先跑 legacy（发送），再以 dry-run 跑 new（只 diff）→ 不双发。
func TestShadowDispatchDryRunNoDoubleSend(t *testing.T) {
	archModeLoadConfig(t, "shadow")
	f := &fakeQQSender{}
	SetOutboundService(outbound.NewService(f, outbound.RetryPolicy{}))
	defer SetOutboundService(nil)

	const action = "tester_shadow_probe"
	callapi.RegisterHandler(action, func(client callapi.Client, api, apiv2 openapi.OpenAPI, msg callapi.ActionMessage) (string, error) {
		if handled, ret := TryOutbound(client, api, apiv2, msg, "hi", map[string][]string{}, identity.TargetGroup, false); handled {
			return ret, nil
		}
		return "legacy-sent", nil
	})

	client := &fakeReceiptClient{}
	got := callapi.CallAPIFromDict(client, nil, nil, callapi.ActionMessage{Action: action, Params: callapi.ParamsContent{GroupID: "12345"}})
	if got != "legacy-sent" {
		t.Fatalf("shadow 应返回 legacy 结果: got %q", got)
	}
	if f.n != 0 {
		t.Fatalf("shadow dry-run 不应经 new 路径发送: got %d", f.n)
	}
}

// TestOutboundControlKeysRoundTrip 钉死控制 key 保留：
// active/active_type/active_sub_type/input_notify/stream 必须随 Parts 往返。
func TestOutboundControlKeysRoundTrip(t *testing.T) {
	target := identity.ResolvedTarget{Kind: identity.TargetGroup, Group: &identity.ResolvedGroup{OpenID: "OPENID_GROUP"}}
	found := map[string][]string{
		"active":          {""},
		"active_type":     {"push"},
		"active_sub_type": {"1"},
		"input_notify":    {`{"type":"1","second":"30"}`},
		"stream":          {`{"type":"start","qq":"42"}`},
	}
	cmd := BuildOutboundCommand("hi", found, target, outbound.DeliveryPolicy{Mode: 1})
	_, rt := PartsToFoundItems(cmd.Message.Parts)
	for k, want := range found {
		got, ok := rt[k]
		if !ok || len(got) != len(want) || got[0] != want[0] {
			t.Fatalf("控制 key %q 未随 Parts 往返保留: got=%v want=%v", k, got, want)
		}
	}
}

// TestPushSSMOn22009Enqueues 钉死 SSM 补发：22009 入栈，其他错误码不入栈。
func TestPushSSMOn22009Enqueues(t *testing.T) {
	gm := &dto.MessageToCreate{Content: "ssm-probe"}
	pushSSMOn22009(errors.New(`PostGroupMessage failed: {"code":22009,"message":"频控"}`), "tester-ssm-group", gm)
	found := false
	for _, it := range echo.PopGlobalStackMulti(1000) {
		if it.Group == "tester-ssm-group" && it.GroupMessage == gm {
			found = true
		}
	}
	if !found {
		t.Fatal("22009 应入 SSM 补发栈")
	}

	pushSSMOn22009(errors.New(`{"code":100}`), "tester-ssm-group-2", gm)
	for _, it := range echo.PopGlobalStackMulti(1000) {
		if it.Group == "tester-ssm-group-2" {
			t.Fatal("非 22009 不应入 SSM 栈")
		}
	}
}

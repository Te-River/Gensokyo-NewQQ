package handlers

// 第二轮修复复验（tester 独立网）：
//   - M2：postGroupMessageWithEventIDFallback / postC2CMessageWithEventIDFallback
//     在 40034025（event_id 失效）时清空 EventID 重发一次，其他错误码不重发；
//   - M1/P3：resolveGroupMessageContext / resolvePrivateMessageContext
//     以虚拟 ID 查 lazy 池、echo 兜底、无 user 上下文清空 msg_id。
//
// 文件名 zzzzzzzz 前缀：在既有 config 注入测试（zzzzz/zzzzzz）之后运行。

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/echo"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/template"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

// round2FallbackAPI 首次调用返回指定 QQ 错误码，之后成功。
type round2FallbackAPI struct {
	openapi.OpenAPI
	failCode   int
	groupCalls int
	c2cCalls   int
	lastGroup  *dto.MessageToCreate
	lastC2C    *dto.MessageToCreate
}

func (m *round2FallbackAPI) PostGroupMessage(_ context.Context, _ string, msg dto.APIMessage) (*dto.GroupMessageResponse, error) {
	m.groupCalls++
	if gm, ok := msg.(*dto.MessageToCreate); ok {
		m.lastGroup = gm
	}
	if m.failCode != 0 && m.groupCalls == 1 {
		return nil, errors.New(`PostGroupMessage failed: {"code":` + strconv.Itoa(m.failCode) + `,"message":"probe"}`)
	}
	return &dto.GroupMessageResponse{Message: &dto.Message{ID: "g-ok"}}, nil
}

func (m *round2FallbackAPI) PostC2CMessage(_ context.Context, _ string, msg dto.APIMessage) (*dto.C2CMessageResponse, error) {
	m.c2cCalls++
	if gm, ok := msg.(*dto.MessageToCreate); ok {
		m.lastC2C = gm
	}
	if m.failCode != 0 && m.c2cCalls == 1 {
		return nil, errors.New(`PostC2CMessage failed: {"code":` + strconv.Itoa(m.failCode) + `,"message":"probe"}`)
	}
	return &dto.C2CMessageResponse{Message: &dto.Message{ID: "c-ok"}}, nil
}

// TestM2GroupEventIDFallbackRetriesOnce 钉死 M2：40034025 清空 EventID 重发一次。
func TestM2GroupEventIDFallbackRetriesOnce(t *testing.T) {
	mock := &round2FallbackAPI{failCode: 40034025}
	gm := &dto.MessageToCreate{Content: "hi", EventID: "EV-OLD"}
	resp, err := postGroupMessageWithEventIDFallback(context.Background(), mock, "g-open", gm)
	if err != nil {
		t.Fatalf("重发后应成功: %v", err)
	}
	if mock.groupCalls != 2 {
		t.Fatalf("40034025 应清空 EventID 重发一次: got %d 次", mock.groupCalls)
	}
	if gm.EventID != "" {
		t.Fatalf("重发前应清空 EventID: got %q", gm.EventID)
	}
	if resp == nil || resp.Message == nil || resp.Message.ID != "g-ok" {
		t.Fatalf("重发响应异常: %#v", resp)
	}
}

// TestM2GroupEventIDFallbackOtherErrorNoRetry 钉死 M2 边界：非 40034025 不重发、不清空。
func TestM2GroupEventIDFallbackOtherErrorNoRetry(t *testing.T) {
	mock := &round2FallbackAPI{failCode: 100}
	gm := &dto.MessageToCreate{Content: "hi", EventID: "EV-OLD"}
	_, err := postGroupMessageWithEventIDFallback(context.Background(), mock, "g-open", gm)
	if err == nil {
		t.Fatal("非 40034025 应返回错误")
	}
	if mock.groupCalls != 1 {
		t.Fatalf("非 40034025 不应重发: got %d 次", mock.groupCalls)
	}
	if gm.EventID != "EV-OLD" {
		t.Fatalf("非 40034025 不应清空 EventID: got %q", gm.EventID)
	}
}

// TestM2C2CEventIDFallbackRetriesOnce 钉死 M2 私聊分支：40034025 清空 EventID 重发一次。
func TestM2C2CEventIDFallbackRetriesOnce(t *testing.T) {
	mock := &round2FallbackAPI{failCode: 40034025}
	gm := &dto.MessageToCreate{Content: "hi", EventID: "EV-OLD"}
	resp, err := postC2CMessageWithEventIDFallback(context.Background(), mock, "u-open", gm)
	if err != nil {
		t.Fatalf("重发后应成功: %v", err)
	}
	if mock.c2cCalls != 2 {
		t.Fatalf("40034025 应清空 EventID 重发一次: got %d 次", mock.c2cCalls)
	}
	if gm.EventID != "" {
		t.Fatalf("重发前应清空 EventID: got %q", gm.EventID)
	}
	if resp == nil || resp.Message == nil || resp.Message.ID != "c-ok" {
		t.Fatalf("重发响应异常: %#v", resp)
	}
}

// TestM1ResolveGroupContextEchoFallbackWithUser 钉死 M1/P3：echo 串兜底 GetMsgIDByKey。
func TestM1ResolveGroupContextEchoFallbackWithUser(t *testing.T) {
	archModeLoadConfig(t, "new")
	appID := config.GetAppIDStr()
	echoKey := appID + "_e-round2"
	echo.AddMsgIDv3(appID, "e-round2", "MSG-FROM-ECHO")

	ctx := withOutboundMsgCtx(context.Background(), outboundMsgCtx{
		virtualGroupID: "12345",
		virtualUserID:  "42",
		echo:           echoKey,
	})
	got, _ := resolveGroupMessageContext(ctx, "12345", map[string][]string{})
	if got != "MSG-FROM-ECHO" {
		t.Fatalf("echo 兜底应命中 GetMsgIDByKey: got %q", got)
	}
}

// TestM1ResolveGroupContextClearsMsgIDWithoutUser 钉死 M1/P3：无 user 上下文清空 msg_id。
func TestM1ResolveGroupContextClearsMsgIDWithoutUser(t *testing.T) {
	archModeLoadConfig(t, "new")
	appID := config.GetAppIDStr()
	virtual, err := idmap.StoreIDv2("12345")
	if err != nil {
		t.Fatalf("StoreIDv2 失败: %v", err)
	}
	echo.AddMsgID(appID, virtual, "CACHED-GROUP-MSG")

	ctx := withOutboundMsgCtx(context.Background(), outboundMsgCtx{
		virtualGroupID: strconv.FormatInt(virtual, 10),
	})
	got, _ := resolveGroupMessageContext(ctx, "12345", map[string][]string{})
	if got != "" {
		t.Fatalf("无 user 上下文应清空缓存 msg_id: got %q", got)
	}
}

// round2LoadConfigLazy 注入 arch_mode=new + lazy_message_id=true。
func round2LoadConfigLazy(t *testing.T) {
	t.Helper()
	yml := strings.Replace(template.ConfigTemplate, "lazy_message_id : false", "lazy_message_id : true", 1)
	path := filepath.Join(t.TempDir(), "config.yml")
	if err := os.WriteFile(path, []byte(yml), 0600); err != nil {
		t.Fatalf("写入测试配置失败: %v", err)
	}
	if _, err := config.LoadConfig(path, false); err != nil {
		t.Fatalf("加载测试配置失败: %v", err)
	}
}

// TestM1ResolveGroupContextUsesVirtualIDForLazyPool 钉死 M1/P3 核心：
// lazy 池以虚拟群号+虚拟用户号为键（而非真实 OpenID）。
func TestM1ResolveGroupContextUsesVirtualIDForLazyPool(t *testing.T) {
	round2LoadConfigLazy(t)
	echo.AddLazyMessageIdv2("12345", "42", "LAZY-MSG", time.Now())

	ctx := withOutboundMsgCtx(context.Background(), outboundMsgCtx{
		virtualGroupID: "12345",
		virtualUserID:  "42",
	})
	got, _ := resolveGroupMessageContext(ctx, "REAL-OPENID", map[string][]string{})
	if got != "LAZY-MSG" {
		t.Fatalf("lazy 池应以虚拟群号+虚拟用户号为键: got %q", got)
	}
}

// TestM1ResolvePrivateContextUsesVirtualUser 钉死 M1/P3：私聊以虚拟 user 号命中缓存。
func TestM1ResolvePrivateContextUsesVirtualUser(t *testing.T) {
	archModeLoadConfig(t, "new")
	appID := config.GetAppIDStr()
	virtual, err := idmap.StoreIDv2("42")
	if err != nil {
		t.Fatalf("StoreIDv2 失败: %v", err)
	}
	echo.AddMsgID(appID, virtual, "PRIV-MSG")

	ctx := withOutboundMsgCtx(context.Background(), outboundMsgCtx{
		virtualUserID: strconv.FormatInt(virtual, 10),
	})
	got, _ := resolvePrivateMessageContext(ctx, "42", map[string][]string{})
	if got != "PRIV-MSG" {
		t.Fatalf("私聊应经虚拟 user 号命中缓存 msg_id: got %q", got)
	}
}

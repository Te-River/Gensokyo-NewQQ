package handlers_test

// 本文件是 tester 对出站 parity 的独立复验网（外部测试包）：
// 经真实 adapter/qq.QQSender 验证 new 私聊路径保留 input_notify/stream/active 语义。
// 外部包避免 handlers→adapter/qq→handlers 的测试导入环。

import (
	"context"
	"os"
	"path/filepath"
	"strconv"
	"strings"
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

// adapterLoadConfig 注入 arch_mode 后加载 config 单例。
func adapterLoadConfig(t *testing.T, mode string) {
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

// adapterTestClient 记录回执 payload。
type adapterTestClient struct{ last map[string]interface{} }

func (c *adapterTestClient) SendMessage(m map[string]interface{}) error { c.last = m; return nil }

// adapterRecordingOpenAPI 记录 C2C 正文、群正文与流式片，其余方法走嵌入接口兜底。
type adapterRecordingOpenAPI struct {
	openapi.OpenAPI
	c2c     []dto.APIMessage
	groups  []dto.APIMessage
	streams []*dto.StreamChunk
}

func (m *adapterRecordingOpenAPI) PostGroupMessage(_ context.Context, _ string, msg dto.APIMessage) (*dto.GroupMessageResponse, error) {
	m.groups = append(m.groups, msg)
	return &dto.GroupMessageResponse{Message: &dto.Message{ID: "group-real-1"}}, nil
}

func (m *adapterRecordingOpenAPI) PostC2CMessage(_ context.Context, _ string, msg dto.APIMessage) (*dto.C2CMessageResponse, error) {
	m.c2c = append(m.c2c, msg)
	return &dto.C2CMessageResponse{Message: &dto.Message{ID: "c2c-real-1"}}, nil
}

func (m *adapterRecordingOpenAPI) PostC2CStreamMessage(_ context.Context, _ string, chunk *dto.StreamChunk) (*dto.C2CMessageResponse, error) {
	m.streams = append(m.streams, chunk)
	return &dto.C2CMessageResponse{Message: &dto.Message{ID: "stream-real-1"}}, nil
}

// TestAdapterPrivateInputNotifyAndStream 钉死 new 私聊路径的 input_notify/stream 语义：
// input_notify 先发 MsgType=6 再发正文；stream 只发流式、不再发正文。
func TestAdapterPrivateInputNotifyAndStream(t *testing.T) {
	adapterLoadConfig(t, "new")
	client := &adapterTestClient{}
	msg := callapi.ActionMessage{Action: "send_private_msg", Params: callapi.ParamsContent{UserID: "42"}, Echo: "e-notify"}

	mock := &adapterRecordingOpenAPI{}
	handlers.SetOutboundService(outbound.NewService(qq.NewQQSender(nil, mock), outbound.RetryPolicy{}))
	defer handlers.SetOutboundService(nil)

	found := map[string][]string{"input_notify": {`{"type":"1","second":"30"}`}}
	handled, _ := handlers.TryOutbound(client, nil, mock, msg, "hi", found, identity.TargetPrivate, false)
	if !handled {
		t.Fatal("new 私聊 input_notify 应 handled")
	}
	if len(mock.c2c) != 2 {
		t.Fatalf("input_notify + 正文应各发一次: got %d", len(mock.c2c))
	}
	if gm, ok := mock.c2c[0].(*dto.MessageToCreate); !ok || gm.MsgType != 6 {
		t.Fatalf("首条应为 MsgType=6 输入状态: %#v", mock.c2c[0])
	}

	mock2 := &adapterRecordingOpenAPI{}
	handlers.SetOutboundService(outbound.NewService(qq.NewQQSender(nil, mock2), outbound.RetryPolicy{}))
	found2 := map[string][]string{"stream": {`{"type":"start","qq":"42"}`}}
	handled, _ = handlers.TryOutbound(client, nil, mock2, msg, "hi", found2, identity.TargetPrivate, false)
	if !handled {
		t.Fatal("new 私聊 stream 应 handled")
	}
	if len(mock2.streams) != 1 {
		t.Fatalf("stream 应发 1 片: got %d", len(mock2.streams))
	}
	if len(mock2.c2c) != 0 {
		t.Fatalf("stream 后不应再发正文: got %d", len(mock2.c2c))
	}
}

// fixedIDSender 返回固定真实 QQ message_id。
type fixedIDSender struct{ id string }

func (f *fixedIDSender) Send(_ context.Context, _ identity.ResolvedTarget, _ outbound.QQMessage) (outbound.QQSendResult, error) {
	return outbound.QQSendResult{MessageID: f.id}, nil
}

// TestAdapterGroupReceiptVirtualMessageID 钉死回执 parity：含 echo/group_id/虚拟 message_id
// （非真实 QQ ID），且经 idmap.StoreCachev2 建立映射（副作用保留）。
func TestAdapterGroupReceiptVirtualMessageID(t *testing.T) {
	adapterLoadConfig(t, "new")
	client := &adapterTestClient{}
	const realID = "REAL-QQ-MSG-ID-1"
	handlers.SetOutboundService(outbound.NewService(&fixedIDSender{id: realID}, outbound.RetryPolicy{}))
	defer handlers.SetOutboundService(nil)

	msg := callapi.ActionMessage{Action: "send_group_msg", Params: callapi.ParamsContent{GroupID: "12345"}, Echo: "e-receipt"}
	handled, ret := handlers.TryOutbound(client, nil, nil, msg, "hi", map[string][]string{}, identity.TargetGroup, false)
	if !handled {
		t.Fatal("new 群聊应 handled")
	}
	if client.last == nil {
		t.Fatal("回执应经 client.SendMessage 回发")
	}
	if client.last["echo"] != "e-receipt" {
		t.Fatalf("回执应含 echo: %#v", client.last)
	}
	if _, ok := client.last["group_id"]; !ok {
		t.Fatalf("回执应含 group_id: %#v", client.last)
	}
	data, ok := client.last["data"].(map[string]interface{})
	if !ok {
		t.Fatalf("回执 data 异常: %#v", client.last)
	}
	want, err := idmap.StoreCachev2(realID)
	if err != nil {
		t.Fatalf("StoreCachev2 失败: %v", err)
	}
	got, ok := data["message_id"].(float64)
	if !ok || int64(got) != want {
		t.Fatalf("回执 message_id 应为虚拟 ID: got=%v want=%d", data["message_id"], want)
	}
	if !strings.Contains(ret, `"message_id":`) {
		t.Fatalf("返回串应含虚拟 message_id: %q", ret)
	}
}

// TestAdapterGroupSendRecordsLatestBotMsg 钉死副作用 parity：群发送后应记录
// "机器人最新消息"（delete_group_msg 依赖），与 legacy rememberLatestBotGroupMessageInGroup 对齐。
func TestAdapterGroupSendRecordsLatestBotMsg(t *testing.T) {
	adapterLoadConfig(t, "new")
	client := &adapterTestClient{}
	const realGroupOpenID = "abcdef0123456789abcdef0123456789"
	virtual, err := idmap.StoreIDv2(realGroupOpenID)
	if err != nil {
		t.Fatalf("StoreIDv2 失败: %v", err)
	}
	const realID = "REAL-QQ-MSG-ID-LATEST"
	handlers.SetOutboundService(outbound.NewService(&fixedIDSender{id: realID}, outbound.RetryPolicy{}))
	defer handlers.SetOutboundService(nil)

	msg := callapi.ActionMessage{Action: "send_group_msg", Params: callapi.ParamsContent{GroupID: strconv.FormatInt(virtual, 10)}, Echo: "e-latest"}
	handled, _ := handlers.TryOutbound(client, nil, nil, msg, "hi", map[string][]string{}, identity.TargetGroup, false)
	if !handled {
		t.Fatal("new 群聊应 handled")
	}
	got, err := idmap.GetLatestBotMsgID(realGroupOpenID)
	if err != nil || got != realID {
		t.Fatalf("群发送后应记录机器人最新消息: got=%q err=%v want=%q", got, err, realID)
	}
}

// TestAdapterGroupNoUserContextClearsMsgID 钉死 legacy 主动推送规则：
// 无 user 上下文时缓存的 msg_id 视为过期，应清空（legacy send_group_msg.go:289-293）。
func TestAdapterGroupNoUserContextClearsMsgID(t *testing.T) {
	adapterLoadConfig(t, "new")
	const realGroupOpenID = "fedcba9876543210fedcba9876543210"
	virtual, err := idmap.StoreIDv2(realGroupOpenID)
	if err != nil {
		t.Fatalf("StoreIDv2 失败: %v", err)
	}
	echo.AddMsgID(config.GetAppIDStr(), virtual, "CACHED-MSG-ID")

	mock := &adapterRecordingOpenAPI{}
	handlers.SetOutboundService(outbound.NewService(qq.NewQQSender(nil, mock), outbound.RetryPolicy{}))
	defer handlers.SetOutboundService(nil)
	client := &adapterTestClient{}

	msg := callapi.ActionMessage{Action: "send_group_msg", Params: callapi.ParamsContent{GroupID: strconv.FormatInt(virtual, 10)}, Echo: "e-nouser"}
	handled, _ := handlers.TryOutbound(client, nil, mock, msg, "hi", map[string][]string{}, identity.TargetGroup, false)
	if !handled {
		t.Fatal("new 群聊应 handled")
	}
	if len(mock.groups) != 1 {
		t.Fatalf("应发 1 条群消息: got %d", len(mock.groups))
	}
	gm, ok := mock.groups[0].(*dto.MessageToCreate)
	if !ok {
		t.Fatalf("类型异常: %#v", mock.groups[0])
	}
	if gm.MsgID != "" {
		t.Fatalf("无 user 上下文应清空缓存 msg_id（legacy 语义）: got %q", gm.MsgID)
	}
}

// TestAdapterPrivateActiveClearsMsgID 钉死 active 语义：主动推送清空 MsgID/EventID。
func TestAdapterPrivateActiveClearsMsgID(t *testing.T) {
	adapterLoadConfig(t, "new")
	mock := &adapterRecordingOpenAPI{}
	handlers.SetOutboundService(outbound.NewService(qq.NewQQSender(nil, mock), outbound.RetryPolicy{}))
	defer handlers.SetOutboundService(nil)
	client := &adapterTestClient{}

	msg := callapi.ActionMessage{Action: "send_private_msg", Params: callapi.ParamsContent{UserID: "42"}, Echo: "e-active"}
	found := map[string][]string{"active": {""}}
	handled, _ := handlers.TryOutbound(client, nil, mock, msg, "hi", found, identity.TargetPrivate, false)
	if !handled {
		t.Fatal("new 私聊 active 应 handled")
	}
	if len(mock.c2c) != 1 {
		t.Fatalf("active 应发 1 条: got %d", len(mock.c2c))
	}
	gm, ok := mock.c2c[0].(*dto.MessageToCreate)
	if !ok {
		t.Fatalf("类型异常: %#v", mock.c2c[0])
	}
	if gm.MsgID != "" || gm.EventID != "" {
		t.Fatalf("active 应清空 MsgID/EventID: %#v", gm)
	}
}

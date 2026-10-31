package handlers_test

// S1 新链复验（外部测试包：handlers→adapter/qq 会成环，只能在 handlers_test 里装配）：
// 模板 markdown 缺 content 时，群/私聊都必须回 retcode=1400 且一个字节都不发出去。

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/adapter/qq"
	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/handlers"
	"github.com/hoshinonyaruko/gensokyo/internal/application/outbound"
	"github.com/hoshinonyaruko/gensokyo/internal/domain/identity"
	"github.com/hoshinonyaruko/gensokyo/template"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

// tplMockAPI 只统计群/C2C 发送，其余方法走嵌入接口兜底。
type tplMockAPI struct {
	openapi.OpenAPI
	groupCalls int
	c2cCalls   int
}

func (m *tplMockAPI) PostGroupMessage(_ context.Context, _ string, _ dto.APIMessage) (*dto.GroupMessageResponse, error) {
	m.groupCalls++
	return &dto.GroupMessageResponse{Message: &dto.Message{ID: "tpl-group-id"}}, nil
}

func (m *tplMockAPI) PostC2CMessage(_ context.Context, _ string, _ dto.APIMessage) (*dto.C2CMessageResponse, error) {
	m.c2cCalls++
	return &dto.C2CMessageResponse{Message: &dto.Message{ID: "tpl-c2c-id"}}, nil
}

// tplLoadNewConfig 用仓库模板（arch_mode=new）加载配置单例，确保走新链。
func tplLoadNewConfig(t *testing.T) {
	t.Helper()
	path := filepath.Join(t.TempDir(), "config.yml")
	if err := os.WriteFile(path, []byte(template.ConfigTemplate), 0600); err != nil {
		t.Fatalf("写入测试配置失败: %v", err)
	}
	if _, err := config.LoadConfig(path, false); err != nil {
		t.Fatalf("加载测试配置失败: %v", err)
	}
}

// tplRecorderClient 记录 handler 交给客户端的回执 payload。
type tplRecorderClient struct{ last map[string]interface{} }

func (c *tplRecorderClient) SendMessage(m map[string]interface{}) error { c.last = m; return nil }

func tplRetcodeOf(t *testing.T, raw string) int {
	t.Helper()
	var resp struct {
		RetCode int `json:"retcode"`
	}
	if err := json.Unmarshal([]byte(raw), &resp); err != nil {
		t.Fatalf("回执不是合法 JSON: %q (%v)", raw, err)
	}
	return resp.RetCode
}

// 群/私聊两条新链路径：模板 md 缺 content → 1400，且不产生任何发送。
func TestTryOutboundRejectsTemplateMarkdownEmptyContent(t *testing.T) {
	tplLoadNewConfig(t)
	mock := &tplMockAPI{}
	handlers.SetOutboundService(outbound.NewService(qq.NewQQSender(nil, mock), outbound.RetryPolicy{}))
	defer handlers.SetOutboundService(nil)

	payload := base64.StdEncoding.EncodeToString([]byte(`{"markdown":{"custom_template_id":"TPL","params":[{"key":"a","values":["b"]}]}}`))
	cases := []struct {
		name string
		kind identity.TargetKind
		msg  callapi.ActionMessage
	}{
		{"群聊", identity.TargetGroup, callapi.ActionMessage{Action: "send_group_msg", Params: callapi.ParamsContent{GroupID: "123456"}}},
		{"私聊", identity.TargetPrivate, callapi.ActionMessage{Action: "send_private_msg", Params: callapi.ParamsContent{UserID: "654321"}}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			found := map[string][]string{"markdown": {payload}}
			handled, raw := handlers.TryOutbound(&tplRecorderClient{}, nil, mock, tc.msg, "", found, tc.kind, false)
			if !handled {
				t.Fatalf("new 模式下应由新链处理")
			}
			if got := tplRetcodeOf(t, raw); got != 1400 {
				t.Fatalf("模板 md 缺 content 应回 1400, got %d (%s)", got, raw)
			}
			if !strings.Contains(raw, "模板消息已废弃") {
				t.Fatalf("回执应说明模板已废弃, got %s", raw)
			}
		})
	}
	if mock.groupCalls != 0 || mock.c2cCalls != 0 {
		t.Fatalf("非法载荷不应发出, got group=%d c2c=%d", mock.groupCalls, mock.c2cCalls)
	}
}

// handler 级复验：send_group_msg 带模板 md（含 @全员 标签）走新链时直接回 1400 且不发送。
func TestHandleSendGroupMsgTemplateMarkdownReturns1400(t *testing.T) {
	tplLoadNewConfig(t)
	mock := &tplMockAPI{}
	handlers.SetOutboundService(outbound.NewService(qq.NewQQSender(nil, mock), outbound.RetryPolicy{}))
	defer handlers.SetOutboundService(nil)

	payload := base64.StdEncoding.EncodeToString([]byte(`{"markdown":{"custom_template_id":"TPL"}}`))
	client := &tplRecorderClient{}
	raw, err := handlers.HandleSendGroupMsg(client, nil, mock, callapi.ActionMessage{
		Action: "send_group_msg",
		Params: callapi.ParamsContent{
			GroupID: "123456",
			UserID:  "654321",
			Message: "公告 <qqbot-at-everyone /> [CQ:markdown,data=base64://" + payload + "]",
		},
		Echo: "tpl-md-handler",
	})
	if err != nil {
		t.Fatalf("handler 返回错误: %v", err)
	}
	if got := tplRetcodeOf(t, raw); got != 1400 {
		t.Fatalf("模板 md 缺 content 应回 1400, got %d (%s)", got, raw)
	}
	if mock.groupCalls != 0 {
		t.Fatalf("非法载荷不应发出, got %d 次发送", mock.groupCalls)
	}
}

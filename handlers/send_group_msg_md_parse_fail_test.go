package handlers

// 回归：markdown 载荷解析失败（parseMarkdownFromMessage 返回 md == nil）时，
// 正文的 [CQ:at] 不得按 markdown 语境转成 <qqbot-at-user>。
// 该标签只在 markdown 消息里可渲染，进了纯文本 Content 会被群成员看成字面文本；
// md == nil 时正文必须原样走纯文本三段式（<@openid> / @用户名 / 删除）。

import (
	"context"
	"encoding/base64"
	"strconv"
	"strings"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

// mdFailRecOpenAPI 记录每次 PostGroupMessage 的请求体（md == nil 时正文与富媒体循环会各发一条）。
type mdFailRecOpenAPI struct {
	openapi.OpenAPI
	msgs []dto.APIMessage
}

func (m *mdFailRecOpenAPI) PostGroupMessage(_ context.Context, _ string, msg dto.APIMessage) (*dto.GroupMessageResponse, error) {
	m.msgs = append(m.msgs, msg)
	return &dto.GroupMessageResponse{Message: &dto.Message{ID: "mock-group-msg-id"}}, nil
}

// mdAtVirtualID 注册 OpenID ↔ 虚拟号映射，返回 (虚拟号, OpenID)。
// resolvePlainTextAtMentions 的用户名缓存为空时回退 <@OpenID>，断言据此判定。
func mdAtVirtualID(t *testing.T, prefix byte) (string, string) {
	t.Helper()
	openID := openID32(prefix)
	virtual, err := idmap.StoreIDv2(openID)
	if err != nil {
		t.Fatalf("StoreIDv2 失败: %v", err)
	}
	return strconv.FormatInt(virtual, 10), openID
}

// TestSendGroupMsgMarkdownParseFailKeepsPlainTextAt markdown 载荷解析失败时，
// 正文 [CQ:at] 走纯文本转换，出站 payload 任何位置都不得出现 <qqbot-at-user>。
func TestSendGroupMsgMarkdownParseFailKeepsPlainTextAt(t *testing.T) {
	// 两种都是 md == nil：{} 解析成功但无 markdown 字段；content 为数字直接解析报错
	for _, payload := range []string{`{}`, `{"markdown":{"content":123}}`} {
		t.Run(payload, func(t *testing.T) {
			virtualID, openID := mdAtVirtualID(t, 'a')
			encoded := base64.StdEncoding.EncodeToString([]byte(payload))

			mock := &mdFailRecOpenAPI{}
			client := &mdGateTestClient{}
			msg := callapi.ActionMessage{
				Action: "send_group_msg",
				Params: callapi.ParamsContent{
					GroupID: openID32('g'),
					UserID:  openID32('u'),
					Message: "PREFIX-KB [CQ:at,qq=" + virtualID + "] [CQ:markdown,data=base64://" + encoded + "]",
				},
				Echo: "md-at-parse-fail",
			}

			// md == nil 时富媒体循环可能因 markdown 键无法生成消息而提前返回错误，正文已在之前发出
			if _, err := HandleSendGroupMsg(client, nil, mock, msg); err != nil {
				t.Logf("handler 返回: %v", err)
			}
			if len(mock.msgs) == 0 {
				t.Fatal("应至少下发一条群消息")
			}

			for i, apiMsg := range mock.msgs {
				mtc, ok := apiMsg.(*dto.MessageToCreate)
				if !ok {
					t.Fatalf("第 %d 条应为 MessageToCreate, got %T", i, apiMsg)
				}
				if strings.Contains(mtc.Content, "qqbot-at-user") {
					t.Errorf("第 %d 条 Content 混入 markdown 语境 @: %q", i, mtc.Content)
				}
				if mtc.Markdown != nil && strings.Contains(mtc.Markdown.Content, "qqbot-at-user") {
					t.Errorf("第 %d 条 Markdown.Content 混入 markdown 语境 @: %q", i, mtc.Markdown.Content)
				}
			}

			first, ok := mock.msgs[0].(*dto.MessageToCreate)
			if !ok {
				t.Fatalf("首条应为 MessageToCreate, got %T", mock.msgs[0])
			}
			if first.MsgType != 0 {
				t.Errorf("首条 MsgType = %d, want 0（纯文本路径）", first.MsgType)
			}
			if !strings.Contains(first.Content, "PREFIX-KB") {
				t.Errorf("正文前缀丢失: %q", first.Content)
			}
			if !strings.Contains(first.Content, "<@"+openID+">") {
				t.Errorf("正文 @ 未走纯文本形态 (<@OpenID>): %q", first.Content)
			}
			if strings.Contains(first.Content, "[CQ:at") {
				t.Errorf("正文残留 CQ at 码: %q", first.Content)
			}
		})
	}
}

// TestSendGroupMsgMarkdownParsedKeepsQqbotAtUserInMarkdown 解析成功时保持既有行为：
// 正文按 markdown 合并进 md.Content，@ 仍转 <qqbot-at-user>，且只下发一条 msg_type=2。
func TestSendGroupMsgMarkdownParsedKeepsQqbotAtUserInMarkdown(t *testing.T) {
	virtualID, openID := mdAtVirtualID(t, 'b')
	encoded := base64.StdEncoding.EncodeToString([]byte(`{"markdown":{"content":"### BODY-TEST [CQ:at,qq=` + virtualID + `]"}}`))

	mock := &mdFailRecOpenAPI{}
	client := &mdGateTestClient{}
	msg := callapi.ActionMessage{
		Action: "send_group_msg",
		Params: callapi.ParamsContent{
			GroupID: openID32('g'),
			UserID:  openID32('u'),
			Message: "PREFIX-KB [CQ:markdown,data=base64://" + encoded + "]",
		},
		Echo: "md-at-parse-ok",
	}

	if _, err := HandleSendGroupMsg(client, nil, mock, msg); err != nil {
		t.Fatalf("HandleSendGroupMsg 返回错误: %v", err)
	}
	if len(mock.msgs) != 1 {
		t.Fatalf("markdown 消息应只下发一条, got %d", len(mock.msgs))
	}
	mtc, ok := mock.msgs[0].(*dto.MessageToCreate)
	if !ok {
		t.Fatalf("应为 MessageToCreate, got %T", mock.msgs[0])
	}
	if mtc.MsgType != 2 || mtc.Markdown == nil {
		t.Fatalf("markdown 未生效: MsgType=%d Markdown=%+v", mtc.MsgType, mtc.Markdown)
	}
	if mtc.Content != "" {
		t.Errorf("Content = %q, want 空串", mtc.Content)
	}
	if !strings.HasPrefix(mtc.Markdown.Content, "PREFIX-KB") || !strings.Contains(mtc.Markdown.Content, "\n### BODY-TEST") {
		t.Errorf("正文未按 markdown 前缀合并: %q", mtc.Markdown.Content)
	}
	if !strings.Contains(mtc.Markdown.Content, `<qqbot-at-user id="`+openID+`" />`) {
		t.Errorf("markdown 内 @ 未转 <qqbot-at-user>: %q", mtc.Markdown.Content)
	}
	if strings.Contains(mtc.Markdown.Content, "[CQ:at") {
		t.Errorf("markdown 内容残留 CQ at 码: %q", mtc.Markdown.Content)
	}
}

// TestSendGroupMsgPlainTextAtUnchanged 无 markdown 的纯文本 + at 保持既有三段式转换。
func TestSendGroupMsgPlainTextAtUnchanged(t *testing.T) {
	virtualID, openID := mdAtVirtualID(t, 'c')

	mock := &mdFailRecOpenAPI{}
	client := &mdGateTestClient{}
	msg := callapi.ActionMessage{
		Action: "send_group_msg",
		Params: callapi.ParamsContent{
			GroupID: openID32('g'),
			UserID:  openID32('u'),
			Message: "纯文本 [CQ:at,qq=" + virtualID + "] 尾巴",
		},
		Echo: "plain-at-unchanged",
	}

	if _, err := HandleSendGroupMsg(client, nil, mock, msg); err != nil {
		t.Fatalf("HandleSendGroupMsg 返回错误: %v", err)
	}
	if len(mock.msgs) != 1 {
		t.Fatalf("纯文本消息应只下发一条, got %d", len(mock.msgs))
	}
	mtc, ok := mock.msgs[0].(*dto.MessageToCreate)
	if !ok {
		t.Fatalf("应为 MessageToCreate, got %T", mock.msgs[0])
	}
	if mtc.MsgType != 0 {
		t.Errorf("MsgType = %d, want 0", mtc.MsgType)
	}
	if !strings.Contains(mtc.Content, "<@"+openID+">") {
		t.Errorf("@ 未转为 <@OpenID>: %q", mtc.Content)
	}
	if strings.Contains(mtc.Content, "qqbot-at-user") || strings.Contains(mtc.Content, "[CQ:at") {
		t.Errorf("纯文本正文残留 markdown/CQ 形态 @: %q", mtc.Content)
	}
}

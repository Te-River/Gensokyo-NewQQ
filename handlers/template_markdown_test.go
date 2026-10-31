package handlers

// S1/S2/S3 回归：模板 markdown 缺 content 必须报错（不静默降级），
// @全员 CQ 码大小写不敏感归一，且非法载荷不得占用当日额度。

import (
	"context"
	"encoding/base64"
	"errors"
	"strings"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/tencent-connect/botgo/dto"
)

// templateMarkdownPayload 构造只含废弃模板字段的 markdown 载荷（base64，与 foundItems 同形）。
func templateMarkdownPayload(t *testing.T, rawJSON string) string {
	t.Helper()
	return base64.StdEncoding.EncodeToString([]byte(rawJSON))
}

// ---------- S1：parseMDData 拒绝模板缺 content ----------

func TestParseMDDataRejectsTemplateWithoutContent(t *testing.T) {
	cases := []struct {
		name string
		in   string
	}{
		{"只有模板 id", `{"markdown":{"custom_template_id":"TPL"}}`},
		{"模板 + params 无 content", `{"markdown":{"custom_template_id":"TPL","params":[{"key":"a","values":["b"]}]}}`},
		{"模板 + 空白 content", `{"markdown":{"custom_template_id":"TPL","content":"   "}}`},
		{"双层包装的模板载荷", `{"markdown":{"markdown":{"custom_template_id":"TPL"}}}`},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			md, kb, err := parseMDData([]byte(tc.in))
			if err == nil {
				t.Fatalf("模板缺 content 应报错, got md=%v kb=%v", md, kb)
			}
			if !errors.Is(err, errTemplateMarkdownNoContent) {
				t.Fatalf("应为 errTemplateMarkdownNoContent, got %v", err)
			}
			if md != nil || kb != nil {
				t.Fatalf("报错时不得返回半成品 md/kb, got md=%v kb=%v", md, kb)
			}
			if !strings.Contains(err.Error(), "模板消息已废弃") {
				t.Fatalf("错误信息应说明模板已废弃, got %v", err)
			}
		})
	}
}

// 模板字段与 content 同现：只发 content（模板字段官方已废弃，不再透传）。
func TestParseMDDataTemplateWithContentSendsContentOnly(t *testing.T) {
	in := `{"markdown":{"custom_template_id":"TPL","params":[{"key":"a","values":["b"]}],"content":"# 标题"}}`
	md, _, err := parseMDData([]byte(in))
	if err != nil {
		t.Fatalf("有 content 时不应报错: %v", err)
	}
	if md == nil || md.Content != "# 标题" {
		t.Fatalf("应保留 content, got %#v", md)
	}
	if md.CustomTemplateID != "" || len(md.Params) != 0 {
		t.Fatalf("不得透传废弃的模板字段, got id=%q params=%v", md.CustomTemplateID, md.Params)
	}
}

// parseMarkdownFromMessageE 必须把错误交给调用方（parseMarkdownFromMessage 只记日志）。
func TestParseMarkdownFromMessageEReturnsTemplateError(t *testing.T) {
	payload := templateMarkdownPayload(t, `{"markdown":{"custom_template_id":"TPL"}}`)
	if _, _, err := parseMarkdownFromMessageE(payload); !errors.Is(err, errTemplateMarkdownNoContent) {
		t.Fatalf("应返回 errTemplateMarkdownNoContent, got %v", err)
	}
}

// ---------- S1：新链群/私聊发送函数拒绝非法载荷（发送前拦截） ----------

func TestSendFoundItemsGroupRejectsTemplateMarkdown(t *testing.T) {
	mock := &cardMockAPI{}
	found := map[string][]string{"markdown": {templateMarkdownPayload(t, `{"markdown":{"custom_template_id":"TPL"}}`)}}
	if _, err := SendFoundItemsGroup(context.Background(), openID32('g'), "", found, mock); !errors.Is(err, errTemplateMarkdownNoContent) {
		t.Fatalf("群聊应返回模板废弃错误, got %v", err)
	}
	if mock.groupCalls != 0 {
		t.Fatalf("非法载荷不应发出, got %d 次发送", mock.groupCalls)
	}
}

func TestSendFoundItemsPrivateRejectsTemplateMarkdown(t *testing.T) {
	mock := &cardMockAPI{}
	found := map[string][]string{"markdown": {templateMarkdownPayload(t, `{"markdown":{"custom_template_id":"TPL"}}`)}}
	if _, err := SendFoundItemsPrivate(context.Background(), openID32('u'), "", found, false, mock); !errors.Is(err, errTemplateMarkdownNoContent) {
		t.Fatalf("私聊应返回模板废弃错误, got %v", err)
	}
	if mock.c2cCalls != 0 {
		t.Fatalf("非法载荷不应发出, got %d 次发送", mock.c2cCalls)
	}
}

// ---------- S1（legacy 链）：handler 级同样回 1400 ----------

// legacy 群聊：模板 md 缺 content → 1400，且一次都不发。
func TestHandleSendGroupMsgLegacyTemplateMarkdownReturns1400(t *testing.T) {
	// 新链未注入时 handler 走 legacy 分支（arch_mode=new 也会回退 legacy）
	SetOutboundService(nil)
	t.Cleanup(func() { SetOutboundService(nil) })

	mock := &cardMockAPI{}
	payload := templateMarkdownPayload(t, `{"markdown":{"custom_template_id":"TPL"}}`)
	raw, err := HandleSendGroupMsg(&cardRecorderClient{}, nil, mock, callapi.ActionMessage{
		Action: "send_group_msg",
		Params: callapi.ParamsContent{
			GroupID: openID32('g'),
			UserID:  openID32('u'),
			Message: "[CQ:markdown,data=base64://" + payload + "]",
		},
		Echo: "legacy-tpl-md-group",
	})
	if err != nil {
		t.Fatalf("handler 返回错误: %v", err)
	}
	if got := retcodeOf(t, raw); got != 1400 {
		t.Fatalf("legacy 群聊模板 md 应回 1400, got %d (%s)", got, raw)
	}
	if !strings.Contains(raw, "模板消息已废弃") {
		t.Fatalf("回执应说明模板已废弃, got %s", raw)
	}
	if mock.groupCalls != 0 {
		t.Fatalf("非法载荷不应发出, got %d 次发送", mock.groupCalls)
	}
}

// legacy 私聊：模板 md 缺 content → 1400，且一次都不发。
func TestHandleSendPrivateMsgLegacyTemplateMarkdownReturns1400(t *testing.T) {
	SetOutboundService(nil)
	t.Cleanup(func() { SetOutboundService(nil) })

	mock := &cardMockAPI{}
	payload := templateMarkdownPayload(t, `{"markdown":{"custom_template_id":"TPL"}}`)
	raw, err := HandleSendPrivateMsg(&cardRecorderClient{}, nil, mock, callapi.ActionMessage{
		Action: "send_private_msg",
		Params: callapi.ParamsContent{
			UserID:  openID32('u'),
			Message: "[CQ:markdown,data=base64://" + payload + "]",
		},
		Echo: "legacy-tpl-md-private",
	})
	if err != nil {
		t.Fatalf("handler 返回错误: %v", err)
	}
	if got := retcodeOf(t, raw); got != 1400 {
		t.Fatalf("legacy 私聊模板 md 应回 1400, got %d (%s)", got, raw)
	}
	if !strings.Contains(raw, "模板消息已废弃") {
		t.Fatalf("回执应说明模板已废弃, got %s", raw)
	}
	if mock.c2cCalls != 0 {
		t.Fatalf("非法载荷不应发出, got %d 次发送", mock.c2cCalls)
	}
}

// ---------- S2：大小写不敏感的快速路径 ----------

func TestNormalizeAtEveryoneLowercasePrefix(t *testing.T) {
	cases := []struct{ name, in, want string }{
		{"全小写前缀", "[cq:at,qq=all] 公告", atEveryoneTag + " 公告"},
		{"混合大小写", "[Cq:At,qq=ALL] 公告", atEveryoneTag + " 公告"},
		{"小写前缀全员别名", "[cq:at,qq=everyone] 公告", atEveryoneTag + " 公告"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got := NormalizeAtEveryone(tc.in)
			if got != tc.want {
				t.Fatalf("NormalizeAtEveryone(%q) = %q, want %q", tc.in, got, tc.want)
			}
			if strings.Contains(strings.ToLower(got), "[cq:at") {
				t.Fatalf("检测到就必须归一，不得残留 CQ 码: %q", got)
			}
		})
	}
}

// markdown 载荷内的小写 CQ 码同样要归一（检测与归一同一口径）。
func TestNormalizeAtEveryoneInMarkdownLowercasePrefix(t *testing.T) {
	payload := templateMarkdownPayload(t, `{"markdown":{"content":"[cq:at,qq=all] 公告"}}`)
	got := normalizeAtEveryoneInMarkdown(payload)
	decoded, err := base64.StdEncoding.DecodeString(got)
	if err != nil {
		t.Fatalf("载荷应仍是合法 base64: %v", err)
	}
	if !strings.Contains(string(decoded), atEveryoneTag) {
		t.Fatalf("小写 CQ 码应归一为官方标签, got %s", decoded)
	}
}

// 端到端：小写前缀的 @全员 既被归一，也只扣一次额度（同日第二次 1400）。
func TestSendGroupMsgLowercaseAtEveryoneNormalizedAndCountedOnce(t *testing.T) {
	store := useMemAtEveryone(t)
	group := openID32('g')
	send := func(echo string) (string, *cardMockAPI) {
		mock := &cardMockAPI{}
		raw, err := HandleSendGroupMsg(&cardRecorderClient{}, nil, mock, callapi.ActionMessage{
			Action: "send_group_msg",
			Params: callapi.ParamsContent{
				GroupID: group,
				UserID:  openID32('u'),
				Message: "[cq:at,qq=all] 公告",
			},
			Echo: echo,
		})
		if err != nil {
			t.Fatalf("handler 返回错误: %v", err)
		}
		return raw, mock
	}

	_, mock := send("lowercase-first")
	mtc, ok := mock.lastGroup.(*dto.MessageToCreate)
	if !ok {
		t.Fatalf("应发送 MessageToCreate, got %T", mock.lastGroup)
	}
	if !strings.Contains(mtc.Content, atEveryoneTag) {
		t.Fatalf("小写 CQ 码应被归一, got %q", mtc.Content)
	}
	if strings.Contains(strings.ToLower(mtc.Content), "[cq:at") {
		t.Fatalf("content 不应残留 CQ 码, got %q", mtc.Content)
	}
	if !strings.Contains(store.data[group], `"count":1`) {
		t.Fatalf("应只扣一次额度, got %q", store.data[group])
	}

	raw, second := send("lowercase-second")
	if got := retcodeOf(t, raw); got != 1400 {
		t.Fatalf("同日第二次应回 1400, got %d (%s)", got, raw)
	}
	if second.groupCalls != 0 {
		t.Fatalf("超限时不应发送消息")
	}
}

// ---------- S3：模板 md + @全员 不得占额 ----------

// 非法载荷的请求一定发不出标签：占额前就应判定为「无 @全员」。
func TestAtEveryonePresentIgnoresRejectedMarkdown(t *testing.T) {
	found := map[string][]string{
		"markdown": {templateMarkdownPayload(t, `{"markdown":{"custom_template_id":"TPL"}}`)},
	}
	if atEveryonePresent("公告 "+atEveryoneTag, found) {
		t.Fatalf("会被拒的载荷不得视为带 @全员")
	}
	// 合法载荷仍要正常判定
	ok := map[string][]string{
		"markdown": {templateMarkdownPayload(t, `{"markdown":{"content":"公告 `+atEveryoneTag+`"}}`)},
	}
	if !atEveryonePresent("公告", ok) {
		t.Fatalf("合法 markdown 里的标签应被识别")
	}
}

// 端到端：模板 md + @全员 的请求结束后，当日额度必须仍可用（未被占用）。
func TestSendGroupMsgTemplateMarkdownDoesNotChargeQuota(t *testing.T) {
	store := useMemAtEveryone(t)
	group := openID32('g')
	payload := templateMarkdownPayload(t, `{"markdown":{"custom_template_id":"TPL"}}`)

	mock := &cardMockAPI{}
	_, err := HandleSendGroupMsg(&cardRecorderClient{}, nil, mock, callapi.ActionMessage{
		Action: "send_group_msg",
		Params: callapi.ParamsContent{
			GroupID: group,
			UserID:  openID32('u'),
			Message: "公告 " + atEveryoneTag + " [CQ:markdown,data=base64://" + payload + "]",
		},
		Echo: "tpl-md-at-everyone",
	})
	if err != nil {
		t.Fatalf("handler 返回错误: %v", err)
	}
	if got, _ := store.Read(group); got != "" {
		t.Fatalf("非法载荷不得占额, got %q", got)
	}
}

package handlers

import (
	"encoding/base64"
	"encoding/json"
	"reflect"
	"strconv"
	"strings"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/tencent-connect/botgo/dto"
)

// mdKeyboardPayload 复现活体载荷：markdown 顶层 content 为字符串，keyboard 走
// content.rows 形态（qbind 等插件 markdown 内嵌按钮的真实结构），权限指定单个用户。
func mdKeyboardPayload(userID string) string {
	return `{"content":"### 确认绑定","keyboard":{"content":{"rows":[{"buttons":[{"id":"btn_x","render_data":{"label":"点我","visited_label":"点我","style":1},"action":{"type":2,"data":"qbind","enter":true,"reply":false,"anchor":0,"unsupport_tips":"当前版本不支持","permission":{"type":0,"specify_user_ids":["` + userID + `"]}}}]}]}}}`
}

// TestParseMarkdownFromMessageEmbeddedKeyboardRows 回归：markdown 顶层 content 是字符串、
// keyboard 形如 {"content":{"rows":[...]}} 时，keyboard 曾被解析成空 content
// （出站 {"keyboard":{"content":{}}}，群里看不到按钮）。
func TestParseMarkdownFromMessageEmbeddedKeyboardRows(t *testing.T) {
	payload := mdKeyboardPayload("610636458")
	md, kb := parseMarkdownFromMessage(base64.StdEncoding.EncodeToString([]byte(payload)))

	if md == nil || md.Content != "### 确认绑定" {
		t.Fatalf("md.Content: got %+v, want 原文", md)
	}
	if kb == nil || kb.Content == nil || len(kb.Content.Rows) == 0 {
		t.Fatalf("keyboard 应解析出 rows, got %+v", kb)
	}

	rows := kb.Content.Rows
	if len(rows) != 1 || len(rows[0].Buttons) != 1 {
		t.Fatalf("rows/buttons 数量: got %d/%d, want 1/1", len(rows), len(rows[0].Buttons))
	}
	btn := rows[0].Buttons[0]
	if btn.ID != "btn_x" {
		t.Errorf("btn.ID = %q, want btn_x", btn.ID)
	}
	if btn.RenderData == nil || btn.RenderData.Label != "点我" || btn.RenderData.VisitedLabel != "点我" || btn.RenderData.Style != 1 {
		t.Errorf("render_data 字段不全: %+v", btn.RenderData)
	}
	if btn.Action == nil {
		t.Fatal("action 缺失")
	}
	if btn.Action.Type != 2 || btn.Action.Data != "qbind" || !btn.Action.Enter || btn.Action.Reply || btn.Action.AnChor != 0 || btn.Action.UnsupportTips != "当前版本不支持" {
		t.Errorf("action 字段不全: %+v", btn.Action)
	}
	perm := btn.Action.Permission
	if perm == nil || perm.Type != 0 || len(perm.SpecifyUserIDs) != 1 || perm.SpecifyUserIDs[0] != "610636458" {
		t.Errorf("permission 字段不全: %+v", perm)
	}

	// 出站体应与载荷里的 keyboard 对象逐字段等价（不丢任何字段）
	var full map[string]json.RawMessage
	if err := json.Unmarshal([]byte(payload), &full); err != nil {
		t.Fatalf("载荷非法: %v", err)
	}
	var want, got interface{}
	if err := json.Unmarshal(full["keyboard"], &want); err != nil {
		t.Fatalf("载荷 keyboard 非法: %v", err)
	}
	out, err := json.Marshal(kb)
	if err != nil {
		t.Fatalf("marshal keyboard 失败: %v", err)
	}
	if err := json.Unmarshal(out, &got); err != nil {
		t.Fatalf("marshal 结果非法: %v", err)
	}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("出站 keyboard 与载荷不一致:\n got: %s\nwant: %s", out, full["keyboard"])
	}
}

// TestParseMDDataKeyboardRawFallback 第三方/类型不符字段无法映射到 DTO 时，
// 整块 keyboard 原样透传（按钮不丢），且 specify_user_ids 的虚拟号仍转 OpenID。
func TestParseMDDataKeyboardRawFallback(t *testing.T) {
	const userOpenID = "k0123456789abcdef0123456789abcde"
	virtual, err := idmap.StoreIDv2(userOpenID)
	if err != nil {
		t.Fatalf("StoreIDv2 失败: %v", err)
	}
	virtualID := strconv.FormatInt(virtual, 10)

	// style 为字符串（第三方扩展写法的类型不符）→ 无法映射到 keyboard.RenderData
	payload := strings.Replace(mdKeyboardPayload(virtualID), `"style":1`, `"style":"1"`, 1)

	md, kb, err := parseMDData([]byte(payload))
	if err != nil {
		t.Fatalf("parseMDData 异常: %v", err)
	}
	if md == nil || md.Content != "### 确认绑定" {
		t.Fatalf("md.Content: got %+v", md)
	}
	if kb == nil || len(kb.Raw) == 0 {
		t.Fatalf("无法映射时应原样透传 keyboard, got %+v", kb)
	}
	out, err := json.Marshal(kb)
	if err != nil {
		t.Fatalf("marshal keyboard 失败: %v", err)
	}
	body := string(out)
	for _, want := range []string{`"btn_x"`, `"style":"1"`, `"qbind"`} {
		if !strings.Contains(body, want) {
			t.Errorf("透传体缺少 %s: %s", want, body)
		}
	}
	var parsed struct {
		Content struct {
			Rows []struct {
				Buttons []struct {
					Action struct {
						Permission struct {
							SpecifyUserIDs []string `json:"specify_user_ids"`
						} `json:"permission"`
					} `json:"action"`
				} `json:"buttons"`
			} `json:"rows"`
		} `json:"content"`
	}
	if err := json.Unmarshal(kb.Raw, &parsed); err != nil {
		t.Fatalf("透传体非法: %v", err)
	}
	ids := parsed.Content.Rows[0].Buttons[0].Action.Permission.SpecifyUserIDs
	if len(ids) != 1 || ids[0] != userOpenID {
		t.Errorf("specify_user_ids = %v, want [%s]（透传体: %s）", ids, userOpenID, body)
	}
}

// TestSendGroupMsgEmbeddedKeyboardRows 端到端：send_group_msg + 该载荷 →
// 上游 MessageToCreate.Keyboard.Content.Rows 非空，specify_user_ids[0] 为 32 位 OpenID。
func TestSendGroupMsgEmbeddedKeyboardRows(t *testing.T) {
	const userOpenID = "kfedcba9876543210fedcba987654321"
	virtual, err := idmap.StoreIDv2(userOpenID)
	if err != nil {
		t.Fatalf("StoreIDv2 失败: %v", err)
	}
	b64 := base64.StdEncoding.EncodeToString([]byte(mdKeyboardPayload(strconv.FormatInt(virtual, 10))))

	mock := &mockMDSendOpenAPI{}
	client := &mdGateTestClient{}
	msg := callapi.ActionMessage{
		Action: "send_group_msg",
		Params: callapi.ParamsContent{
			GroupID: openID32('g'),
			UserID:  openID32('u'),
			Message: "[CQ:markdown,data=base64://" + b64 + "]",
		},
		Echo: "md-kb-rows",
	}

	if _, err := HandleSendGroupMsg(client, nil, mock, msg); err != nil {
		t.Fatalf("HandleSendGroupMsg 返回错误: %v", err)
	}
	if mock.groupCalls != 1 {
		t.Fatalf("应调用一次 PostGroupMessage, got %d", mock.groupCalls)
	}
	mtc, ok := mock.lastGroup.(*dto.MessageToCreate)
	if !ok {
		t.Fatalf("应发送 MessageToCreate, got %T", mock.lastGroup)
	}
	if mtc.MsgType != 2 || mtc.Markdown == nil || mtc.Markdown.Content != "### 确认绑定" {
		t.Fatalf("markdown 未生效: MsgType=%d Markdown=%+v", mtc.MsgType, mtc.Markdown)
	}
	if mtc.Keyboard == nil || mtc.Keyboard.Content == nil || len(mtc.Keyboard.Content.Rows) == 0 {
		t.Fatalf("Keyboard.Content.Rows 应非空, got %+v", mtc.Keyboard)
	}
	btn := mtc.Keyboard.Content.Rows[0].Buttons[0]
	if btn.ID != "btn_x" || btn.Action == nil || btn.Action.Data != "qbind" {
		t.Fatalf("按钮字段不全: %+v", btn)
	}
	perm := btn.Action.Permission
	if perm == nil || len(perm.SpecifyUserIDs) != 1 {
		t.Fatalf("permission 缺失: %+v", perm)
	}
	if perm.SpecifyUserIDs[0] != userOpenID || len(perm.SpecifyUserIDs[0]) != 32 {
		t.Errorf("specify_user_ids[0] = %q, want 32 位 OpenID %q", perm.SpecifyUserIDs[0], userOpenID)
	}
}

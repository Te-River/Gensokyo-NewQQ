package handlers

// P3 全链路审计三条缺口（G3/G6/G7）的回归用例：
//   G3：send_private_msg_wakeup 丢 keyboard/card/stream
//   G6：send_group_forward_msg 丢节点 uin/name 与节点内 keyboard
//   G7：[CQ:contact/poke/json/forward] 无解析分支，字面量泄漏给群成员

import (
	"context"
	"strconv"
	"strings"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/dto/keyboard"
	"github.com/tencent-connect/botgo/openapi"
)

// kbJSON 单行单按钮 keyboard 的 JSON 载荷（[CQ:keyboard,data=...] 形态）。
func kbJSON(pt keyboard.PermissionType, userIDs []string) string {
	ids := make([]string, 0, len(userIDs))
	for _, id := range userIDs {
		ids = append(ids, `"`+id+`"`)
	}
	return `{"content":{"rows":[{"buttons":[{"render_data":{"label":"确认","style":1},` +
		`"action":{"type":2,"data":"ok","permission":{"type":` + strconv.Itoa(int(pt)) + `,"specify_user_ids":[` + strings.Join(ids, ",") + `]}}}]}]}}`
}

// drainMyLog 取走日志通道中当前的条目文本（emitLog 为非阻塞投递，测试侧主动消费）。
func drainMyLog() []string {
	var out []string
	for {
		select {
		case e := <-mylog.LogChannel():
			out = append(out, e.Message)
		default:
			return out
		}
	}
}

// ---------- G3：唤醒消息的 keyboard ----------

// TestWakeupStandaloneKeyboardAttached 独立 [CQ:keyboard] 必须随召回消息下发，
// 且保持 wakeup 语义（is_wakeup=true、msg_id/event_id 为空）、C2C 权限归一为 type=2。
func TestWakeupStandaloneKeyboardAttached(t *testing.T) {
	mock := &mockMDSendOpenAPI{}
	client := &mdGateTestClient{}

	msg := callapi.ActionMessage{
		Action: "send_private_msg_wakeup",
		Params: callapi.ParamsContent{
			UserID:  openID32('u'),
			Message: "[CQ:keyboard,data=" + kbJSON(keyboard.PermissionTypeSpecifyUserIDs, []string{"openid-a"}) + "]召回文本",
		},
	}

	if _, err := HandleSendPrivateMsgWakeup(client, nil, mock, msg); err != nil {
		t.Fatalf("HandleSendPrivateMsgWakeup 返回错误: %v", err)
	}
	if mock.c2cCalls != 1 {
		t.Fatalf("应调用一次 PostC2CMessage, got %d", mock.c2cCalls)
	}
	mtc, ok := mock.lastC2C.(*dto.MessageToCreate)
	if !ok {
		t.Fatalf("应发送 MessageToCreate, got %T", mock.lastC2C)
	}
	if mtc.Keyboard == nil || mtc.Keyboard.Content == nil || len(mtc.Keyboard.Content.Rows) == 0 {
		t.Fatalf("独立 [CQ:keyboard] 应附加到召回消息, got %+v", mtc.Keyboard)
	}
	perm := mtc.Keyboard.Content.Rows[0].Buttons[0].Action.Permission
	if perm == nil || perm.Type != keyboard.PermissionTypAll {
		t.Errorf("Permission = %+v, want type=2（C2C 归一）", perm)
	}
	if !mtc.IsWakeup || mtc.MsgID != "" || mtc.EventID != "" {
		t.Errorf("唤醒语义被破坏: IsWakeup=%v msg_id=%q event_id=%q", mtc.IsWakeup, mtc.MsgID, mtc.EventID)
	}
	if mtc.Content != "召回文本" {
		t.Errorf("正文 = %q, want 召回文本", mtc.Content)
	}
}

// TestWakeupMarkdownEmbeddedKeyboard markdown 内嵌 keyboard 的唤醒消息同样带按钮。
func TestWakeupMarkdownEmbeddedKeyboard(t *testing.T) {
	mock := &mockMDSendOpenAPI{}
	client := &mdGateTestClient{}

	mdJSON := `{"markdown":{"content":"### 召回"},"keyboard":` + kbJSON(keyboard.PermissionTypeSpecifyUserIDs, []string{"openid-a"}) + `}`
	msg := callapi.ActionMessage{
		Action: "send_private_msg_wakeup",
		Params: callapi.ParamsContent{
			UserID:  openID32('u'),
			Message: "[CQ:markdown,data=" + mdJSON + "]",
		},
	}

	if _, err := HandleSendPrivateMsgWakeup(client, nil, mock, msg); err != nil {
		t.Fatalf("HandleSendPrivateMsgWakeup 返回错误: %v", err)
	}
	if mock.c2cCalls != 1 {
		t.Fatalf("应调用一次 PostC2CMessage, got %d", mock.c2cCalls)
	}
	mtc, ok := mock.lastC2C.(*dto.MessageToCreate)
	if !ok {
		t.Fatalf("应发送 MessageToCreate, got %T", mock.lastC2C)
	}
	if mtc.Markdown == nil || mtc.MsgType != 2 {
		t.Fatalf("markdown 未生效: Markdown=%+v MsgType=%d", mtc.Markdown, mtc.MsgType)
	}
	if mtc.Keyboard == nil || mtc.Keyboard.Content == nil || len(mtc.Keyboard.Content.Rows) == 0 {
		t.Fatalf("markdown 内嵌 keyboard 应下发, got %+v", mtc.Keyboard)
	}
	if !mtc.IsWakeup {
		t.Error("is_wakeup 应保持 true")
	}
}

// ---------- G6：转发节点 uin/name 与节点内 keyboard ----------

// forwardRecMock 记录每次 PostGroupMessage 的请求体（逐节点降级下发）。
type forwardRecMock struct {
	openapi.OpenAPI
	groupMsgs []dto.APIMessage
}

func (m *forwardRecMock) PostGroupMessage(_ context.Context, _ string, msg dto.APIMessage) (*dto.GroupMessageResponse, error) {
	m.groupMsgs = append(m.groupMsgs, msg)
	return &dto.GroupMessageResponse{Message: &dto.Message{ID: "mock-group-msg-id"}}, nil
}

// TestForwardNodeUinNameAndKeyboard 两节点转发：每节点 uin/name 保留在上游请求体，
// 节点内 [CQ:keyboard] 下发且 specify_user_ids 虚拟号已转 OpenID。
func TestForwardNodeUinNameAndKeyboard(t *testing.T) {
	const nodeUserOpenID = "k0123456789abcdef0123456789abcde"
	virtual, err := idmap.StoreIDv2(nodeUserOpenID)
	if err != nil {
		t.Fatalf("StoreIDv2 失败: %v", err)
	}
	virtualID := strconv.FormatInt(virtual, 10)

	mock := &forwardRecMock{}
	client := &mdGateTestClient{}

	nodes := []interface{}{
		map[string]interface{}{"type": "node", "data": map[string]interface{}{
			"uin":  "10001",
			"name": "甲",
			"content": []interface{}{
				textSegment("第一段"),
				keyboardRowsSegment(keyboard.PermissionTypeSpecifyUserIDs, []string{virtualID}),
			},
		}},
		map[string]interface{}{"type": "node", "data": map[string]interface{}{
			"uin":     float64(10002),
			"name":    "乙",
			"content": "第二段",
		}},
	}

	msg := callapi.ActionMessage{
		Action: "send_group_forward_msg",
		Params: callapi.ParamsContent{GroupID: openID32('g'), Messages: nodes},
	}
	if _, err := HandleSendGroupForwardMsg(client, nil, mock, msg); err != nil {
		t.Fatalf("HandleSendGroupForwardMsg 返回错误: %v", err)
	}
	if len(mock.groupMsgs) != 2 {
		t.Fatalf("两节点应各下发一条群消息, got %d", len(mock.groupMsgs))
	}

	first, ok := mock.groupMsgs[0].(*dto.MessageToCreate)
	if !ok {
		t.Fatalf("第一条应为 MessageToCreate, got %T", mock.groupMsgs[0])
	}
	if !strings.Contains(first.Content, "甲(10001)") {
		t.Errorf("节点 uin/name 未保留: content=%q", first.Content)
	}
	if first.Keyboard == nil || first.Keyboard.Content == nil || len(first.Keyboard.Content.Rows) == 0 {
		t.Fatalf("节点内 keyboard 未下发: %+v", first.Keyboard)
	}
	perm := first.Keyboard.Content.Rows[0].Buttons[0].Action.Permission
	if perm == nil || len(perm.SpecifyUserIDs) != 1 || perm.SpecifyUserIDs[0] != nodeUserOpenID {
		t.Fatalf("specify_user_ids 未转 OpenID: %+v, want [%s]", perm, nodeUserOpenID)
	}
	if perm.Type != keyboard.PermissionTypeSpecifyUserIDs {
		t.Errorf("群聊应保留插件原始 permission.type, got %d", perm.Type)
	}

	second, ok := mock.groupMsgs[1].(*dto.MessageToCreate)
	if !ok {
		t.Fatalf("第二条应为 MessageToCreate, got %T", mock.groupMsgs[1])
	}
	if !strings.Contains(second.Content, "乙(10002)") {
		t.Errorf("数值形态 uin 未保留: content=%q", second.Content)
	}
	if second.Keyboard != nil {
		t.Errorf("无 keyboard 的节点不应凭空带上键盘: %+v", second.Keyboard)
	}
}

// ---------- G7：官方出站不支持的 CQ 码 ----------

// TestUnsupportedCQCodesStrippedPipeline legacy 管道：contact/poke/json/forward 剥离且正文保留。
func TestUnsupportedCQCodesStrippedPipeline(t *testing.T) {
	cases := []struct{ name, code string }{
		{"contact", "[CQ:contact,type=qq,id=10001]"},
		{"poke", "[CQ:poke,qq=10086]"},
		{"json", `[CQ:json,data={"app":"com.tencent.miniapp"}]`},
		{"forward", "[CQ:forward,id=abc123]"},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			drainMyLog()
			found := make(map[string][]string)
			got := ProcessCQCodePipeline("前"+c.code+"后", found, nil)
			if got != "前后" {
				t.Errorf("码未剥离: got %q, want 前后", got)
			}
			if len(found) != 0 {
				t.Errorf("不应产出 foundItems: %v", found)
			}
			logs := strings.Join(drainMyLog(), "\n")
			if !strings.Contains(logs, "[CQ:"+c.name+"]") {
				t.Errorf("缺少剥离 warn 日志: %q", logs)
			}
		})
	}
}

// TestUnsupportedCQCodesStrippedCQParse 新解析器（cqparse）同口径：剥离 + warn，字面量不泄漏。
func TestUnsupportedCQCodesStrippedCQParse(t *testing.T) {
	cases := []struct{ name, code string }{
		{"contact", "[CQ:contact,type=qq,id=10001]"},
		{"poke", "[CQ:poke,qq=10086]"},
		{"json", `[CQ:json,data={"app":"com.tencent.miniapp"}]`},
		{"forward", "[CQ:forward,id=abc123]"},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			drainMyLog()
			text, items, pendings := regParse(t, regStrIn("前"+c.code+"后"), regDeps(nil, nil))
			if text != "前后" {
				t.Errorf("码未剥离: got %q, want 前后", text)
			}
			if len(items) != 0 || len(pendings) != 0 {
				t.Errorf("不应产出 foundItems/pendings: items=%v pendings=%v", items, pendings)
			}
			logs := strings.Join(drainMyLog(), "\n")
			if !strings.Contains(logs, "[CQ:"+c.name+"]") {
				t.Errorf("缺少剥离 warn 日志: %q", logs)
			}
		})
	}
}

// TestUnsupportedCQCodesKeepKnownCodes 相邻标准码不受影响（at/face 仍原样保留）。
func TestUnsupportedCQCodesKeepKnownCodes(t *testing.T) {
	found := make(map[string][]string)
	got := ProcessCQCodePipeline("[CQ:face,id=1]表情[CQ:poke,qq=1]", found, nil)
	if got != "[CQ:face,id=1]表情" {
		t.Errorf("got %q, want [CQ:face,id=1]表情", got)
	}
}

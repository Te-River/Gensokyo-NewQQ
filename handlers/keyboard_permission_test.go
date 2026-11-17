package handlers

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/dto/keyboard"
)

// kbWithPermission 构造只含一个按钮的 keyboard，便于断言 permission 归一结果。
func kbWithPermission(pt keyboard.PermissionType, userIDs []string) *keyboard.MessageKeyboard {
	return &keyboard.MessageKeyboard{
		Content: &keyboard.CustomKeyboard{
			Rows: []*keyboard.Row{
				{Buttons: []*keyboard.Button{
					{Action: &keyboard.Action{Permission: &keyboard.Permission{Type: pt, SpecifyUserIDs: userIDs}}},
				}},
			},
		},
	}
}

// keyboardRows 构造 keyboard 的 content 载荷：单行单按钮，权限类型与指定用户可参数化。
func keyboardRows(pt keyboard.PermissionType, userIDs []string) map[string]interface{} {
	ids := make([]interface{}, 0, len(userIDs))
	for _, id := range userIDs {
		ids = append(ids, id)
	}
	return map[string]interface{}{
		"rows": []interface{}{
			map[string]interface{}{
				"buttons": []interface{}{
					map[string]interface{}{
						"render_data": map[string]interface{}{"label": "确认", "visited_label": "确认", "style": 1},
						"action": map[string]interface{}{
							"type": 2,
							"permission": map[string]interface{}{
								"type":             int(pt),
								"specify_user_ids": ids,
							},
							"data": "confirm",
						},
					},
				},
			},
		},
	}
}

// mdSegmentWithKeyboard 构造 markdown 段内嵌 keyboard（qbind 等插件的真实形态）。
func mdSegmentWithKeyboard(pt keyboard.PermissionType, userIDs []string) map[string]interface{} {
	return map[string]interface{}{
		"type": "markdown",
		"data": map[string]interface{}{
			"data": map[string]interface{}{
				"markdown": map[string]interface{}{"content": "### 确认绑定"},
				"keyboard": map[string]interface{}{"content": keyboardRows(pt, userIDs)},
			},
		},
	}
}

// keyboardRowsSegment 构造独立 [CQ:keyboard]（段形态，带自定义按钮权限）。
func keyboardRowsSegment(pt keyboard.PermissionType, userIDs []string) map[string]interface{} {
	return map[string]interface{}{
		"type": "keyboard",
		"data": map[string]interface{}{
			"data": map[string]interface{}{"content": keyboardRows(pt, userIDs)},
		},
	}
}

// mdFoundItems 构造 foundItems["markdown"]：base64 编码的 markdown + 内嵌 keyboard。
func mdFoundItems(pt keyboard.PermissionType, userIDs []string) map[string][]string {
	raw, _ := json.Marshal(map[string]interface{}{
		"markdown": map[string]interface{}{"content": "### 确认绑定"},
		"keyboard": map[string]interface{}{"content": keyboardRows(pt, userIDs)},
	})
	return map[string][]string{"markdown": {base64.StdEncoding.EncodeToString(raw)}}
}

// TestNormalizeC2CKeyboardPermissionAllTypes C2C 单聊只支持 permission.type=2：
// 指定用户（0，含空名单）/仅管理者（1）/指定身份组（3）一律归一为 2，type=2 保持原样。
func TestNormalizeC2CKeyboardPermissionAllTypes(t *testing.T) {
	cases := []struct {
		name    string
		typ     keyboard.PermissionType
		userIDs []string
	}{
		{"type0带名单", keyboard.PermissionTypeSpecifyUserIDs, []string{"openid-a"}},
		{"type0空名单", keyboard.PermissionTypeSpecifyUserIDs, nil},
		{"type1仅管理者", keyboard.PermissionTypManager, nil},
		{"type2所有人", keyboard.PermissionTypAll, nil},
		{"type3指定身份组", keyboard.PermissionTypSpecifyRoleIDs, nil},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			kb := kbWithPermission(c.typ, c.userIDs)
			NormalizeC2CKeyboardPermission(kb)
			perm := kb.Content.Rows[0].Buttons[0].Action.Permission
			if perm.Type != keyboard.PermissionTypAll {
				t.Errorf("Permission.Type = %d, want 2", perm.Type)
			}
			if c.userIDs != nil && len(perm.SpecifyUserIDs) != len(c.userIDs) {
				t.Errorf("SpecifyUserIDs 应原样保留, got %v", perm.SpecifyUserIDs)
			}
		})
	}
}

// TestResolvePlaceholderUserIDsKeepsPermissionType 占位符解析为群聊/私聊共用能力：
// 只替换 __USER_ID__，不得改动插件原始 permission.type（群聊的合法能力）。
func TestResolvePlaceholderUserIDsKeepsPermissionType(t *testing.T) {
	kb := kbWithPermission(keyboard.PermissionTypeSpecifyUserIDs, []string{"__USER_ID__"})
	ResolvePlaceholderUserIDs(kb, "real-openid")
	perm := kb.Content.Rows[0].Buttons[0].Action.Permission
	if len(perm.SpecifyUserIDs) != 1 || perm.SpecifyUserIDs[0] != "real-openid" {
		t.Errorf("SpecifyUserIDs = %v, want [real-openid]", perm.SpecifyUserIDs)
	}
	if perm.Type != keyboard.PermissionTypeSpecifyUserIDs {
		t.Errorf("Permission.Type = %d, want 0（群聊保留插件原始类型）", perm.Type)
	}
}

// TestSendPrivateMsgEmbeddedKeyboardPermissionType3 私聊 markdown 段内嵌 keyboard 的 type=3
// （指定身份组，仅频道可用）应归一为 type=2，specify_user_ids 保留。
func TestSendPrivateMsgEmbeddedKeyboardPermissionType3(t *testing.T) {
	mock := &mockMDSendOpenAPI{}
	client := &mdGateTestClient{}

	msg := callapi.ActionMessage{
		Action: "send_private_msg",
		Params: callapi.ParamsContent{
			UserID:  openID32('u'),
			Message: []interface{}{mdSegmentWithKeyboard(keyboard.PermissionTypSpecifyRoleIDs, []string{"openid-a"})},
		},
		Echo: "kb-c2c-embedded-type3",
	}

	if _, err := HandleSendPrivateMsg(client, nil, mock, msg); err != nil {
		t.Fatalf("HandleSendPrivateMsg 返回错误: %v", err)
	}
	if mock.c2cCalls != 1 {
		t.Fatalf("内嵌 keyboard 私聊消息应调用一次 PostC2CMessage, got %d", mock.c2cCalls)
	}
	mtc, ok := mock.lastC2C.(*dto.MessageToCreate)
	if !ok {
		t.Fatalf("应发送 MessageToCreate, got %T", mock.lastC2C)
	}
	if mtc.Keyboard == nil || mtc.Keyboard.Content == nil || len(mtc.Keyboard.Content.Rows) == 0 || len(mtc.Keyboard.Content.Rows[0].Buttons) == 0 {
		t.Fatalf("内嵌 Keyboard 应解析出 rows/buttons, got %+v", mtc.Keyboard)
	}
	perm := mtc.Keyboard.Content.Rows[0].Buttons[0].Action.Permission
	if perm == nil || perm.Type != keyboard.PermissionTypAll {
		t.Errorf("Permission = %+v, want type=2（C2C 不支持 type=3）", perm)
	}
	if len(perm.SpecifyUserIDs) == 0 {
		t.Error("SpecifyUserIDs 应保留非空")
	}
}

// TestSendPrivateMsgStandaloneKeyboardPermissionType1 私聊独立 [CQ:keyboard] 的 type=1
// （仅管理者）应归一为 type=2。
func TestSendPrivateMsgStandaloneKeyboardPermissionType1(t *testing.T) {
	mock := &mockMDSendOpenAPI{}
	client := &mdGateTestClient{}

	msg := callapi.ActionMessage{
		Action: "send_private_msg",
		Params: callapi.ParamsContent{
			UserID:  openID32('u'),
			Message: []interface{}{mdSegment("### 独立键盘"), keyboardRowsSegment(keyboard.PermissionTypManager, []string{"openid-a"})},
		},
		Echo: "kb-c2c-standalone-type1",
	}

	if _, err := HandleSendPrivateMsg(client, nil, mock, msg); err != nil {
		t.Fatalf("HandleSendPrivateMsg 返回错误: %v", err)
	}
	if mock.c2cCalls != 1 {
		t.Fatalf("独立 keyboard 私聊消息应调用一次 PostC2CMessage, got %d", mock.c2cCalls)
	}
	mtc, ok := mock.lastC2C.(*dto.MessageToCreate)
	if !ok {
		t.Fatalf("应发送 MessageToCreate, got %T", mock.lastC2C)
	}
	if mtc.Keyboard == nil || mtc.Keyboard.Content == nil || len(mtc.Keyboard.Content.Rows) == 0 || len(mtc.Keyboard.Content.Rows[0].Buttons) == 0 {
		t.Fatalf("独立 Keyboard 应附加到消息, got %+v", mtc.Keyboard)
	}
	perm := mtc.Keyboard.Content.Rows[0].Buttons[0].Action.Permission
	if perm == nil || perm.Type != keyboard.PermissionTypAll {
		t.Errorf("Permission = %+v, want type=2（C2C 不支持 type=1）", perm)
	}
	if len(perm.SpecifyUserIDs) == 0 {
		t.Error("SpecifyUserIDs 应保留非空")
	}
}

// TestGeneratePrivateMessageKeyboardPermissionType 覆盖 generatePrivateMessage 的 markdown 分支
// （send_private_msg_wakeup 等路径共用）：type=0 带名单应归一为 type=2。
func TestGeneratePrivateMessageKeyboardPermissionType(t *testing.T) {
	mock := &mockMDSendOpenAPI{}
	found := mdFoundItems(keyboard.PermissionTypeSpecifyUserIDs, []string{"openid-a"})

	reply := generatePrivateMessage("mid-1", "eid-1", found, "", 1, mock, "user-openid")
	mtc, ok := reply.(*dto.MessageToCreate)
	if !ok {
		t.Fatalf("应返回 MessageToCreate, got %T", reply)
	}
	if mtc.Keyboard == nil || mtc.Keyboard.Content == nil || len(mtc.Keyboard.Content.Rows) == 0 || len(mtc.Keyboard.Content.Rows[0].Buttons) == 0 {
		t.Fatalf("Keyboard 应解析出 rows/buttons, got %+v", mtc.Keyboard)
	}
	perm := mtc.Keyboard.Content.Rows[0].Buttons[0].Action.Permission
	if perm == nil || perm.Type != keyboard.PermissionTypAll {
		t.Errorf("Permission = %+v, want type=2（该分支只构造 C2C 消息）", perm)
	}
}

// TestSendGroupMsgEmbeddedKeyboardKeepsPermissionType 群聊 markdown 段内嵌 keyboard：
// permission.type=0 是群聊合法能力，必须原样保留（不得误转 type=2）。
func TestSendGroupMsgEmbeddedKeyboardKeepsPermissionType(t *testing.T) {
	mock := &mockMDSendOpenAPI{}
	client := &mdGateTestClient{}

	msg := callapi.ActionMessage{
		Action: "send_group_msg",
		Params: callapi.ParamsContent{
			GroupID: openID32('g'),
			UserID:  openID32('u'),
			Message: []interface{}{mdSegmentWithKeyboard(keyboard.PermissionTypeSpecifyUserIDs, []string{"openid-a"})},
		},
		Echo: "kb-group-embedded-type0",
	}

	if _, err := HandleSendGroupMsg(client, nil, mock, msg); err != nil {
		t.Fatalf("HandleSendGroupMsg 返回错误: %v", err)
	}
	if mock.groupCalls != 1 {
		t.Fatalf("内嵌 keyboard 群消息应调用一次 PostGroupMessage, got %d", mock.groupCalls)
	}
	mtc, ok := mock.lastGroup.(*dto.MessageToCreate)
	if !ok {
		t.Fatalf("应发送 MessageToCreate, got %T", mock.lastGroup)
	}
	if mtc.Keyboard == nil || mtc.Keyboard.Content == nil || len(mtc.Keyboard.Content.Rows) == 0 || len(mtc.Keyboard.Content.Rows[0].Buttons) == 0 {
		t.Fatalf("内嵌 Keyboard 应解析出 rows/buttons, got %+v", mtc.Keyboard)
	}
	perm := mtc.Keyboard.Content.Rows[0].Buttons[0].Action.Permission
	if perm == nil || perm.Type != keyboard.PermissionTypeSpecifyUserIDs {
		t.Errorf("Permission = %+v, want type=0（群聊保留插件原始类型）", perm)
	}
	if len(perm.SpecifyUserIDs) != 1 || perm.SpecifyUserIDs[0] != "openid-a" {
		t.Errorf("SpecifyUserIDs = %v, want [openid-a]", perm.SpecifyUserIDs)
	}
}

// TestSendFoundItemsGroupKeepsKeyboardPermissionType 新链群聊（applyGroupMarkdownKeyboard）：
// permission.type=0 保持不动，不做 C2C 归一化。
func TestSendFoundItemsGroupKeepsKeyboardPermissionType(t *testing.T) {
	mock := &mockMDSendOpenAPI{}
	found := mdFoundItems(keyboard.PermissionTypeSpecifyUserIDs, []string{"openid-a"})

	if _, err := SendFoundItemsGroup(context.Background(), "group-openid", "", found, mock); err != nil {
		t.Fatalf("SendFoundItemsGroup 返回错误: %v", err)
	}
	if mock.groupCalls != 1 {
		t.Fatalf("md 群消息应调用一次 PostGroupMessage, got %d", mock.groupCalls)
	}
	mtc, ok := mock.lastGroup.(*dto.MessageToCreate)
	if !ok {
		t.Fatalf("应发送 MessageToCreate, got %T", mock.lastGroup)
	}
	if mtc.Keyboard == nil || mtc.Keyboard.Content == nil || len(mtc.Keyboard.Content.Rows) == 0 || len(mtc.Keyboard.Content.Rows[0].Buttons) == 0 {
		t.Fatalf("Keyboard 应解析出 rows/buttons, got %+v", mtc.Keyboard)
	}
	perm := mtc.Keyboard.Content.Rows[0].Buttons[0].Action.Permission
	if perm == nil || perm.Type != keyboard.PermissionTypeSpecifyUserIDs {
		t.Errorf("Permission = %+v, want type=0（群聊保留插件原始类型）", perm)
	}
}

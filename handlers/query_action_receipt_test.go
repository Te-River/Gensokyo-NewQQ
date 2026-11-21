package handlers

// 回归（v016）：查询类 action 在 id 反查失败 / 参数错误时同样必须回执。
// 与 send_action_receipt_test.go 同因：裸 return "", nil 时反向 WS 客户端收不到
// 任何 action 回执，只能等超时断开。

import (
	"testing"

	"github.com/hoshinonyaruko/gensokyo/callapi"
)

// TestGetGroupMemberListUnmappedGroupIDReplies 未映射的 group_id → 回执 1400，不再静默返回。
func TestGetGroupMemberListUnmappedGroupIDReplies(t *testing.T) {
	client := &contractTestClient{}
	msg := callapi.ActionMessage{
		Action: "get_group_member_list",
		Params: callapi.ParamsContent{GroupID: "9600000001"},
		Echo:   "echo-unmapped-member-list",
	}

	raw, err := GetGroupMemberList(client, nil, nil, msg)
	if err != nil {
		t.Fatalf("GetGroupMemberList 返回错误: %v", err)
	}
	assertFailedReceipt(t, client, raw, sendParamRetCode, "echo-unmapped-member-list")
}

// TestGetGroupInfoUnmappedGroupIDReplies 未映射的 group_id → 仍下发失败回执（历史口径 retcode 100）。
func TestGetGroupInfoUnmappedGroupIDReplies(t *testing.T) {
	client := &contractTestClient{}
	msg := callapi.ActionMessage{
		Action: "get_group_info",
		Params: callapi.ParamsContent{GroupID: "9600000002"},
		Echo:   "echo-unmapped-group-info",
	}

	raw, err := HandleGetGroupInfo(client, nil, nil, msg)
	if err != nil {
		t.Fatalf("HandleGetGroupInfo 返回错误: %v", err)
	}
	if client.last == nil {
		t.Fatalf("客户端未收到任何 SendMessage 回执（静默无响应）: raw=%s", raw)
	}
	// sendGroupInfoError 沿用历史口径 100（见 set_group_add_request.go 常量注释）
	if client.last["retcode"] != float64(100) || client.last["status"] != "failed" {
		t.Errorf("回执头不符: %v", client.last)
	}
	if client.last["echo"] != "echo-unmapped-group-info" {
		t.Errorf("回执 echo = %v, want echo-unmapped-group-info", client.last["echo"])
	}
	if raw == "" {
		t.Error("返回体为空，HTTP API 模式下客户端同样收不到响应")
	}
}

// TestGetAvatarUnmappedUserIDReplies 未映射的 user_id → 回执 1400，不再返回空 openid 的头像 URL。
func TestGetAvatarUnmappedUserIDReplies(t *testing.T) {
	client := &contractTestClient{}
	msg := callapi.ActionMessage{
		Action: "get_avatar",
		Params: callapi.ParamsContent{GroupID: "9600000004", UserID: "9600000003"},
		Echo:   "echo-unmapped-avatar",
	}

	raw, err := GetAvatar(client, nil, nil, msg)
	if err != nil {
		t.Fatalf("GetAvatar 返回错误: %v", err)
	}
	assertFailedReceipt(t, client, raw, sendParamRetCode, "echo-unmapped-avatar")
}

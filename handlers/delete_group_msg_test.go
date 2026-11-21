package handlers

import (
	"fmt"
	"reflect"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/tencent-connect/botgo/openapi"
)

type deleteGroupMsgTestClient struct {
	response map[string]interface{}
}

func (client *deleteGroupMsgTestClient) SendMessage(message map[string]interface{}) error {
	client.response = message
	return nil
}

func TestDeleteGroupMsgFindsSpecifiedUsersLatestMessage(t *testing.T) {
	restore := stubDeleteGroupMsgDependencies(t)
	defer restore()

	deleteGroupMsgResolveOriginalID = func(value string) string {
		return map[string]string{
			"100": "GROUP-OPENID",
			"200": "USER-OPENID",
		}[value]
	}
	deleteGroupMsgGetLatestUser = func(groupOpenID, userOpenID string) (string, error) {
		if groupOpenID != "GROUP-OPENID" || userOpenID != "USER-OPENID" {
			t.Fatalf("latest-user lookup = (%q, %q)", groupOpenID, userOpenID)
		}
		return "USER-LAST-MESSAGE", nil
	}

	var retractedGroup, retractedMessage string
	deleteGroupMsgRetract = func(api openapi.OpenAPI, groupOpenID, messageID string) error {
		retractedGroup = groupOpenID
		retractedMessage = messageID
		return nil
	}

	client := &deleteGroupMsgTestClient{}
	result, err := DeleteGroupMsg(client, nil, nil, callapi.ActionMessage{
		Action: "delete_group_msg",
		Params: callapi.ParamsContent{
			GroupID: "100",
			UserID:  "200",
		},
		Echo: "test-user-last",
	})
	if err != nil {
		t.Fatalf("DeleteGroupMsg: %v", err)
	}
	if retractedGroup != "GROUP-OPENID" || retractedMessage != "USER-LAST-MESSAGE" {
		t.Fatalf("retracted (%q, %q)", retractedGroup, retractedMessage)
	}
	if result == "" || client.response["status"] != "ok" {
		t.Fatalf("unexpected response: result=%q map=%v", result, client.response)
	}
}

func TestDeleteGroupMsgNonPositiveUserFindsBotsLatestMessage(t *testing.T) {
	restore := stubDeleteGroupMsgDependencies(t)
	defer restore()

	deleteGroupMsgResolveOriginalID = func(value string) string {
		if value == "100" {
			return "GROUP-OPENID"
		}
		return value
	}
	deleteGroupMsgGetLatestBot = func(groupOpenID string) (string, error) {
		if groupOpenID != "GROUP-OPENID" {
			t.Fatalf("latest-bot lookup group = %q", groupOpenID)
		}
		return "BOT-LAST-MESSAGE", nil
	}

	var retractedMessage string
	deleteGroupMsgRetract = func(api openapi.OpenAPI, groupOpenID, messageID string) error {
		retractedMessage = messageID
		return nil
	}

	client := &deleteGroupMsgTestClient{}
	_, err := DeleteGroupMsg(client, nil, nil, callapi.ActionMessage{
		Action: "delete_group_msg",
		Params: callapi.ParamsContent{
			GroupID: "100",
			UserID:  "-1",
		},
	})
	if err != nil {
		t.Fatalf("DeleteGroupMsg: %v", err)
	}
	if retractedMessage != "BOT-LAST-MESSAGE" {
		t.Fatalf("retracted message = %q, want BOT-LAST-MESSAGE", retractedMessage)
	}
	if client.response["status"] != "ok" {
		t.Fatalf("unexpected response: %v", client.response)
	}
}

func TestDeleteGroupMsgExplicitMessageDoesNotNeedPositiveUser(t *testing.T) {
	restore := stubDeleteGroupMsgDependencies(t)
	defer restore()

	deleteGroupMsgResolveOriginalID = func(value string) string {
		return value
	}
	deleteGroupMsgGetLatestBot = func(groupOpenID string) (string, error) {
		return "", fmt.Errorf("latest bot lookup should not be called")
	}

	var got []string
	deleteGroupMsgRetract = func(api openapi.OpenAPI, groupOpenID, messageID string) error {
		got = []string{groupOpenID, messageID}
		return nil
	}

	client := &deleteGroupMsgTestClient{}
	_, err := DeleteGroupMsg(client, nil, nil, callapi.ActionMessage{
		Action: "delete_group_msg",
		Params: callapi.ParamsContent{
			GroupID:   "GROUP-OPENID",
			UserID:    "0",
			MessageID: "ROBOT1.0_REAL-MESSAGE-ID",
		},
	})
	if err != nil {
		t.Fatalf("DeleteGroupMsg: %v", err)
	}
	want := []string{"GROUP-OPENID", "ROBOT1.0_REAL-MESSAGE-ID"}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("retract args = %v, want %v", got, want)
	}
}

func TestResolveDeleteGroupMsgTargetDefaultsMissingUserToBot(t *testing.T) {
	restore := stubDeleteGroupMsgDependencies(t)
	defer restore()

	deleteGroupMsgResolveOriginalID = func(value string) string {
		return value
	}

	target, err := resolveDeleteGroupMsgTarget(callapi.ParamsContent{
		GroupID:   "GROUP-OPENID",
		MessageID: "ROBOT1.0_REAL-MESSAGE-ID",
	})
	if err != nil {
		t.Fatalf("resolveDeleteGroupMsgTarget: %v", err)
	}
	if !target.BotMessage || target.UserOpenID != "" {
		t.Fatalf("missing user did not select Bot message: %#v", target)
	}
}

func TestDeleteGroupMsgRejectsMissingGroupID(t *testing.T) {
	restore := stubDeleteGroupMsgDependencies(t)
	defer restore()

	retractCalled := false
	deleteGroupMsgRetract = func(api openapi.OpenAPI, groupOpenID, messageID string) error {
		retractCalled = true
		return nil
	}

	client := &deleteGroupMsgTestClient{}
	result, err := DeleteGroupMsg(client, nil, nil, callapi.ActionMessage{
		Action: "delete_group_msg",
		Params: callapi.ParamsContent{
			UserID: "200",
		},
	})
	if err != nil {
		t.Fatalf("DeleteGroupMsg response: %v", err)
	}
	if retractCalled {
		t.Fatal("retract called without group_id")
	}
	if result == "" || client.response["status"] != "failed" {
		t.Fatalf("unexpected failure response: result=%q map=%v", result, client.response)
	}
}

func stubDeleteGroupMsgDependencies(t *testing.T) func() {
	t.Helper()
	originalResolve := deleteGroupMsgResolveOriginalID
	originalLatestUser := deleteGroupMsgGetLatestUser
	originalLatestBot := deleteGroupMsgGetLatestBot
	originalRetract := deleteGroupMsgRetract
	originalStringOb11 := deleteGroupMsgGetStringOb11

	deleteGroupMsgGetStringOb11 = func() bool { return false }
	deleteGroupMsgGetLatestUser = func(groupOpenID, userOpenID string) (string, error) {
		return "", fmt.Errorf("unexpected user latest-message lookup")
	}
	deleteGroupMsgGetLatestBot = func(groupOpenID string) (string, error) {
		return "", fmt.Errorf("unexpected bot latest-message lookup")
	}

	return func() {
		deleteGroupMsgResolveOriginalID = originalResolve
		deleteGroupMsgGetLatestUser = originalLatestUser
		deleteGroupMsgGetLatestBot = originalLatestBot
		deleteGroupMsgRetract = originalRetract
		deleteGroupMsgGetStringOb11 = originalStringOb11
	}
}

// TestDeleteGroupMsgStringOb11SkipsMessageIDLookup string_ob11 下 msg_id 就是真实 ID：
// 不得按虚拟号反查（原来会因查不到映射而撤回失败），与 delete_msg 同口径。
func TestDeleteGroupMsgStringOb11SkipsMessageIDLookup(t *testing.T) {
	restore := stubDeleteGroupMsgDependencies(t)
	defer restore()

	deleteGroupMsgGetStringOb11 = func() bool { return true }
	deleteGroupMsgResolveOriginalID = func(value string) string {
		if value == "100" {
			return "GROUP-OPENID"
		}
		return value
	}

	var retractedGroup, retractedMessage string
	deleteGroupMsgRetract = func(api openapi.OpenAPI, groupOpenID, messageID string) error {
		retractedGroup, retractedMessage = groupOpenID, messageID
		return nil
	}

	client := &deleteGroupMsgTestClient{}
	_, err := DeleteGroupMsg(client, nil, nil, callapi.ActionMessage{
		Action: "delete_group_msg",
		Params: callapi.ParamsContent{
			GroupID:   "100",
			UserID:    "0",
			MessageID: "1234567890",
		},
	})
	if err != nil {
		t.Fatalf("DeleteGroupMsg: %v", err)
	}
	if retractedGroup != "GROUP-OPENID" || retractedMessage != "1234567890" {
		t.Fatalf("retract args = (%q, %q), want (GROUP-OPENID, 1234567890)", retractedGroup, retractedMessage)
	}
	if client.response["status"] != "ok" {
		t.Fatalf("string_ob11 下应撤回成功: %v", client.response)
	}
}

// TestDeleteGroupMsgNonStringOb11ResolvesMessageID 非 string_ob11 仍按虚拟号反查真实 message_id。
func TestDeleteGroupMsgNonStringOb11ResolvesMessageID(t *testing.T) {
	restore := stubDeleteGroupMsgDependencies(t)
	defer restore()

	const realMessageID = "ROBOT1.0_DELETE_GROUP_STRING_OB11"
	deleteGroupMsgResolveOriginalID = func(value string) string {
		if value == "100" {
			return "GROUP-OPENID"
		}
		return value
	}

	var retractedMessage string
	deleteGroupMsgRetract = func(api openapi.OpenAPI, groupOpenID, messageID string) error {
		retractedMessage = messageID
		return nil
	}

	client := &deleteGroupMsgTestClient{}
	_, err := DeleteGroupMsg(client, nil, nil, callapi.ActionMessage{
		Action: "delete_group_msg",
		Params: callapi.ParamsContent{
			GroupID:   "100",
			UserID:    "0",
			MessageID: deleteMsgMessageID(t, realMessageID),
		},
	})
	if err != nil {
		t.Fatalf("DeleteGroupMsg: %v", err)
	}
	if retractedMessage != realMessageID {
		t.Fatalf("retract message_id = %q, want 反查后的 %q", retractedMessage, realMessageID)
	}
	if client.response["status"] != "ok" {
		t.Fatalf("unexpected response: %v", client.response)
	}
}

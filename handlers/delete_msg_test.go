package handlers

// 回归（v016 活体对照）：delete_msg 曾把虚拟群号/用户号直接当 group_openid 发给上游
// （上游回 404 11001 不支持的调用），且无论撤回成败都回 retcode:0 / status:"ok"。
// 本文件钉住：入参反查成 OpenID 后才调上游、缺目标回 1400、上游失败回 1500。

import (
	"context"
	"errors"
	"strconv"
	"strings"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/echo"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/tencent-connect/botgo/openapi"
)

const (
	deleteMsgGroupOpenID = "31AB82CE7526BBFE4B1002C4B258FD0E"
	deleteMsgUserOpenID  = "D8FB9A0A5F5E4C3B9A1B2C3D4E5F6071"
)

// deleteMsgTestAPI 记录撤回入参并按预设返回错误；其余方法走嵌入接口兜底。
type deleteMsgTestAPI struct {
	openapi.OpenAPI
	err        error
	groupID    string
	userID     string
	messageID  string
	groupCalls int
	c2cCalls   int
}

func (m *deleteMsgTestAPI) RetractGroupMessage(_ context.Context, groupID, msgID string, _ ...openapi.RetractMessageOption) error {
	m.groupCalls++
	m.groupID, m.messageID = groupID, msgID
	return m.err
}

func (m *deleteMsgTestAPI) RetractC2CMessage(_ context.Context, userID, msgID string, _ ...openapi.RetractMessageOption) error {
	m.c2cCalls++
	m.userID, m.messageID = userID, msgID
	return m.err
}

// deleteMsgMessageID 按当前配置把真实 message_id 换成 handler 期望的入参形态，
// 使 handler 的虚拟→真实反查必然还原为 realID。
func deleteMsgMessageID(t *testing.T, realID string) string {
	t.Helper()
	if config.GetStringOb11() {
		return realID
	}
	if config.GetMemoryMsgid() {
		virtual, err := echo.StoreCacheInMemory(realID)
		if err != nil {
			t.Fatalf("StoreCacheInMemory: %v", err)
		}
		return strconv.FormatInt(virtual, 10)
	}
	virtual, err := idmap.StoreCachev2(realID)
	if err != nil {
		t.Fatalf("StoreCachev2: %v", err)
	}
	return strconv.FormatInt(virtual, 10)
}

// deleteMsgVirtualID 预置 真实 OpenID → 虚拟号 映射并返回虚拟号入参。
func deleteMsgVirtualID(t *testing.T, openID string) string {
	t.Helper()
	virtual, err := idmap.StoreIDv2(openID)
	if err != nil {
		t.Fatalf("StoreIDv2(%s): %v", openID, err)
	}
	return strconv.FormatInt(virtual, 10)
}

func assertDeleteMsgOK(t *testing.T, client *contractTestClient, raw string, wantEcho string) {
	t.Helper()
	if client.last == nil {
		t.Fatalf("客户端未收到回执: raw=%s", raw)
	}
	if client.last["retcode"] != float64(0) || client.last["status"] != "ok" {
		t.Errorf("回执应为 retcode 0 / status ok: %v", client.last)
	}
	if data, ok := client.last["data"]; !ok || data != nil {
		t.Errorf("成功回执 data 应为 null（不得复用 get_status 的状态结构）: %v", data)
	}
	if client.last["echo"] != wantEcho {
		t.Errorf("回执 echo = %v, want %s", client.last["echo"], wantEcho)
	}
}

// TestDeleteMsgRequiresGroupOrUser 缺 group_id/user_id → 1400 / failed，且不触碰上游。
func TestDeleteMsgRequiresGroupOrUser(t *testing.T) {
	client := &contractTestClient{}
	api := &deleteMsgTestAPI{}

	raw, err := DeleteMsg(client, api, nil, callapi.ActionMessage{
		Action: "delete_msg",
		Params: callapi.ParamsContent{MessageID: "ROBOT1.0_DELETE_NO_TARGET"},
		Echo:   "del-no-target",
	})
	if err != nil {
		t.Fatalf("DeleteMsg: %v", err)
	}
	assertFailedReceipt(t, client, raw, sendParamRetCode, "del-no-target")
	if api.groupCalls != 0 || api.c2cCalls != 0 {
		t.Errorf("缺目标时不应调用撤回: group=%d c2c=%d", api.groupCalls, api.c2cCalls)
	}
	if msg, _ := client.last["message"].(string); !strings.Contains(msg, "group_id 或 user_id") {
		t.Errorf("message 应说明需要 group_id 或 user_id: %v", client.last["message"])
	}
}

// TestDeleteMsgGroupUsesResolvedOpenID 群撤回必须以反查后的 group_openid 调上游，且回执 data 不是状态结构。
func TestDeleteMsgGroupUsesResolvedOpenID(t *testing.T) {
	const realMsgID = "ROBOT1.0_DELETE_GROUP_MSG"
	groupID := deleteMsgVirtualID(t, deleteMsgGroupOpenID)
	api := &deleteMsgTestAPI{}
	client := &contractTestClient{}

	raw, err := DeleteMsg(client, api, nil, callapi.ActionMessage{
		Action: "delete_msg",
		Params: callapi.ParamsContent{GroupID: groupID, MessageID: deleteMsgMessageID(t, realMsgID)},
		Echo:   "del-group",
	})
	if err != nil {
		t.Fatalf("DeleteMsg: %v", err)
	}
	if api.groupCalls != 1 || api.c2cCalls != 0 {
		t.Fatalf("应只调用一次群撤回: group=%d c2c=%d", api.groupCalls, api.c2cCalls)
	}
	if api.groupID != deleteMsgGroupOpenID {
		t.Fatalf("RetractGroupMessage 群号 = %q（虚拟号被直接当 group_openid），want %q", api.groupID, deleteMsgGroupOpenID)
	}
	if api.messageID != realMsgID {
		t.Errorf("RetractGroupMessage message_id = %q, want 反查后的 %q", api.messageID, realMsgID)
	}
	assertDeleteMsgOK(t, client, raw, "del-group")
}

// TestDeleteMsgC2CUsesResolvedOpenID C2C 撤回同样必须用反查后的 user_openid。
func TestDeleteMsgC2CUsesResolvedOpenID(t *testing.T) {
	const realMsgID = "ROBOT1.0_DELETE_C2C_MSG"
	userID := deleteMsgVirtualID(t, deleteMsgUserOpenID)
	api := &deleteMsgTestAPI{}
	client := &contractTestClient{}

	raw, err := DeleteMsg(client, api, nil, callapi.ActionMessage{
		Action: "delete_msg",
		Params: callapi.ParamsContent{UserID: userID, MessageID: deleteMsgMessageID(t, realMsgID)},
		Echo:   "del-c2c",
	})
	if err != nil {
		t.Fatalf("DeleteMsg: %v", err)
	}
	if api.c2cCalls != 1 || api.groupCalls != 0 {
		t.Fatalf("应只调用一次 C2C 撤回: group=%d c2c=%d", api.groupCalls, api.c2cCalls)
	}
	if api.userID != deleteMsgUserOpenID {
		t.Fatalf("RetractC2CMessage 用户 = %q（虚拟号被直接当 openid），want %q", api.userID, deleteMsgUserOpenID)
	}
	if api.messageID != realMsgID {
		t.Errorf("RetractC2CMessage message_id = %q, want 反查后的 %q", api.messageID, realMsgID)
	}
	assertDeleteMsgOK(t, client, raw, "del-c2c")
}

// TestDeleteMsgRetractFailureRepliesFailed 上游撤回报错 → 1500 / failed，message 带真实错误。
func TestDeleteMsgRetractFailureRepliesFailed(t *testing.T) {
	upstreamErr := errors.New(`code:404, text:{"message":"不支持的调用","code":11001,"err_code":40011002}`)
	groupID := deleteMsgVirtualID(t, deleteMsgGroupOpenID)
	api := &deleteMsgTestAPI{err: upstreamErr}
	client := &contractTestClient{}

	raw, err := DeleteMsg(client, api, nil, callapi.ActionMessage{
		Action: "delete_msg",
		Params: callapi.ParamsContent{GroupID: groupID, MessageID: deleteMsgMessageID(t, "ROBOT1.0_DELETE_FAIL")},
		Echo:   "del-fail",
	})
	if err != nil {
		t.Fatalf("DeleteMsg: %v", err)
	}
	assertFailedReceipt(t, client, raw, sendFailRetCode, "del-fail")
	if msg, _ := client.last["message"].(string); !strings.Contains(msg, "11001") {
		t.Errorf("message 应带上游真实错误: %v", client.last["message"])
	}
}

// TestDeleteMsgUnmappedGroupRepliesParam 反查不出 OpenID 属参数/映射问题 → 1400，且不调上游。
func TestDeleteMsgUnmappedGroupRepliesParam(t *testing.T) {
	api := &deleteMsgTestAPI{}
	client := &contractTestClient{}

	raw, err := DeleteMsg(client, api, nil, callapi.ActionMessage{
		Action: "delete_msg",
		Params: callapi.ParamsContent{GroupID: "499967399", MessageID: "ROBOT1.0_DELETE_UNMAPPED"},
		Echo:   "del-unmapped",
	})
	if err != nil {
		t.Fatalf("DeleteMsg: %v", err)
	}
	assertFailedReceipt(t, client, raw, sendParamRetCode, "del-unmapped")
	if api.groupCalls != 0 {
		t.Errorf("反查失败不应调用撤回: group=%d", api.groupCalls)
	}
	if msg, _ := client.last["message"].(string); !strings.Contains(msg, "无法反查") {
		t.Errorf("message 应说明无法反查: %v", client.last["message"])
	}
}

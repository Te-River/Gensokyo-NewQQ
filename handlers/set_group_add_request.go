package handlers

import (
	"context"
	"encoding/json"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

func init() {
	callapi.RegisterHandler("set_group_add_request", SetGroupAddRequest)
}

// SetGroupAddRequest 审批入群申请
// params: group_id(虚拟/真实群ID), user_id(虚拟/真实用户ID), flag(join_request_id), approve(bool)
// 扩展参数: reason(拒绝理由), add_to_member_blacklist(是否同时拉黑)
func SetGroupAddRequest(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	groupID, _ := message.Params.GroupID.(string)
	userID, _ := message.Params.UserID.(string)

	// 反查真实 OpenID（32 位原生 OpenID 直接使用）
	groupOpenID := groupID
	if len(groupID) != 32 {
		realGroupID, err := idmap.RetrieveRowByIDv2(groupID)
		if err != nil || realGroupID == "" {
			mylog.Printf("set_group_add_request: 反查 group_openid 失败: %v", err)
			return sendGroupAddRequestResponse(client, message, "无法反查群 OpenID", 100)
		}
		groupOpenID = realGroupID
	}
	memberOpenID := userID
	if len(userID) != 32 {
		realUserID, err := idmap.RetrieveRowByIDv2(userID)
		if err != nil || realUserID == "" {
			mylog.Printf("set_group_add_request: 反查 member_openid 失败: %v", err)
			return sendGroupAddRequestResponse(client, message, "无法反查用户 OpenID", 100)
		}
		memberOpenID = realUserID
	}

	op := "decline"
	if message.Params.Approve {
		op = "approve"
	}

	req := &dto.ApprovalJoinRequest{
		Op:                   op,
		JoinRequestID:        message.Params.Flag,
		RejectReason:         message.Params.Reason,
		AddToMemberBlacklist: message.Params.AddToMemberBlacklist,
	}

	if err := apiv2.ApprovalJoinRequest(context.TODO(), groupOpenID, memberOpenID, req); err != nil {
		mylog.Printf("set_group_add_request: 审批失败: %v", err)
		return sendGroupAddRequestResponse(client, message, err.Error(), 100)
	}

	mylog.Printf("set_group_add_request: 审批成功 group=%s user=%s op=%s", groupOpenID, memberOpenID, op)
	return sendGroupAddRequestResponse(client, message, "", 0)
}

// sendGroupAddRequestResponse 构造并回传审批结果响应
func sendGroupAddRequestResponse(client callapi.Client, message callapi.ActionMessage, errMsg string, retCode int) (string, error) {
	status := "ok"
	if retCode != 0 {
		status = "failed"
	}
	response := struct {
		Message string      `json:"message"`
		RetCode int         `json:"retcode"`
		Status  string      `json:"status"`
		Echo    interface{} `json:"echo,omitempty"`
	}{
		Message: errMsg,
		RetCode: retCode,
		Status:  status,
		Echo:    message.Echo,
	}
	outputMap := structToMap(response)
	if client != nil {
		if err := client.SendMessage(outputMap); err != nil {
			mylog.Printf("set_group_add_request: 发送响应失败: %v", err)
		}
	}
	result, err := json.Marshal(response)
	if err != nil {
		mylog.Printf("set_group_add_request: 序列化响应失败: %v", err)
		return "", err
	}
	return string(result), nil
}

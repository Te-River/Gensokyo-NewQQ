package handlers

import (
	"context"
	"encoding/json"
	"strconv"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/mylog"

	"github.com/tencent-connect/botgo/openapi"
)

func init() {
	callapi.RegisterHandler("get_group_info", HandleGetGroupInfo)
}

type OnebotGroupInfo struct {
	Data    GroupInfo   `json:"data"`
	Message string      `json:"message"`
	RetCode int         `json:"retcode"`
	Status  string      `json:"status"`
	Echo    interface{} `json:"echo"`
}

type GroupInfo struct {
	GroupID         int64  `json:"group_id"`
	GroupName       string `json:"group_name"`
	GroupMemo       string `json:"group_memo"`
	GroupCreateTime int32  `json:"group_create_time"`
	GroupLevel      int32  `json:"group_level"`
	MemberCount     int32  `json:"member_count"`
	MaxMemberCount  int32  `json:"max_member_count"`
}

func HandleGetGroupInfo(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	groupID := message.Params.GroupID.(string)

	// 反查真实 OpenID（32 位原生 OpenID 直接使用）
	groupOpenID := groupID
	if len(groupID) != 32 {
		realGroupID, err := idmap.RetrieveRowByIDv2(groupID)
		if err != nil || realGroupID == "" {
			mylog.Printf("get_group_info: 反查 group_openid 失败: %v", err)
			return sendGroupInfoError(client, message, "无法反查群 OpenID")
		}
		groupOpenID = realGroupID
	}

	info, err := apiv2.GroupInfo(context.TODO(), groupOpenID)
	if err != nil {
		mylog.Printf("get_group_info: 获取群信息失败: %v", err)
		return sendGroupInfoError(client, message, err.Error())
	}

	groupid, _ := strconv.ParseInt(groupID, 10, 64)
	groupInfo := &OnebotGroupInfo{
		Data: GroupInfo{
			GroupID:     groupid,
			GroupName:   info.GroupName,
			GroupMemo:   info.GroupFingerMemo,
			MemberCount: int32(info.GroupMemberNum),
		},
		Message: "success",
		RetCode: 0,
		Status:  "ok",
	}
	if message.Echo == "" {
		groupInfo.Echo = "0"
	} else {
		groupInfo.Echo = message.Echo
	}

	groupInfoMap := structToMap(groupInfo)
	mylog.Printf("get_group_info: group=%s name=%s members=%d", groupOpenID, info.GroupName, info.GroupMemberNum)

	if client != nil {
		if err := client.SendMessage(groupInfoMap); err != nil {
			mylog.Printf("get_group_info: 发送响应失败: %v", err)
		}
	}
	result, err := json.Marshal(groupInfo)
	if err != nil {
		mylog.Printf("get_group_info: 序列化响应失败: %v", err)
		return "", nil
	}
	return string(result), nil
}

// sendGroupInfoError 构造并回传错误响应
func sendGroupInfoError(client callapi.Client, message callapi.ActionMessage, errMsg string) (string, error) {
	response := struct {
		Message string      `json:"message"`
		RetCode int         `json:"retcode"`
		Status  string      `json:"status"`
		Echo    interface{} `json:"echo,omitempty"`
	}{
		Message: errMsg,
		RetCode: 100,
		Status:  "failed",
		Echo:    message.Echo,
	}
	outputMap := structToMap(response)
	if client != nil {
		if err := client.SendMessage(outputMap); err != nil {
			mylog.Printf("get_group_info: 发送错误响应失败: %v", err)
		}
	}
	result, err := json.Marshal(response)
	if err != nil {
		return "", err
	}
	return string(result), nil
}

// 将结构体转换为 map[string]interface{}
func structToMap(obj interface{}) map[string]interface{} {
	out := make(map[string]interface{})
	j, _ := json.Marshal(obj)
	json.Unmarshal(j, &out)
	return out
}

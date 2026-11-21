package handlers

import (
	"context"
	"encoding/json"
	"fmt"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/openapi"
)

func init() {
	callapi.RegisterHandler("get_user_info", GetUserInfo)
}

// UserInfoData get_user_info 成功回执的 data
type UserInfoData struct {
	UserID            string `json:"user_id"` // 原样回显入参
	Username          string `json:"username"`
	AllowProactiveMsg bool   `json:"allow_proactive_msg"`
	Avatar            string `json:"avatar"`
}

// GetUserInfoResponse get_user_info 回执；失败时 data 序列化为 null
type GetUserInfoResponse struct {
	RetCode int           `json:"retcode"`
	Status  string        `json:"status"`
	Data    *UserInfoData `json:"data"`
	Message string        `json:"message"`
	Echo    interface{}   `json:"echo,omitempty"`
}

// GetUserInfo 获取用户基本信息（QQ 官方 2026-11-16 新增 GET /v2/users/{user_openid}/info）
// params: user_id(虚拟号或 32 位 openid)
func GetUserInfo(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	userID, _ := message.Params.UserID.(string)
	if userID == "" {
		return sendUserInfoResult(client, message, nil, "缺少 user_id 参数", 1400)
	}

	userOpenID, err := resolveUserOpenID(userID)
	if err != nil {
		mylog.Printf("get_user_info: 反查 user_openid 失败: %v", err)
		return sendUserInfoResult(client, message, nil, err.Error(), 1500)
	}

	info, err := apiv2.UserInfo(context.TODO(), userOpenID)
	if err != nil {
		mylog.Printf("get_user_info: 获取用户信息失败: %v", err)
		return sendUserInfoResult(client, message, nil, err.Error(), 1500)
	}

	data := &UserInfoData{
		UserID:            userID,
		Username:          info.Username,
		AllowProactiveMsg: info.AllowProactiveMsg,
		Avatar:            info.Avatar,
	}
	mylog.Printf("get_user_info: user=%s username=%s allow_proactive_msg=%v", userOpenID, info.Username, info.AllowProactiveMsg)
	return sendUserInfoResult(client, message, data, "", 0)
}

// resolveUserOpenID 虚拟号反查真实 OpenID；32 位原生 OpenID 直接使用（与 get_avatar 同口径）
func resolveUserOpenID(userID string) (string, error) {
	if len(userID) == 32 {
		return userID, nil
	}
	openID, err := idmap.RetrieveRowByIDv2(userID)
	if err != nil || openID == "" {
		return "", fmt.Errorf("无法反查用户 OpenID: %v", err)
	}
	return openID, nil
}

// sendUserInfoResult 构造并回传 get_user_info 回执（retcode 0=成功；data 为 nil 时序列化为 null）
func sendUserInfoResult(client callapi.Client, message callapi.ActionMessage, data *UserInfoData, errMsg string, retCode int) (string, error) {
	status := "ok"
	if retCode != 0 {
		status = "failed"
	}
	response := GetUserInfoResponse{
		RetCode: retCode,
		Status:  status,
		Data:    data,
		Message: errMsg,
		Echo:    message.Echo,
	}
	outputMap := structToMap(response)
	if client != nil {
		if err := client.SendMessage(outputMap); err != nil {
			mylog.Printf("get_user_info: 发送响应失败: %v", err)
		}
	}
	result, err := json.Marshal(response)
	if err != nil {
		mylog.Printf("get_user_info: 序列化响应失败: %v", err)
		return "", err
	}
	return string(result), nil
}

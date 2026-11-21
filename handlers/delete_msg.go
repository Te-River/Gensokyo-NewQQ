package handlers

import (
	"context"
	"fmt"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/echo"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/internal/domain/identity"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/openapi"
)

func init() {
	callapi.RegisterHandler("delete_msg", DeleteMsg)
}

// DeleteMsg 撤回群消息或 C2C 消息。
//
// group_id / user_id 至少提供其一：给 group_id 走群撤回，只给 user_id 走 C2C 撤回。
// 两者皆缺属参数错误（1400）；上游撤回失败回 1500 并带真实错误——撤回失败不再回 retcode:0。
func DeleteMsg(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	// message_id 为外部输入，缺失或类型不符时无法定位消息，直接返回错误
	msgID, ok := message.Params.MessageID.(string)
	if !ok || msgID == "" {
		mylog.Printf("delete_msg: message_id 缺失或类型错误，跳过撤回: %#v", message.Params.MessageID)
		return sendActionResult(client, message, "message_id 缺失或类型错误", sendParamRetCode)
	}

	groupID := actionIDString(message.Params.GroupID)
	userID := actionIDString(message.Params.UserID)
	// 两者皆缺无法定位撤回目标：什么也不做却回 retcode:0 会让调用方以为消息已撤回
	if groupID == "" && userID == "" {
		mylog.Printf("delete_msg: group_id 与 user_id 均缺失，无法定位撤回目标: message_id=%s", msgID)
		return sendActionResult(client, message, "delete_msg 需要 group_id 或 user_id", sendParamRetCode)
	}

	// 虚拟 message_id → 真实 ID（string_ob11 模式下原样使用）
	realMsgID := msgID
	if !config.GetStringOb11() {
		if config.GetMemoryMsgid() {
			realMsgID, _ = echo.GetCacheIDFromMemoryByRowID(msgID)
		} else {
			var err error
			realMsgID, err = idmap.RetrieveRowByCachev2(msgID)
			if err != nil {
				mylog.Printf("delete_msg: message_id 反查真实 ID 失败: %v", err)
			}
		}
	}

	var retractErr error
	if groupID != "" {
		groupOpenID, err := resolveRetractOpenID(groupID, "group_id")
		if err != nil {
			mylog.Printf("delete_msg: %v", err)
			return sendActionResult(client, message, err.Error(), sendParamRetCode)
		}
		// 必须以反查后的 group_openid 调上游：虚拟号不是合法的 group_openid（上游回 404 11001 不支持的调用）
		retractErr = api.RetractGroupMessage(context.TODO(), groupOpenID, realMsgID, openapi.RetractMessageOptionHidetip)
		if retractErr != nil {
			mylog.Printf("delete_msg 群消息撤回失败: group_id=%s group_openid=%s message_id=%s err=%v", groupID, groupOpenID, realMsgID, retractErr)
		}
	} else {
		userOpenID, err := resolveRetractOpenID(userID, "user_id")
		if err != nil {
			mylog.Printf("delete_msg: %v", err)
			return sendActionResult(client, message, err.Error(), sendParamRetCode)
		}
		retractErr = api.RetractC2CMessage(context.TODO(), userOpenID, realMsgID, openapi.RetractMessageOptionHidetip)
		if retractErr != nil {
			mylog.Printf("delete_msg C2C 消息撤回失败: user_id=%s user_openid=%s message_id=%s err=%v", userID, userOpenID, realMsgID, retractErr)
		}
	}

	if retractErr != nil {
		// 撤回失败的 message 带上上游真实错误；sendFailureMessage 的「审核中可能已发出」提示只适用于发送，不追加
		return sendActionResult(client, message, retractErr.Error(), sendFailRetCode)
	}

	mylog.Printf("delete_msg 撤回成功: group_id=%s user_id=%s message_id=%s", groupID, userID, realMsgID)
	return sendActionResult(client, message, "", 0)
}

// resolveRetractOpenID 把撤回接口的入参还原为 QQ 官方 OpenID：
// 已是 OpenID 则原样返回，虚拟号按 idmap 反查；反查失败属参数/映射问题（1400）。
func resolveRetractOpenID(value, field string) (string, error) {
	if identity.IsOpenID(value) {
		return value, nil
	}
	openID, err := idmap.RetrieveRowByIDv2(value)
	if err != nil {
		return "", fmt.Errorf("无法反查 %s 的 OpenID: %w", field, err)
	}
	if openID == "" {
		return "", fmt.Errorf("无法反查 %s 的 OpenID: %s 无映射", field, value)
	}
	return openID, nil
}

package handlers

import (
	"context"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/openapi"
)

func init() {
	callapi.RegisterHandler("get_group_bot_state", GetGroupBotState)
}

// GroupBotState OneBot 机器人群内状态
type GroupBotState struct {
	GroupID        int64  `json:"group_id"`
	JoinTime       int64  `json:"join_time"`
	CanPush        bool   `json:"can_push"`
	PushMsgSetting string `json:"push_msg_setting"`
	Role           string `json:"role"`
}

// GetGroupBotState 获取机器人在群内的状态
// params: group_id(虚拟/真实群ID)
func GetGroupBotState(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	groupID := message.Params.GroupID.(string)

	// 反查真实 OpenID（32 位原生 OpenID 直接使用）
	groupOpenID := groupID
	if len(groupID) != 32 {
		realGroupID, err := idmap.RetrieveRowByIDv2(groupID)
		if err != nil || realGroupID == "" {
			mylog.Printf("get_group_bot_state: 反查 group_openid 失败: %v", err)
			return sendActionResult(client, message, "无法反查群 OpenID", 100)
		}
		groupOpenID = realGroupID
	}

	state, err := apiv2.BotInGroupState(context.TODO(), groupOpenID)
	if err != nil {
		mylog.Printf("get_group_bot_state: 获取群内状态失败: %v", err)
		return sendActionResult(client, message, err.Error(), 100)
	}

	groupID64, _ := idmap.StoreIDv2(groupOpenID)
	data := GroupBotState{
		GroupID:        groupID64,
		JoinTime:       state.JoinTime,
		CanPush:        state.CanPush,
		PushMsgSetting: state.PushMsgSetting,
		Role:           state.Role,
	}
	mylog.Printf("get_group_bot_state: group=%s role=%s can_push=%v", groupOpenID, state.Role, state.CanPush)
	return sendActionResultWithData(client, message, data)
}

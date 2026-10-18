package handlers

import (
	"context"
	"time"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/openapi"
)

func init() {
	callapi.RegisterHandler("get_group_bot_state", GetGroupBotState)
}

// GroupBotState OneBot 机器人群内状态
// 兼容双写：旧字段 join_time/can_push/push_msg_setting/role 取自官方新字段，
// 新字段 joined_at/allow_proactive_msg/recv_msg_setting/member_role 与官方一致。
type GroupBotState struct {
	GroupID           int64  `json:"group_id"`
	JoinTime          int64  `json:"join_time"`
	CanPush           bool   `json:"can_push"`
	PushMsgSetting    string `json:"push_msg_setting"`
	Role              string `json:"role"`
	JoinedAt          string `json:"joined_at"`
	AllowProactiveMsg bool   `json:"allow_proactive_msg"`
	RecvMsgSetting    string `json:"recv_msg_setting"`
	MemberRole        string `json:"member_role"`
	MemberOpenID      string `json:"member_openid"` // 机器人的 OpenID（官方 member_openid 原样透传）
}

// parseBotJoinedAt 解析官方 joined_at（RFC3339）为 Unix 秒，空串/非法格式置 0
func parseBotJoinedAt(value string) int64 {
	if value == "" {
		return 0
	}
	t, err := time.Parse(time.RFC3339, value)
	if err != nil {
		mylog.Printf("get_group_bot_state: joined_at 解析失败(%s),置 0: %v", value, err)
		return 0
	}
	return t.Unix()
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
		GroupID:           groupID64,
		JoinTime:          parseBotJoinedAt(state.JoinedAt),
		CanPush:           state.AllowProactiveMsg,
		PushMsgSetting:    state.RecvMsgSetting,
		Role:              state.MemberRole,
		JoinedAt:          state.JoinedAt,
		AllowProactiveMsg: state.AllowProactiveMsg,
		RecvMsgSetting:    state.RecvMsgSetting,
		MemberRole:        state.MemberRole,
		MemberOpenID:      state.MemberOpenID,
	}
	mylog.Printf("get_group_bot_state: group=%s member_role=%s allow_proactive_msg=%v", groupOpenID, state.MemberRole, state.AllowProactiveMsg)
	return sendActionResultWithData(client, message, data)
}

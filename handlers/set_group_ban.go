package handlers

import (
	"context"
	"time"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

func init() {
	callapi.RegisterHandler("set_group_ban", SetGroupBan)
	callapi.RegisterHandler("get_group_ban", SetGroupBan) // 兼容旧 action 名
}

// SetGroupBan 设置群成员禁言
// params: group_id(虚拟/真实群ID), user_id(虚拟/真实用户ID), duration(秒, 0=解除禁言)
func SetGroupBan(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	groupID := message.Params.GroupID.(string)
	msgType, err := idmap.ReadConfigv2(groupID, "type")
	if err != nil {
		mylog.Printf("set_group_ban: 读取群类型失败: %v", err)
		return "", nil
	}
	if msgType != "group" {
		mylog.Printf("set_group_ban: 仅支持群聊场景, 当前类型 %s", msgType)
		return "", nil
	}

	// 反查真实 OpenID（32 位原生 OpenID 直接使用）
	groupOpenID := groupID
	if len(groupID) != 32 {
		realGroupID, err := idmap.RetrieveRowByIDv2(groupID)
		if err != nil || realGroupID == "" {
			mylog.Printf("set_group_ban: 反查 group_openid 失败: %v", err)
			return sendActionResult(client, message, "无法反查群 OpenID", 100)
		}
		groupOpenID = realGroupID
	}
	userID := message.Params.UserID.(string)
	memberOpenID := userID
	if len(userID) != 32 {
		realUserID, err := idmap.RetrieveRowByIDv2(userID)
		if err != nil || realUserID == "" {
			mylog.Printf("set_group_ban: 反查 member_openid 失败: %v", err)
			return sendActionResult(client, message, "无法反查用户 OpenID", 100)
		}
		memberOpenID = realUserID
	}

	setting := &dto.RestrictChatSetting{GroupOpenID: groupOpenID}
	if message.Params.Duration > 0 {
		// 禁言到当前时间 + duration
		setting.MemberRestrict = []dto.MemberRestrict{{
			MemberOpenID:  memberOpenID,
			RestrictUntil: time.Now().Unix() + int64(message.Params.Duration),
		}}
	} else {
		// 解除禁言：查询当前设置, 移除该成员后提交
		cur, err := apiv2.RestrictChatSetting(context.TODO(), groupOpenID)
		if err != nil {
			mylog.Printf("set_group_ban: 查询禁言状态失败: %v", err)
			return sendActionResult(client, message, err.Error(), 100)
		}
		for _, m := range cur.MemberRestrict {
			if m.MemberOpenID != memberOpenID {
				setting.MemberRestrict = append(setting.MemberRestrict, m)
			}
		}
	}

	if err := apiv2.SetRestrictChatSetting(context.TODO(), groupOpenID, setting); err != nil {
		mylog.Printf("set_group_ban: 设置禁言失败: %v", err)
		return sendActionResult(client, message, err.Error(), 100)
	}

	mylog.Printf("set_group_ban: 成功 group=%s user=%s duration=%d", groupOpenID, memberOpenID, message.Params.Duration)
	return sendActionResult(client, message, "", 0)
}

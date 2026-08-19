package handlers

import (
	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/openapi"
)

func init() {
	callapi.RegisterHandler("set_group_ban", SetGroupBan)
	callapi.RegisterHandler("get_group_ban", SetGroupBan) // 兼容旧 action 名
}

// SetGroupBan 设置群成员禁言
// params: group_id(虚拟/真实群ID), user_id(虚拟/真实用户ID), duration(秒, 0=解除禁言)
// 内部复用 set_group_helpers.go 共享底层，与 [CQ:set_group,action=ban] 行为一致
func SetGroupBan(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	groupID, _ := message.Params.GroupID.(string)
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
	groupOpenID, err := resolveGroupOpenID(groupID)
	if err != nil {
		mylog.Printf("set_group_ban: 反查 group_openid 失败: %v", err)
		return sendActionResult(client, message, "无法反查群 OpenID", 100)
	}
	userID, _ := message.Params.UserID.(string)
	memberOpenID, err := resolveMemberOpenID(userID)
	if err != nil {
		mylog.Printf("set_group_ban: 反查 member_openid 失败: %v", err)
		return sendActionResult(client, message, "无法反查用户 OpenID", 100)
	}

	if err := applyRestrictChatSetting(apiv2, groupOpenID, memberOpenID, message.Params.Duration, nil); err != nil {
		mylog.Printf("set_group_ban: 设置禁言失败: %v", err)
		return sendActionResult(client, message, err.Error(), 100)
	}

	mylog.Printf("set_group_ban: 成功 group=%s user=%s duration=%d", groupOpenID, memberOpenID, message.Params.Duration)
	return sendActionResult(client, message, "", 0)
}

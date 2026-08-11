package handlers

import (
	"context"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

func init() {
	callapi.RegisterHandler("set_group_whole_ban", SetGroupWholeBan)
	callapi.RegisterHandler("get_group_whole_ban", SetGroupWholeBan) // 兼容旧 action 名
}

// SetGroupWholeBan 设置群全员禁言
// params: group_id(虚拟/真实群ID), enable(bool, true=全员禁言 false=解除)
func SetGroupWholeBan(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	groupID := message.Params.GroupID.(string)
	msgType, err := idmap.ReadConfigv2(groupID, "type")
	if err != nil {
		mylog.Printf("set_group_whole_ban: 读取群类型失败: %v", err)
		return "", nil
	}
	if msgType != "group" {
		mylog.Printf("set_group_whole_ban: 仅支持群聊场景, 当前类型 %s", msgType)
		return "", nil
	}

	// 反查真实 OpenID（32 位原生 OpenID 直接使用）
	groupOpenID := groupID
	if len(groupID) != 32 {
		realGroupID, err := idmap.RetrieveRowByIDv2(groupID)
		if err != nil || realGroupID == "" {
			mylog.Printf("set_group_whole_ban: 反查 group_openid 失败: %v", err)
			return sendActionResult(client, message, "无法反查群 OpenID", 100)
		}
		groupOpenID = realGroupID
	}

	// 查询当前设置, 保留已有成员级禁言, 仅切换全员禁言
	setting := &dto.RestrictChatSetting{GroupOpenID: groupOpenID, AllMute: message.Params.Enable}
	if cur, err := apiv2.RestrictChatSetting(context.TODO(), groupOpenID); err == nil {
		setting.MemberRestrict = cur.MemberRestrict
	} else {
		mylog.Printf("set_group_whole_ban: 查询禁言状态失败: %v", err)
	}

	if err := apiv2.SetRestrictChatSetting(context.TODO(), groupOpenID, setting); err != nil {
		mylog.Printf("set_group_whole_ban: 设置全员禁言失败: %v", err)
		return sendActionResult(client, message, err.Error(), 100)
	}

	mylog.Printf("set_group_whole_ban: 成功 group=%s enable=%v", groupOpenID, message.Params.Enable)
	return sendActionResult(client, message, "", 0)
}

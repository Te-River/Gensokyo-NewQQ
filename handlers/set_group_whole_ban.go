package handlers

import (
	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/openapi"
)

func init() {
	callapi.RegisterHandler("set_group_whole_ban", SetGroupWholeBan)
	callapi.RegisterHandler("get_group_whole_ban", SetGroupWholeBan) // 兼容旧 action 名
}

// SetGroupWholeBan 设置群全员禁言
// params: group_id(虚拟/真实群ID), enable(bool, true=全员禁言 false=解除)
// 内部复用 set_group_helpers.go 共享底层，与 [CQ:set_group,action=whole_ban] 行为一致
func SetGroupWholeBan(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	groupID, _ := message.Params.GroupID.(string)
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
	groupOpenID, err := resolveGroupOpenID(groupID)
	if err != nil {
		mylog.Printf("set_group_whole_ban: 反查 group_openid 失败: %v", err)
		return sendActionResult(client, message, "无法反查群 OpenID", 100)
	}

	// 查询当前设置, 保留已有成员级禁言, 仅切换全员禁言
	allMute := message.Params.Enable
	if err := applyRestrictChatSetting(apiv2, groupOpenID, "", 0, &allMute); err != nil {
		mylog.Printf("set_group_whole_ban: 设置全员禁言失败: %v", err)
		return sendActionResult(client, message, err.Error(), 100)
	}

	mylog.Printf("set_group_whole_ban: 成功 group=%s enable=%v", groupOpenID, message.Params.Enable)
	return sendActionResult(client, message, "", 0)
}

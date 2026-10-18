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
// 官方 POST /restrict_chat_setting 请求体只有 members，无全员禁言写入字段，
// 固定返回失败回执(retcode 100)；与 [CQ:set_group,action=whole_ban] 共用 set_group_helpers.go 共享底层
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

	// 官方 POST /restrict_chat_setting 请求体只有 members，无全员禁言写入字段：
	// allMute 非 nil 时 applyRestrictChatSetting 恒定返回 errWholeBanUnsupported，
	// 故不设成功分支，固定返回失败回执（retcode 100）
	allMute := message.Params.Enable
	err = applyRestrictChatSetting(apiv2, groupOpenID, "", 0, &allMute)
	mylog.Printf("set_group_whole_ban: 设置全员禁言失败: %v", err)
	return sendActionResult(client, message, err.Error(), 100)
}

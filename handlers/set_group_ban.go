package handlers

import (
	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/openapi"
)

func init() {
	callapi.RegisterHandler("get_group_ban", SetGroupBan)
}

func SetGroupBan(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	// 从message中获取group_id
	groupID := message.Params.GroupID.(string)
	msgType, err := idmap.ReadConfigv2(groupID, "type")
	if err != nil {
		mylog.Printf("Error reading config: %v", err)
		return "", nil
	}
	switch msgType {
	case "group":
		mylog.Printf("setGroupBan(频道): 目前暂未开放该能力")
		return "", nil
	case "private":
		mylog.Printf("setGroupBan(频道): 目前暂未适配私聊虚拟群场景的禁言能力")
		return "", nil
	}
	return "", nil
}

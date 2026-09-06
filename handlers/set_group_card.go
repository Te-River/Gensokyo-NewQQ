package handlers

import (
	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/tencent-connect/botgo/openapi"
)

func init() {
	callapi.RegisterHandler("set_group_card", SetGroupCard)
}

// SetGroupCard QQ官方API未提供设置群名片接口,明确失败回执(不再假成功)
func SetGroupCard(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	return sendActionResult(client, message, "QQ官方API未提供设置群名片接口,set_group_card无法实现", 100)
}

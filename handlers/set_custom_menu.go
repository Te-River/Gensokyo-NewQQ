package handlers

import (
	"context"
	"encoding/json"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

func init() {
	callapi.RegisterHandler("set_custom_menu", SetCustomMenu)
}

// SetCustomMenu 设置 C2C 自定义菜单（官方 PUT /v2/menu，覆盖式）。
// params: menu(菜单对象,原样透传官方校验:items≤10,name≤10字符,link 必须 https://)
func SetCustomMenu(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	if message.Params.Menu == nil {
		mylog.Printf("set_custom_menu: menu 参数缺失")
		return sendActionResult(client, message, "menu 参数缺失", 100)
	}

	// 原始对象透传官方校验：Marshal→Unmarshal 复用 dto.Menu 结构序列化
	raw, err := json.Marshal(message.Params.Menu)
	if err != nil {
		mylog.Printf("set_custom_menu: menu 参数序列化失败: %v", err)
		return sendActionResult(client, message, "menu 参数序列化失败", 100)
	}
	var menu dto.Menu
	if err := decodeStrictJSON(raw, &menu); err != nil {
		mylog.Printf("set_custom_menu: menu 参数格式无效(含未知字段或类型错误): %v", err)
		return sendActionResult(client, message, "menu 参数格式无效: "+err.Error(), 100)
	}

	resp, err := apiv2.PutMenu(context.TODO(), &dto.PutMenuRequest{Menu: &menu})
	if err != nil {
		mylog.Printf("set_custom_menu: 设置菜单失败: %v", err)
		return sendActionResult(client, message, err.Error(), 100)
	}

	data := struct {
		Version int64 `json:"version"`
	}{
		Version: resp.Version,
	}
	mylog.Printf("set_custom_menu: 设置成功 version=%d", resp.Version)
	return sendActionResultWithData(client, message, data)
}

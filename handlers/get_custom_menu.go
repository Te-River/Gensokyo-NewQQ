package handlers

import (
	"context"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/openapi"
)

func init() {
	callapi.RegisterHandler("get_custom_menu", GetCustomMenu)
}

// GetCustomMenu 获取 C2C 自定义菜单（官方 GET /v2/menu，全局生效）。
// 未设置过菜单时 data.menu 为空对象 {}。
func GetCustomMenu(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	resp, err := apiv2.GetMenu(context.TODO())
	if err != nil {
		mylog.Printf("get_custom_menu: 获取菜单失败: %v", err)
		return sendActionResult(client, message, err.Error(), 100)
	}

	// 官方未设置过时 menu 为 null,按契约输出空对象 {}
	var menu interface{} = map[string]interface{}{}
	if resp.Menu != nil {
		menu = resp.Menu
	}
	data := struct {
		Version int64       `json:"version"`
		Menu    interface{} `json:"menu"`
	}{
		Version: resp.Version,
		Menu:    menu,
	}
	mylog.Printf("get_custom_menu: version=%d", resp.Version)
	return sendActionResultWithData(client, message, data)
}

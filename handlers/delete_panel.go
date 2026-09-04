package handlers

import (
	"context"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/openapi"
)

func init() {
	callapi.RegisterHandler("delete_panel", DeletePanel)
}

// DeletePanel 删除指令面板（官方 DELETE /v2/panels/{panel_id}，响应空）。
// params: panel_id(必填)
func DeletePanel(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	if message.Params.PanelID == "" {
		mylog.Printf("delete_panel: panel_id 参数缺失")
		return sendActionResult(client, message, "panel_id 参数缺失", 100)
	}

	if err := apiv2.DeletePanel(context.TODO(), message.Params.PanelID); err != nil {
		mylog.Printf("delete_panel: 删除面板失败: %v", err)
		return sendActionResult(client, message, err.Error(), 100)
	}

	mylog.Printf("delete_panel: 已删除 panel_id=%s", message.Params.PanelID)
	return sendActionResultWithData(client, message, map[string]interface{}{})
}

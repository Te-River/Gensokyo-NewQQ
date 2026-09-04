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
	callapi.RegisterHandler("set_panel", SetPanel)
}

// SetPanel 更新指令面板元素与备注（官方 PUT /v2/panels/{panel_id}，覆盖式，不影响关联对象）。
// params: panel_id(必填), panel(面板对象,覆盖元素与备注,原样透传官方校验)
func SetPanel(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	if message.Params.PanelID == "" {
		mylog.Printf("set_panel: panel_id 参数缺失")
		return sendActionResult(client, message, "panel_id 参数缺失", 100)
	}
	if message.Params.Panel == nil {
		mylog.Printf("set_panel: panel 参数缺失")
		return sendActionResult(client, message, "panel 参数缺失", 100)
	}

	// 原始对象透传官方校验：Marshal→Unmarshal 复用 dto.Panel 结构序列化
	raw, err := json.Marshal(message.Params.Panel)
	if err != nil {
		mylog.Printf("set_panel: panel 参数序列化失败: %v", err)
		return sendActionResult(client, message, "panel 参数序列化失败", 100)
	}
	var panel dto.Panel
	if err := decodeStrictJSON(raw, &panel); err != nil {
		mylog.Printf("set_panel: panel 参数格式无效(含未知字段或类型错误): %v", err)
		return sendActionResult(client, message, "panel 参数格式无效: "+err.Error(), 100)
	}

	resp, err := apiv2.UpdatePanel(context.TODO(), message.Params.PanelID, &dto.UpdatePanelRequest{Panel: &panel})
	if err != nil {
		mylog.Printf("set_panel: 更新面板失败: %v", err)
		return sendActionResult(client, message, err.Error(), 100)
	}

	data := struct {
		Version int64 `json:"version"`
	}{
		Version: resp.Version,
	}
	mylog.Printf("set_panel: 更新成功 panel_id=%s version=%d", message.Params.PanelID, resp.Version)
	return sendActionResultWithData(client, message, data)
}

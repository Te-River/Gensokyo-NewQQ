package handlers

import (
	"context"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

func init() {
	callapi.RegisterHandler("set_panel_target", SetPanelTarget)
}

// SetPanelTarget 增删指令面板关联对象（官方 PUT /v2/panels/{panel_id}/target，响应无）。
// params: panel_id(必填), op(add|del 必填), user_openids/group_openids(虚拟 ID 列表,二选一或同时)
// 关联对象列表任一反查失败 → 整体失败（specific 列表必须准确）。
func SetPanelTarget(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	if message.Params.PanelID == "" {
		mylog.Printf("set_panel_target: panel_id 参数缺失")
		return sendActionResult(client, message, "panel_id 参数缺失", 100)
	}
	if message.Params.Op != "add" && message.Params.Op != "del" {
		mylog.Printf("set_panel_target: op 参数无效: %s", message.Params.Op)
		return sendActionResult(client, message, "op 必须为 add 或 del", 100)
	}
	if len(message.Params.UserOpenIDs) == 0 && len(message.Params.GroupOpenIDs) == 0 {
		mylog.Printf("set_panel_target: user_openids 与 group_openids 不能同时为空")
		return sendActionResult(client, message, "user_openids 或 group_openids 至少提供一个", 100)
	}

	// 关联对象：虚拟 ID → 真实 OpenID（任一反查失败整体失败）
	userOpenIDs, err := resolveOpenIDList(message.Params.UserOpenIDs, resolveMemberOpenID)
	if err != nil {
		mylog.Printf("set_panel_target: user_openids 反查失败: %v", err)
		return sendActionResult(client, message, err.Error(), 100)
	}
	groupOpenIDs, err := resolveOpenIDList(message.Params.GroupOpenIDs, resolveGroupOpenID)
	if err != nil {
		mylog.Printf("set_panel_target: group_openids 反查失败: %v", err)
		return sendActionResult(client, message, err.Error(), 100)
	}

	req := &dto.PanelTargetRequest{
		Op:           message.Params.Op,
		UserOpenIDs:  userOpenIDs,
		GroupOpenIDs: groupOpenIDs,
	}
	if err := apiv2.UpdatePanelTarget(context.TODO(), message.Params.PanelID, req); err != nil {
		mylog.Printf("set_panel_target: 更新关联对象失败 op=%s: %v", message.Params.Op, err)
		return sendActionResult(client, message, err.Error(), 100)
	}

	mylog.Printf("set_panel_target: 已更新关联对象 panel_id=%s op=%s", message.Params.PanelID, message.Params.Op)
	return sendActionResultWithData(client, message, map[string]interface{}{})
}

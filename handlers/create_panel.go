package handlers

import (
	"bytes"
	"context"
	"encoding/json"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

func init() {
	callapi.RegisterHandler("create_panel", CreatePanel)
}

// CreatePanel 创建指令面板（官方 POST /v2/panels）。
// params: scope(必填), target_type(all|specific), user_openids/group_openids(虚拟 ID 列表,仅 specific),
//
//	panel(面板对象,原样透传官方校验:items≤20)
//
// specific 对象列表任一反查失败 → 整体失败（specific 列表必须准确）。
func CreatePanel(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	if message.Params.Scope == "" {
		mylog.Printf("create_panel: scope 参数缺失")
		return sendActionResult(client, message, "scope 参数缺失", 100)
	}
	if message.Params.Panel == nil {
		mylog.Printf("create_panel: panel 参数缺失")
		return sendActionResult(client, message, "panel 参数缺失", 100)
	}

	// 原始对象透传官方校验：Marshal→Unmarshal 复用 dto.Panel 结构序列化
	raw, err := json.Marshal(message.Params.Panel)
	if err != nil {
		mylog.Printf("create_panel: panel 参数序列化失败: %v", err)
		return sendActionResult(client, message, "panel 参数序列化失败", 100)
	}
	var panel dto.Panel
	if err := decodeStrictJSON(raw, &panel); err != nil {
		mylog.Printf("create_panel: panel 参数格式无效(含未知字段或类型错误): %v", err)
		return sendActionResult(client, message, "panel 参数格式无效: "+err.Error(), 100)
	}

	// 关联对象：虚拟 ID → 真实 OpenID（任一反查失败整体失败）
	userOpenIDs, err := resolveOpenIDList(message.Params.UserOpenIDs, resolveMemberOpenID)
	if err != nil {
		mylog.Printf("create_panel: user_openids 反查失败: %v", err)
		return sendActionResult(client, message, err.Error(), 100)
	}
	groupOpenIDs, err := resolveOpenIDList(message.Params.GroupOpenIDs, resolveGroupOpenID)
	if err != nil {
		mylog.Printf("create_panel: group_openids 反查失败: %v", err)
		return sendActionResult(client, message, err.Error(), 100)
	}

	req := &dto.CreatePanelRequest{
		Scope:        message.Params.Scope,
		TargetType:   message.Params.TargetType,
		UserOpenIDs:  userOpenIDs,
		GroupOpenIDs: groupOpenIDs,
		Panel:        &panel,
	}
	resp, err := apiv2.CreatePanel(context.TODO(), req)
	if err != nil {
		mylog.Printf("create_panel: 创建面板失败: %v", err)
		return sendActionResult(client, message, err.Error(), 100)
	}

	data := struct {
		PanelID string `json:"panel_id"`
	}{
		PanelID: resp.PanelID,
	}
	mylog.Printf("create_panel: 创建成功 panel_id=%s scope=%s", resp.PanelID, message.Params.Scope)
	return sendActionResultWithData(client, message, data)
}

// resolveOpenIDList 逐个反查对象 ID 列表，任一失败返回整体错误（specific 列表必须准确）。
func resolveOpenIDList(ids []string, resolve func(string) (string, error)) ([]string, error) {
	if len(ids) == 0 {
		return nil, nil
	}
	openIDs := make([]string, 0, len(ids))
	for _, id := range ids {
		openID, err := resolve(id)
		if err != nil {
			return nil, err
		}
		openIDs = append(openIDs, openID)
	}
	return openIDs, nil
}

// decodeStrictJSON 严格解码客户端透传对象:未知字段(如拼错的 sub_menus)直接报错而非静默丢弃,
// 防止覆盖式 PUT/POST 把残缺对象提交官方后清空既有菜单/面板配置。
func decodeStrictJSON(raw []byte, v interface{}) error {
	dec := json.NewDecoder(bytes.NewReader(raw))
	dec.DisallowUnknownFields()
	return dec.Decode(v)
}

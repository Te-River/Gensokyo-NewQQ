package handlers

import (
	"context"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

func init() {
	callapi.RegisterHandler("get_panel", GetPanel)
}

// panelDetailResponse 指令面板详情响应。
// 关联对象输出虚拟 ID（经 StoreIDv2 转换,字段名沿用官方 user_openids/group_openids）。
type panelDetailResponse struct {
	PanelID      string     `json:"panel_id"`
	Scope        string     `json:"scope"`
	TargetType   string     `json:"target_type"`
	Panel        *dto.Panel `json:"panel"`
	CreatedAt    string     `json:"created_at"`
	UpdatedAt    string     `json:"updated_at"`
	Version      int64      `json:"version"`
	UserOpenIDs  []int64    `json:"user_openids,omitempty"`
	GroupOpenIDs []int64    `json:"group_openids,omitempty"`
}

// GetPanel 获取单个指令面板详情（官方 GET /v2/panels/{panel_id}）。
// params: panel_id(必填)
func GetPanel(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	if message.Params.PanelID == "" {
		mylog.Printf("get_panel: panel_id 参数缺失")
		return sendActionResult(client, message, "panel_id 参数缺失", 100)
	}

	detail, err := apiv2.GetPanel(context.TODO(), message.Params.PanelID)
	if err != nil {
		mylog.Printf("get_panel: 获取面板详情失败: %v", err)
		return sendActionResult(client, message, err.Error(), 100)
	}

	resp := panelDetailResponse{
		PanelID:    detail.PanelID,
		Scope:      detail.Scope,
		TargetType: detail.TargetType,
		Panel:      detail.Panel,
		CreatedAt:  detail.CreatedAt,
		UpdatedAt:  detail.UpdatedAt,
		Version:    detail.Version,
	}
	// 关联对象 openid → 虚拟 ID 输出（应用端可直接回传 set_panel_target 操作）
	if len(detail.UserOpenIDs) > 0 {
		resp.UserOpenIDs = make([]int64, 0, len(detail.UserOpenIDs))
		for _, openID := range detail.UserOpenIDs {
			vid, err := idmap.StoreIDv2(openID)
			if err != nil {
				// 反查失败跳过该条目,避免 user_id=0 进入响应被误用
				mylog.Printf("get_panel: user openid 转虚拟 ID 失败,跳过: %v", err)
				continue
			}
			resp.UserOpenIDs = append(resp.UserOpenIDs, vid)
		}
	}
	if len(detail.GroupOpenIDs) > 0 {
		resp.GroupOpenIDs = make([]int64, 0, len(detail.GroupOpenIDs))
		for _, openID := range detail.GroupOpenIDs {
			vid, err := idmap.StoreIDv2(openID)
			if err != nil {
				// 反查失败跳过该条目,避免 group_id=0 进入响应被误用
				mylog.Printf("get_panel: group openid 转虚拟 ID 失败,跳过: %v", err)
				continue
			}
			resp.GroupOpenIDs = append(resp.GroupOpenIDs, vid)
		}
	}
	mylog.Printf("get_panel: panel_id=%s scope=%s", detail.PanelID, detail.Scope)
	return sendActionResultWithData(client, message, resp)
}

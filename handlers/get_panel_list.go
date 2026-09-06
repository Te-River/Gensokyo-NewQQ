package handlers

import (
	"context"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

func init() {
	callapi.RegisterHandler("get_panel_list", GetPanelList)
}

// GetPanelList 获取指令面板列表（官方 GET /v2/panels，游标分页）。
// params: scope(必填, c2c/group/channel/dm), cursor(可选分页游标), limit(可选单页数量,默认 20)
func GetPanelList(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	if message.Params.Scope == "" {
		mylog.Printf("get_panel_list: scope 参数缺失")
		return sendActionResult(client, message, "scope 参数缺失", 100)
	}

	limit := message.Params.Limit
	if limit <= 0 {
		limit = 20
	}

	list, err := apiv2.GetPanels(context.TODO(), message.Params.Scope, message.Params.Cursor, limit)
	if err != nil {
		mylog.Printf("get_panel_list: 拉取面板列表失败: %v", err)
		return sendActionResult(client, message, err.Error(), 100)
	}

	records := list.Records
	if records == nil {
		records = []dto.PanelRecord{}
	}
	data := struct {
		Records    []dto.PanelRecord `json:"records"`
		NextCursor string            `json:"next_cursor"`
		IsEnd      bool              `json:"is_end"`
	}{
		Records:    records,
		NextCursor: list.NextCursor,
		IsEnd:      list.IsEnd,
	}
	mylog.Printf("get_panel_list: scope=%s 共 %d 条记录", message.Params.Scope, len(records))
	return sendActionResultWithData(client, message, data)
}

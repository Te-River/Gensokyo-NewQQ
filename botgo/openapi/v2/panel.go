package v2

import (
	"context"
	"strconv"

	"github.com/tencent-connect/botgo/dto"
)

// GetPanels 指令面板列表
func (o *openAPIv2) GetPanels(ctx context.Context, scope, cursor string, limit int) (*dto.PanelList, error) {
	request := o.request(ctx).
		SetResult(dto.PanelList{})
	if scope != "" {
		request = request.SetQueryParam("scope", scope)
	}
	if cursor != "" {
		request = request.SetQueryParam("cursor", cursor)
	}
	if limit > 0 {
		request = request.SetQueryParam("limit", strconv.Itoa(limit))
	}
	resp, err := request.Get(o.getURL(panelsURI))
	if err != nil {
		return nil, err
	}
	return resp.Result().(*dto.PanelList), nil
}

// CreatePanel 创建指令面板
func (o *openAPIv2) CreatePanel(ctx context.Context, req *dto.CreatePanelRequest) (*dto.CreatePanelResponse, error) {
	resp, err := o.request(ctx).
		SetResult(dto.CreatePanelResponse{}).
		SetBody(req).
		Post(o.getURL(panelsURI))
	if err != nil {
		return nil, err
	}
	return resp.Result().(*dto.CreatePanelResponse), nil
}

// GetPanel 获取单个指令面板详情
func (o *openAPIv2) GetPanel(ctx context.Context, panelID string) (*dto.PanelRecordDetail, error) {
	resp, err := o.request(ctx).
		SetResult(dto.PanelRecordDetail{}).
		SetPathParam("panel_id", panelID).
		Get(o.getURL(panelURI))
	if err != nil {
		return nil, err
	}
	return resp.Result().(*dto.PanelRecordDetail), nil
}

// UpdatePanel 更新指令面板元素与备注
func (o *openAPIv2) UpdatePanel(ctx context.Context, panelID string, req *dto.UpdatePanelRequest) (*dto.PanelVersionResponse, error) {
	resp, err := o.request(ctx).
		SetResult(dto.PanelVersionResponse{}).
		SetPathParam("panel_id", panelID).
		SetBody(req).
		Put(o.getURL(panelURI))
	if err != nil {
		return nil, err
	}
	return resp.Result().(*dto.PanelVersionResponse), nil
}

// DeletePanel 删除指令面板（响应空 {}）
func (o *openAPIv2) DeletePanel(ctx context.Context, panelID string) error {
	_, err := o.request(ctx).
		SetPathParam("panel_id", panelID).
		Delete(o.getURL(panelURI))
	return err
}

// UpdatePanelTarget 增删指令面板关联对象（响应无）
func (o *openAPIv2) UpdatePanelTarget(ctx context.Context, panelID string, req *dto.PanelTargetRequest) error {
	_, err := o.request(ctx).
		SetPathParam("panel_id", panelID).
		SetBody(req).
		Put(o.getURL(panelTargetURI))
	return err
}

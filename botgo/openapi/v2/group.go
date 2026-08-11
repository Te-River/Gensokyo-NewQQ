package v2

import (
	"context"
	"strconv"

	"github.com/tencent-connect/botgo/dto"
)

// GroupInfo 获取群基本信息
func (o *openAPIv2) GroupInfo(ctx context.Context, groupOpenID string) (*dto.GroupInfo, error) {
	resp, err := o.request(ctx).
		SetResult(dto.GroupInfo{}).
		SetPathParam("group_id", groupOpenID).
		Get(o.getURL(groupInfoURI))
	if err != nil {
		return nil, err
	}
	return resp.Result().(*dto.GroupInfo), nil
}

// BotInGroupState 获取机器人群内状态
func (o *openAPIv2) BotInGroupState(ctx context.Context, groupOpenID string) (*dto.BotInGroupState, error) {
	resp, err := o.request(ctx).
		SetResult(dto.BotInGroupState{}).
		SetPathParam("group_id", groupOpenID).
		Get(o.getURL(groupBotStateURI))
	if err != nil {
		return nil, err
	}
	return resp.Result().(*dto.BotInGroupState), nil
}

// JoinRequestList 入群申请列表
func (o *openAPIv2) JoinRequestList(ctx context.Context, groupOpenID string, nextIndex int) (*dto.JoinRequestList, error) {
	request := o.request(ctx).
		SetResult(dto.JoinRequestList{}).
		SetPathParam("group_id", groupOpenID)
	if nextIndex > 0 {
		request = request.SetQueryParam("next_index", strconv.Itoa(nextIndex))
	}
	resp, err := request.Get(o.getURL(groupJoinRequestListURI))
	if err != nil {
		return nil, err
	}
	return resp.Result().(*dto.JoinRequestList), nil
}

// ApprovalJoinRequest 入群申请审批
func (o *openAPIv2) ApprovalJoinRequest(ctx context.Context, groupOpenID, memberOpenID string, req *dto.ApprovalJoinRequest) error {
	_, err := o.request(ctx).
		SetPathParam("group_id", groupOpenID).
		SetPathParam("member_openid", memberOpenID).
		SetBody(req).
		Post(o.getURL(groupApprovalJoinReqURI))
	return err
}

// RestrictChatSetting 查询群禁言状态
func (o *openAPIv2) RestrictChatSetting(ctx context.Context, groupOpenID string) (*dto.RestrictChatSetting, error) {
	resp, err := o.request(ctx).
		SetResult(dto.RestrictChatSetting{}).
		SetPathParam("group_id", groupOpenID).
		Get(o.getURL(groupRestrictChatURI))
	if err != nil {
		return nil, err
	}
	return resp.Result().(*dto.RestrictChatSetting), nil
}

// SetRestrictChatSetting 设置群成员禁言
func (o *openAPIv2) SetRestrictChatSetting(ctx context.Context, groupOpenID string, setting *dto.RestrictChatSetting) error {
	_, err := o.request(ctx).
		SetPathParam("group_id", groupOpenID).
		SetBody(setting).
		Post(o.getURL(groupRestrictChatURI))
	return err
}

// CreateJoinApprovalStrategy 创建入群自动审批策略
func (o *openAPIv2) CreateJoinApprovalStrategy(ctx context.Context, req *dto.CreateJoinApprovalStrategyRequest) (*dto.CreateJoinApprovalStrategyResponse, error) {
	resp, err := o.request(ctx).
		SetResult(dto.CreateJoinApprovalStrategyResponse{}).
		SetBody(req).
		Post(o.getURL(groupJoinApprovalStrategyURI))
	if err != nil {
		return nil, err
	}
	return resp.Result().(*dto.CreateJoinApprovalStrategyResponse), nil
}

// ListJoinApprovalStrategy 查询入群自动审批策略列表
func (o *openAPIv2) ListJoinApprovalStrategy(ctx context.Context, cursor string, limit int) (*dto.JoinApprovalStrategyList, error) {
	request := o.request(ctx).
		SetResult(dto.JoinApprovalStrategyList{})
	if cursor != "" {
		request = request.SetQueryParam("cursor", cursor)
	}
	if limit > 0 {
		request = request.SetQueryParam("limit", strconv.Itoa(limit))
	}
	resp, err := request.Get(o.getURL(groupJoinApprovalStrategyURI))
	if err != nil {
		return nil, err
	}
	return resp.Result().(*dto.JoinApprovalStrategyList), nil
}

// UpdateJoinApprovalStrategy 修改入群自动审批策略
func (o *openAPIv2) UpdateJoinApprovalStrategy(ctx context.Context, strategyID string, req *dto.UpdateJoinApprovalStrategyRequest) (*dto.UpdateJoinApprovalStrategyResponse, error) {
	resp, err := o.request(ctx).
		SetResult(dto.UpdateJoinApprovalStrategyResponse{}).
		SetPathParam("strategy_id", strategyID).
		SetBody(req).
		Patch(o.getURL(groupJoinApprovalStrategyItemURI))
	if err != nil {
		return nil, err
	}
	return resp.Result().(*dto.UpdateJoinApprovalStrategyResponse), nil
}

// ExecuteJoinApprovalStrategy 执行入群自动审批策略（异步全量扫描）
func (o *openAPIv2) ExecuteJoinApprovalStrategy(ctx context.Context, strategyID string) error {
	_, err := o.request(ctx).
		SetPathParam("strategy_id", strategyID).
		Post(o.getURL(groupJoinApprovalStrategyExecURI))
	return err
}

// UpdateJoinApprovalStrategyWhitelist 修改入群自动审批策略白名单号码
func (o *openAPIv2) UpdateJoinApprovalStrategyWhitelist(ctx context.Context, strategyID string, req *dto.WhitelistUsersRequest) (*dto.WhitelistUsersResponse, error) {
	resp, err := o.request(ctx).
		SetResult(dto.WhitelistUsersResponse{}).
		SetPathParam("strategy_id", strategyID).
		SetBody(req).
		Post(o.getURL(groupJoinApprovalStrategyWhiteURI))
	if err != nil {
		return nil, err
	}
	return resp.Result().(*dto.WhitelistUsersResponse), nil
}

// DeleteJoinApprovalStrategy 删除入群自动审批策略
func (o *openAPIv2) DeleteJoinApprovalStrategy(ctx context.Context, strategyID string) error {
	_, err := o.request(ctx).
		SetPathParam("strategy_id", strategyID).
		Delete(o.getURL(groupJoinApprovalStrategyItemURI))
	return err
}

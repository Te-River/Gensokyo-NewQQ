package v1

import (
	"context"
	"fmt"

	"github.com/tencent-connect/botgo/dto"
)

// GroupInfo v1 不支持群聊接口
func (o *openAPI) GroupInfo(ctx context.Context, groupOpenID string) (*dto.GroupInfo, error) {
	return nil, fmt.Errorf("v1 openapi does not support GroupInfo")
}

// BotInGroupState v1 不支持群聊接口
func (o *openAPI) BotInGroupState(ctx context.Context, groupOpenID string) (*dto.BotInGroupState, error) {
	return nil, fmt.Errorf("v1 openapi does not support BotInGroupState")
}

// JoinRequestList v1 不支持群聊接口
func (o *openAPI) JoinRequestList(ctx context.Context, groupOpenID string, nextIndex int) (*dto.JoinRequestList, error) {
	return nil, fmt.Errorf("v1 openapi does not support JoinRequestList")
}

// ApprovalJoinRequest v1 不支持群聊接口
func (o *openAPI) ApprovalJoinRequest(ctx context.Context, groupOpenID, memberOpenID string, req *dto.ApprovalJoinRequest) error {
	return fmt.Errorf("v1 openapi does not support ApprovalJoinRequest")
}

// RestrictChatSetting v1 不支持群聊接口
func (o *openAPI) RestrictChatSetting(ctx context.Context, groupOpenID string) (*dto.RestrictChatSetting, error) {
	return nil, fmt.Errorf("v1 openapi does not support RestrictChatSetting")
}

// SetRestrictChatSetting v1 不支持群聊接口
func (o *openAPI) SetRestrictChatSetting(ctx context.Context, groupOpenID string, setting *dto.RestrictChatSetting) error {
	return fmt.Errorf("v1 openapi does not support SetRestrictChatSetting")
}

// CreateJoinApprovalStrategy v1 不支持群聊接口
func (o *openAPI) CreateJoinApprovalStrategy(ctx context.Context, req *dto.CreateJoinApprovalStrategyRequest) (*dto.CreateJoinApprovalStrategyResponse, error) {
	return nil, fmt.Errorf("v1 openapi does not support CreateJoinApprovalStrategy")
}

// ListJoinApprovalStrategy v1 不支持群聊接口
func (o *openAPI) ListJoinApprovalStrategy(ctx context.Context, cursor string, limit int) (*dto.JoinApprovalStrategyList, error) {
	return nil, fmt.Errorf("v1 openapi does not support ListJoinApprovalStrategy")
}

// UpdateJoinApprovalStrategy v1 不支持群聊接口
func (o *openAPI) UpdateJoinApprovalStrategy(ctx context.Context, strategyID string, req *dto.UpdateJoinApprovalStrategyRequest) (*dto.UpdateJoinApprovalStrategyResponse, error) {
	return nil, fmt.Errorf("v1 openapi does not support UpdateJoinApprovalStrategy")
}

// ExecuteJoinApprovalStrategy v1 不支持群聊接口
func (o *openAPI) ExecuteJoinApprovalStrategy(ctx context.Context, strategyID string) error {
	return fmt.Errorf("v1 openapi does not support ExecuteJoinApprovalStrategy")
}

// UpdateJoinApprovalStrategyWhitelist v1 不支持群聊接口
func (o *openAPI) UpdateJoinApprovalStrategyWhitelist(ctx context.Context, strategyID string, req *dto.WhitelistUsersRequest) (*dto.WhitelistUsersResponse, error) {
	return nil, fmt.Errorf("v1 openapi does not support UpdateJoinApprovalStrategyWhitelist")
}

// DeleteJoinApprovalStrategy v1 不支持群聊接口
func (o *openAPI) DeleteJoinApprovalStrategy(ctx context.Context, strategyID string) error {
	return fmt.Errorf("v1 openapi does not support DeleteJoinApprovalStrategy")
}

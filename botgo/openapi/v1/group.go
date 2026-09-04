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

// GroupMemberList v1 不支持群聊接口
func (o *openAPI) GroupMemberList(ctx context.Context, groupOpenID, cursor string, limit int) (*dto.GroupMemberList, error) {
	return nil, fmt.Errorf("v1 openapi does not support GroupMemberList")
}

// GroupMemberInfo v1 不支持群聊接口
func (o *openAPI) GroupMemberInfo(ctx context.Context, groupOpenID, memberOpenID string) (*dto.GroupMember, error) {
	return nil, fmt.Errorf("v1 openapi does not support GroupMemberInfo")
}

// BatchRemoveMembers v1 不支持群聊接口
func (o *openAPI) BatchRemoveMembers(ctx context.Context, groupOpenID string, req *dto.BatchRemoveMembersRequest) (*dto.BatchRemoveMembersResponse, error) {
	return nil, fmt.Errorf("v1 openapi does not support BatchRemoveMembers")
}

// MemberBlacklistList v1 不支持群聊接口
func (o *openAPI) MemberBlacklistList(ctx context.Context, groupOpenID, cursor string, limit int) (*dto.MemberBlacklistList, error) {
	return nil, fmt.Errorf("v1 openapi does not support MemberBlacklistList")
}

// UpdateMemberBlacklist v1 不支持群聊接口
func (o *openAPI) UpdateMemberBlacklist(ctx context.Context, groupOpenID string, req *dto.MemberBlacklistRequest) (*dto.MemberBlacklistResponse, error) {
	return nil, fmt.Errorf("v1 openapi does not support UpdateMemberBlacklist")
}

// GetMenu v1 不支持群聊接口
func (o *openAPI) GetMenu(ctx context.Context) (*dto.MenuResponse, error) {
	return nil, fmt.Errorf("v1 openapi does not support GetMenu")
}

// PutMenu v1 不支持群聊接口
func (o *openAPI) PutMenu(ctx context.Context, req *dto.PutMenuRequest) (*dto.MenuVersionResponse, error) {
	return nil, fmt.Errorf("v1 openapi does not support PutMenu")
}

// GetPanels v1 不支持群聊接口
func (o *openAPI) GetPanels(ctx context.Context, scope, cursor string, limit int) (*dto.PanelList, error) {
	return nil, fmt.Errorf("v1 openapi does not support GetPanels")
}

// CreatePanel v1 不支持群聊接口
func (o *openAPI) CreatePanel(ctx context.Context, req *dto.CreatePanelRequest) (*dto.CreatePanelResponse, error) {
	return nil, fmt.Errorf("v1 openapi does not support CreatePanel")
}

// GetPanel v1 不支持群聊接口
func (o *openAPI) GetPanel(ctx context.Context, panelID string) (*dto.PanelRecordDetail, error) {
	return nil, fmt.Errorf("v1 openapi does not support GetPanel")
}

// UpdatePanel v1 不支持群聊接口
func (o *openAPI) UpdatePanel(ctx context.Context, panelID string, req *dto.UpdatePanelRequest) (*dto.PanelVersionResponse, error) {
	return nil, fmt.Errorf("v1 openapi does not support UpdatePanel")
}

// DeletePanel v1 不支持群聊接口
func (o *openAPI) DeletePanel(ctx context.Context, panelID string) error {
	return fmt.Errorf("v1 openapi does not support DeletePanel")
}

// UpdatePanelTarget v1 不支持群聊接口
func (o *openAPI) UpdatePanelTarget(ctx context.Context, panelID string, req *dto.PanelTargetRequest) error {
	return fmt.Errorf("v1 openapi does not support UpdatePanelTarget")
}

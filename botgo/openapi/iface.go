package openapi

import (
	"context"
	"time"

	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/token"
)

// RetractMessageOption 撤回消息可选参数
type RetractMessageOption int

const (
	// RetractMessageOptionHidetip 撤回消息隐藏小灰条可选参数
	RetractMessageOptionHidetip RetractMessageOption = 1
)

// OpenAPI openapi 完整实现
type OpenAPI interface {
	Base
	WebsocketAPI
	UserAPI
	MessageAPI
	WebhookAPI
	InteractionAPI
	GroupAPI
}

// Base 基础能力接口
type Base interface {
	Version() APIVersion
	Setup(token *token.Token, inSandbox bool) OpenAPI
	// WithTimeout 设置请求接口超时时间
	WithTimeout(duration time.Duration) OpenAPI
	// Transport 透传请求，如果 sdk 没有及时跟进新的接口的变更，可以使用该方法进行透传，openapi 实现时可以按需选择是否实现该接口
	Transport(ctx context.Context, method, url string, body interface{}) ([]byte, error)
	// TraceID 返回上一次请求的 trace id
	TraceID() string
}

// WebsocketAPI websocket 接入地址
type WebsocketAPI interface {
	WS(ctx context.Context, params map[string]string, body string) (*dto.WebsocketAP, error)
}

// UserAPI 用户相关接口
type UserAPI interface {
	Me(ctx context.Context) (*dto.User, error)
}

// MessageAPI 消息相关接口
type MessageAPI interface {
	RetractGroupMessage(ctx context.Context, groupID, msgID string, options ...RetractMessageOption) error
	RetractC2CMessage(ctx context.Context, userID, msgID string, options ...RetractMessageOption) error

	// PostGroupMessage 发送群消息
	PostGroupMessage(ctx context.Context, groupID string, msg dto.APIMessage) (*dto.GroupMessageResponse, error)
	// PostC2CMessage 发送C2C消息
	PostC2CMessage(ctx context.Context, userID string, msg dto.APIMessage) (*dto.C2CMessageResponse, error)
	// PostC2CMessage 发送C2CSSE消息
	PostC2CMessageSSE(ctx context.Context, userID string, msg dto.APIMessage) (*dto.C2CMessageResponse, error)

	// PostC2CStreamMessage 发送C2C流式消息（stream_messages）
	PostC2CStreamMessage(ctx context.Context, userID string, chunk *dto.StreamChunk) (*dto.C2CMessageResponse, error)

	// 如果你有 UserAPI interface，加在这里；如果没有，加在你汇总的 API interface 里
	// GenerateURLLink 获取机器人资料页分享链接
	GenerateURLLink(ctx context.Context, params *dto.GenerateURLLinkToCreate) (*dto.GenerateURLLink, error)

	// FileUploadPrepare 单聊/群聊文件分片上传 — 预上传
	FileUploadPrepare(ctx context.Context, id string, isGroup bool, req *dto.UploadPrepareRequest) (*dto.UploadPrepareResponse, error)
	// FileUploadPartFinish 单聊/群聊文件分片上传 — 通知分片完成
	FileUploadPartFinish(ctx context.Context, id string, isGroup bool, req *dto.UploadPartFinishRequest) error
	// FileUploadMerge 单聊/群聊文件分片上传 — 合并分片（返回 file_info）
	FileUploadMerge(ctx context.Context, id string, isGroup bool, req *dto.FileUploadRequest) (*dto.MediaResponse, error)
}


// InteractionAPI 互动接口
type InteractionAPI interface {
	// PutInteraction 更新互动信息
	PutInteraction(ctx context.Context, interactionID string, body string) error
}

// WebhookAPI http 事件网关相关接口
type WebhookAPI interface {
	CreateSession(ctx context.Context, identity dto.HTTPIdentity) (*dto.HTTPReady, error)
	CheckSessions(ctx context.Context) ([]*dto.HTTPSession, error)
	SessionList(ctx context.Context) ([]*dto.HTTPSession, error)
	RemoveSession(ctx context.Context, sessionID string) error
}

// GroupAPI 群聊相关接口
type GroupAPI interface {
	// GroupInfo 获取群基本信息
	GroupInfo(ctx context.Context, groupOpenID string) (*dto.GroupInfo, error)
	// BotInGroupState 获取机器人群内状态
	BotInGroupState(ctx context.Context, groupOpenID string) (*dto.BotInGroupState, error)
	// JoinRequestList 入群申请列表
	JoinRequestList(ctx context.Context, groupOpenID string, nextIndex int) (*dto.JoinRequestList, error)
	// ApprovalJoinRequest 入群申请审批 (op: "approve" / "decline")
	ApprovalJoinRequest(ctx context.Context, groupOpenID, memberOpenID string, req *dto.ApprovalJoinRequest) error
	// RestrictChatSetting 查询群禁言状态
	RestrictChatSetting(ctx context.Context, groupOpenID string) (*dto.RestrictChatSetting, error)
	// SetRestrictChatSetting 设置群成员禁言
	SetRestrictChatSetting(ctx context.Context, groupOpenID string, setting *dto.RestrictChatSetting) error
	// CreateJoinApprovalStrategy 创建入群自动审批策略
	CreateJoinApprovalStrategy(ctx context.Context, req *dto.CreateJoinApprovalStrategyRequest) (*dto.CreateJoinApprovalStrategyResponse, error)
	// ListJoinApprovalStrategy 查询入群自动审批策略列表 (cursor 分页游标, limit 单页数量 默认20 最大100)
	ListJoinApprovalStrategy(ctx context.Context, cursor string, limit int) (*dto.JoinApprovalStrategyList, error)
	// UpdateJoinApprovalStrategy 修改入群自动审批策略
	UpdateJoinApprovalStrategy(ctx context.Context, strategyID string, req *dto.UpdateJoinApprovalStrategyRequest) (*dto.UpdateJoinApprovalStrategyResponse, error)
	// ExecuteJoinApprovalStrategy 执行入群自动审批策略（对关联群全量扫描，异步约10分钟）
	ExecuteJoinApprovalStrategy(ctx context.Context, strategyID string) error
	// UpdateJoinApprovalStrategyWhitelist 修改入群自动审批策略白名单号码
	UpdateJoinApprovalStrategyWhitelist(ctx context.Context, strategyID string, req *dto.WhitelistUsersRequest) (*dto.WhitelistUsersResponse, error)
	// DeleteJoinApprovalStrategy 删除入群自动审批策略
	DeleteJoinApprovalStrategy(ctx context.Context, strategyID string) error
	// ---------- 群成员管理 ----------
	// GroupMemberList 群成员列表 (cursor 分页, 单页≤30)
	GroupMemberList(ctx context.Context, groupOpenID, cursor string, limit int) (*dto.GroupMemberList, error)
	// GroupMemberInfo 获取单个群成员信息
	GroupMemberInfo(ctx context.Context, groupOpenID, memberOpenID string) (*dto.GroupMember, error)
	// BatchRemoveMembers 批量移出群成员 (member_openids≤20)
	BatchRemoveMembers(ctx context.Context, groupOpenID string, req *dto.BatchRemoveMembersRequest) (*dto.BatchRemoveMembersResponse, error)
	// MemberBlacklistList 群黑名单列表 (cursor 分页, 单页≤100)
	MemberBlacklistList(ctx context.Context, groupOpenID, cursor string, limit int) (*dto.MemberBlacklistList, error)
	// UpdateMemberBlacklist 群黑名单增删 (op: "add"|"del", member_openids≤20)
	UpdateMemberBlacklist(ctx context.Context, groupOpenID string, req *dto.MemberBlacklistRequest) (*dto.MemberBlacklistResponse, error)
	// ---------- 自定义菜单 ----------
	// GetMenu 获取自定义菜单
	GetMenu(ctx context.Context) (*dto.MenuResponse, error)
	// PutMenu 设置自定义菜单 (覆盖式)
	PutMenu(ctx context.Context, req *dto.PutMenuRequest) (*dto.MenuVersionResponse, error)
	// ---------- 指令面板 ----------
	// GetPanels 指令面板列表 (cursor 分页)
	GetPanels(ctx context.Context, scope, cursor string, limit int) (*dto.PanelList, error)
	// CreatePanel 创建指令面板
	CreatePanel(ctx context.Context, req *dto.CreatePanelRequest) (*dto.CreatePanelResponse, error)
	// GetPanel 获取单个指令面板详情
	GetPanel(ctx context.Context, panelID string) (*dto.PanelRecordDetail, error)
	// UpdatePanel 更新指令面板元素与备注 (覆盖式,不影响关联对象)
	UpdatePanel(ctx context.Context, panelID string, req *dto.UpdatePanelRequest) (*dto.PanelVersionResponse, error)
	// DeletePanel 删除指令面板 (响应空 {})
	DeletePanel(ctx context.Context, panelID string) error
	// UpdatePanelTarget 增删指令面板关联对象 (响应无)
	UpdatePanelTarget(ctx context.Context, panelID string, req *dto.PanelTargetRequest) error
}

package dto

// GroupInfo 群基本信息
type GroupInfo struct {
	GroupOpenID     string   `json:"group_openid"`
	GroupName       string   `json:"group_name"`
	GroupFingerMemo string   `json:"group_finger_memo"`
	GroupClassText  string   `json:"group_class_text"`
	GroupTags       []string `json:"group_tags"`
	GroupMemberNum  int      `json:"group_member_num"`
}

// BotInGroupState 机器人群内状态
type BotInGroupState struct {
	GroupOpenID    string `json:"group_openid"`
	JoinTime       int64  `json:"join_time"`
	CanPush        bool   `json:"can_push"`
	PushMsgSetting string `json:"push_msg_setting"`
	Role           string `json:"role"` // owner/admin/member
}

// JoinRequest 入群申请
type JoinRequest struct {
	GroupOpenID   string `json:"group_openid"`
	JoinRequestID string `json:"join_request_id"`
	MemberOpenID  string `json:"member_openid"`
	Username      string `json:"username"`
	ApplyAt       int64  `json:"apply_at"`
	ApplySource   string `json:"apply_source"`
	InvitedBy     string `json:"invited_by"`
	VerifyInfo    string `json:"verify_info"`
	AutoApproved  bool   `json:"auto_approved"`
}

// JoinRequestList 入群申请列表响应
type JoinRequestList struct {
	JoinRequests []JoinRequest `json:"join_requests"`
	NextIndex    int           `json:"next_index"`
}

// RestrictChatSetting 禁言设置
type RestrictChatSetting struct {
	GroupOpenID    string           `json:"group_openid"`
	AllMute        bool             `json:"all_mute"`
	MemberRestrict []MemberRestrict `json:"member_restrict"`
}

// MemberRestrict 成员禁言信息
type MemberRestrict struct {
	MemberOpenID  string `json:"member_openid"`
	RestrictUntil int64  `json:"restrict_until"`
}

// GroupJoinRequestEvent GROUP_JOIN_REQUEST 事件数据
type GroupJoinRequestEvent struct {
	ID            string      `json:"id"`
	EventID       string      `json:"event_id"`
	GroupOpenID   string      `json:"group_openid"`
	JoinRequestID string      `json:"join_request_id"`
	MemberOpenID  string      `json:"member_openid"`
	Username      string      `json:"username"`
	ApplyAt       interface{} `json:"apply_at"`
	ApplySource   string      `json:"apply_source"`
	InvitedBy     string      `json:"invited_by"`
	VerifyInfo    string      `json:"verify_info"`
	AutoApproved  bool        `json:"auto_approved"`
}

// ApprovalJoinRequest 入群申请审批请求体
// op: approve 通过 / decline 拒绝
// join_request_id 为事件下发的申请 ID，decline 时可选填 reject_reason / add_to_member_blacklist
type ApprovalJoinRequest struct {
	Op                   string `json:"op"`
	JoinRequestID        string `json:"join_request_id,omitempty"`
	RejectReason         string `json:"reject_reason,omitempty"`
	AddToMemberBlacklist bool   `json:"add_to_member_blacklist,omitempty"`
}

// JoinApprovalStrategy 入群自动审批策略
// group_openids / group_ids 按创建时使用的字段返回
type JoinApprovalStrategy struct {
	StrategyID         string   `json:"strategy_id"`
	GroupOpenIDs       []string `json:"group_openids"`
	GroupIDs           []uint64 `json:"group_ids"`
	WhitelistUserCount int      `json:"whitelist_user_count"`
	IsEnable           string   `json:"is_enable"`
	ExpireAt           string   `json:"expire_at"`
	CreatedAt          string   `json:"created_at"`
	UpdatedAt          string   `json:"updated_at"`
	Remark             string   `json:"remark"`
}

// JoinApprovalStrategyList 入群自动审批策略列表响应
// next_cursor 为空串表示已到末页
type JoinApprovalStrategyList struct {
	Strategies []JoinApprovalStrategy `json:"strategies"`
	NextCursor string                 `json:"next_cursor"`
}

// CreateJoinApprovalStrategyRequest 创建入群自动审批策略请求体
// group_openids 与 group_ids 二选一必填，同时传入或均未传入均返回错误
type CreateJoinApprovalStrategyRequest struct {
	GroupOpenIDs []string `json:"group_openids,omitempty"`
	GroupIDs     []uint64 `json:"group_ids,omitempty"`
	IsEnable     string   `json:"is_enable,omitempty"` // on-启用 off-关闭，默认 on
	ExpireAt     string   `json:"expire_at,omitempty"` // RFC3339 格式，不传默认一年过期
	Remark       string   `json:"remark,omitempty"`    // 最多 255 个汉字
}

// CreateJoinApprovalStrategyResponse 创建入群自动审批策略响应
type CreateJoinApprovalStrategyResponse struct {
	StrategyID string `json:"strategy_id"`
	IsEnable   string `json:"is_enable"`
	ExpireAt   string `json:"expire_at"`
}

// GroupAction 关联群增删操作
// group_openids 与 group_ids 互斥，群标识形式须与创建时一致
type GroupAction struct {
	Op           string   `json:"op"` // add 新增关联群 / del 删除关联群
	GroupOpenIDs []string `json:"group_openids,omitempty"`
	GroupIDs     []uint64 `json:"group_ids,omitempty"`
}

// UpdateJoinApprovalStrategyRequest 修改入群自动审批策略请求体
type UpdateJoinApprovalStrategyRequest struct {
	IsEnable    string       `json:"is_enable,omitempty"`
	ExpireAt    string       `json:"expire_at,omitempty"`
	GroupAction *GroupAction `json:"group_action,omitempty"`
	Remark      string       `json:"remark,omitempty"`
}

// UpdateJoinApprovalStrategyResponse 修改入群自动审批策略响应
type UpdateJoinApprovalStrategyResponse struct {
	IsEnable string `json:"is_enable"`
	ExpireAt string `json:"expire_at"`
}

// WhitelistUsersRequest 修改入群自动审批策略白名单请求体
// whitelist_users 为 QQ 号码列表（字符串避免 JS 精度问题），单次最多 10000 个
type WhitelistUsersRequest struct {
	Op             string   `json:"op"` // add 新增号码 / del 删除号码
	WhitelistUsers []string `json:"whitelist_users"`
}

// WhitelistUsersResponse 修改入群自动审批策略白名单响应
type WhitelistUsersResponse struct {
	StrategyID         string `json:"strategy_id"`
	WhitelistUserCount int    `json:"whitelist_user_count"`
	UpdatedAt          string `json:"updated_at"`
}

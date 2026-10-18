package dto

import (
	"encoding/json"
	"fmt"
)

// VerifyInfo 入群申请的验证信息，兼容两种格式：
//   - 旧格式（纯字符串）："111"
//   - 新格式（对象）：{"method":"verify_message","verify_message":"111","review_qa_list":[...]}
type VerifyInfo struct {
	Method       string     `json:"method"`
	Message      string     `json:"verify_message"`
	ReviewQAList []ReviewQA `json:"review_qa_list"`
}

// ReviewQA 入群验证问答（method=admin_review_qa 时携带）
type ReviewQA struct {
	Question string `json:"question"`
	Answer   string `json:"answer"`
}

// UnmarshalJSON 兼容字符串与对象两种格式的 verify_info；
// 对象形态字段全空（{} / 仅 review_qa_list）也视为成功，避免整个事件解析失败。
func (v *VerifyInfo) UnmarshalJSON(data []byte) error {
	// 优先尝试对象格式（method + verify_message + review_qa_list）
	var obj struct {
		Method       string     `json:"method"`
		Message      string     `json:"verify_message"`
		ReviewQAList []ReviewQA `json:"review_qa_list"`
	}
	if err := json.Unmarshal(data, &obj); err == nil {
		v.Method = obj.Method
		v.Message = obj.Message
		v.ReviewQAList = obj.ReviewQAList
		return nil
	}
	// 回退到纯字符串格式
	var s string
	if err := json.Unmarshal(data, &s); err == nil {
		v.Message = s
		return nil
	}
	return fmt.Errorf("无法解析 verify_info: %s", data)
}

// String 返回验证消息内容（verify_message 字段）。
// 仅当 verify_info 为对象且含 verify_message 时才有内容；
// 纯字符串格式原样返回；method 只是验证方式名，不作为内容返回。
func (v *VerifyInfo) String() string {
	if v == nil {
		return ""
	}
	return v.Message
}

// GroupInfo 群基本信息
type GroupInfo struct {
	GroupOpenID     string   `json:"group_openid"`
	GroupName       string   `json:"group_name"`
	GroupFingerMemo string   `json:"group_finger_memo"`
	GroupClassText  string   `json:"group_class_text"`
	GroupTags       []string `json:"group_tags"`
	GroupMemberNum  int      `json:"group_member_num"`
}

// BotInGroupState 机器人群内状态（GET /v2/groups/{group_openid}/bot_state）
type BotInGroupState struct {
	MemberOpenID      string `json:"member_openid"`       // 机器人的 openid
	JoinedAt          string `json:"joined_at"`           // 入群时间戳（RFC3339）
	AllowProactiveMsg bool   `json:"allow_proactive_msg"` // 是否接收主动推送
	RecvMsgSetting    string `json:"recv_msg_setting"`    // all / only_mention / mention_and_context
	MemberRole        string `json:"member_role"`         // member / owner / admin
}

// AutoApprovedInfo 入群申请自动审批信息（官方类型名 AutoAppproved，仅下行事件携带）。
// 官方文档为对象 {"strategy_id":...}；服务端回退形态 true/false/null 亦容忍，
// 避免整个事件/列表反序列化失败。
type AutoApprovedInfo struct {
	StrategyID string `json:"strategy_id"` // 自动审批通过的策略 ID
	// Approved 记录"是否已自动审批"布尔语义（对象/true 形态为 true，false 形态为 false）；
	// json:"-" 不参与序列化。
	Approved bool `json:"-"`
}

// UnmarshalJSON 容忍对象 / true / false 三种形态（null 由解码器置 nil 指针，不经此处）。
func (a *AutoApprovedInfo) UnmarshalJSON(data []byte) error {
	var obj struct {
		StrategyID string `json:"strategy_id"`
	}
	if err := json.Unmarshal(data, &obj); err == nil {
		a.StrategyID = obj.StrategyID
		a.Approved = true
		return nil
	}
	var b bool
	if err := json.Unmarshal(data, &b); err == nil {
		a.Approved = b
		return nil
	}
	return fmt.Errorf("无法解析 auto_approved: %s", data)
}

// JoinRequest 入群申请
type JoinRequest struct {
	JoinRequestID string            `json:"join_request_id"`
	RiskTips      string            `json:"risk_tips"`
	UnionOpenID   string            `json:"union_openid"`
	MemberOpenID  string            `json:"member_openid"`
	Username      string            `json:"username"`
	ApplyAt       string            `json:"apply_at"` // RFC3339
	ApplySource   string            `json:"apply_source"`
	InvitedBy     string            `json:"invited_by"`
	Bot           bool              `json:"bot"`
	VerifyInfo    VerifyInfo        `json:"verify_info"`
	AutoApproved  *AutoApprovedInfo `json:"auto_approved"`
}

// JoinRequestList 入群申请列表响应
type JoinRequestList struct {
	List       []JoinRequest `json:"list"`
	NextCursor string        `json:"next_cursor"` // 空串表示已到末页
}

// RestrictChatSetting 群禁言设置：GET 响应（global_rule + members）与
// POST 请求体（官方只接受 members，按成员 op 增量生效）共用同一结构，
// POST 时 GlobalRule 为 nil 不参与序列化。
type RestrictChatSetting struct {
	GlobalRule *GlobalMuteRule   `json:"global_rule,omitempty"`
	Members    []MemberMuteState `json:"members,omitempty"`
}

// GlobalMuteRule 群级禁言规则
type GlobalMuteRule struct {
	Mode           string              `json:"mode"` // none 未开启 / always 始终禁言 / schedule 定时禁言
	ScheduleRules  []MuteScheduleRule  `json:"schedule_rules"`
	RecurringRules []MuteRecurringRule `json:"recurring_rules"`
}

// MuteScheduleRule 定时禁言规则
type MuteScheduleRule struct {
	TaskID  string `json:"task_id"`
	StartAt string `json:"start_at"` // RFC3339
	EndAt   string `json:"end_at"`   // RFC3339
	Enabled bool   `json:"enabled"`
}

// MuteRecurringRule 周期性禁言规则
type MuteRecurringRule struct {
	TaskID    string `json:"task_id"`
	Weekdays  []int  `json:"weekdays"`   // 1~7（1=周一）
	StartTime string `json:"start_time"` // HH:mm
	EndTime   string `json:"end_time"`   // HH:mm，小于 start_time 表示跨天
	Enabled   bool   `json:"enabled"`
}

// MemberMuteState 成员禁言条目：GET 响应携带 username / union_openid；
// POST 请求体（SetMemberMuteState 语义）用 op 控制增/改/删，op=del 时 mute_expire_at 传空串立即解除。
type MemberMuteState struct {
	Op           string `json:"op,omitempty"` // add 增加禁言 / update 更新到期时间 / del 解除禁言
	MemberOpenID string `json:"member_openid"`
	MuteExpireAt string `json:"mute_expire_at"` // RFC3339
	Username     string `json:"username,omitempty"`
	UnionOpenID  string `json:"union_openid,omitempty"`
}

// GroupJoinRequestEvent GROUP_JOIN_REQUEST 事件数据
type GroupJoinRequestEvent struct {
	ID            string            `json:"id"`
	EventID       string            `json:"event_id"`
	GroupOpenID   string            `json:"group_openid"`
	JoinRequestID string            `json:"join_request_id"`
	MemberOpenID  string            `json:"member_openid"`
	Username      string            `json:"username"`
	ApplyAt       interface{}       `json:"apply_at"` // RFC3339 字符串（兼容历史数字时间戳）
	ApplySource   string            `json:"apply_source"`
	InvitedBy     string            `json:"invited_by"`
	RiskTips      string            `json:"risk_tips"`
	UnionOpenID   string            `json:"union_openid"`
	Bot           bool              `json:"bot"`
	VerifyInfo    VerifyInfo        `json:"verify_info"`
	AutoApproved  *AutoApprovedInfo `json:"auto_approved"` // 仅下行事件携带
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

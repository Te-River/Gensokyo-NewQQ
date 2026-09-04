package dto

// GroupMember 群成员信息
type GroupMember struct {
	MemberOpenID string `json:"member_openid"` // member_openid
	Username     string `json:"username"`      // username
	MemberRole   string `json:"member_role"`   // member_role: member|owner|admin
	Bot          bool   `json:"bot"`           // bot
	JoinedAt     string `json:"joined_at"`     // joined_at (RFC3339 字符串)
	UnionOpenID  string `json:"union_openid"`  // union_openid
}

// GroupMemberList 成员列表响应 (cursor 分页, 单页≤30)
type GroupMemberList struct {
	Members    []GroupMember `json:"members"`
	NextCursor string        `json:"next_cursor"`
}

// BatchRemoveMembersRequest 批量移出成员请求 (member_openids≤20)
type BatchRemoveMembersRequest struct {
	MemberOpenIDs        []string `json:"member_openids"`
	AddToMemberBlacklist bool     `json:"add_to_member_blacklist,omitempty"`
}

// BatchRemoveMembersResponse 批量移出响应
type BatchRemoveMembersResponse struct {
	RemoveMembersResult             string   `json:"remove_members_result"` // "success"
	AddToMemberBlacklistFailOpenids []string `json:"add_to_member_blacklist_fail_openids"`
}

// MemberBlacklistRequest 黑名单增删请求 (op: "add"|"del", member_openids≤20)
type MemberBlacklistRequest struct {
	Op            string   `json:"op"`
	MemberOpenIDs []string `json:"member_openids"`
}

// MemberBlacklistResponse 黑名单增删响应
type MemberBlacklistResponse struct {
	FailOpenids []string `json:"fail_openids"`
}

// BlacklistUser 黑名单用户
type BlacklistUser struct {
	UnionOpenID  string `json:"union_openid"`
	MemberOpenID string `json:"member_openid"`
	Username     string `json:"username"`
	BannedAt     string `json:"banned_at"` // RFC3339 字符串
	Bot          bool   `json:"bot"`
}

// MemberBlacklistList 黑名单列表响应 (cursor 分页, 单页≤100)
type MemberBlacklistList struct {
	Users      []BlacklistUser `json:"users"`
	NextCursor string          `json:"next_cursor"`
}

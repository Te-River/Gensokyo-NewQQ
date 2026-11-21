package dto

// User 用户
type User struct {
	ID               string `json:"id"`
	Username         string `json:"username"`
	MemberNickname   string `json:"member_nickname,omitempty"` // 群内昵称（2026-11-16 群消息事件新增）
	Avatar           string `json:"avatar"`
	Bot              bool   `json:"bot"`
	UnionOpenID      string `json:"union_openid"`       // 特殊关联应用的 openid
	UnionUserAccount string `json:"union_user_account"` // 机器人关联的用户信息，与union_openid关联的应用是同一个
	IsYou            bool   `json:"is_you,omitempty"`   // 是否 @ 当前机器人
	MemberRole       string `json:"member_role,omitempty"` // 群成员身份: owner/member/admin
	UserOpenID       string `json:"user_openid,omitempty"` // 用户 openid
	MemberOpenID     string `json:"member_openid,omitempty"` // 群成员 openid
}

// UserInfo 用户基本信息（GET /v2/users/{user_openid}/info，2026-11-16 官方新增）
type UserInfo struct {
	Username          string `json:"username"`
	AllowProactiveMsg bool   `json:"allow_proactive_msg"`
	Avatar            string `json:"avatar"`
}

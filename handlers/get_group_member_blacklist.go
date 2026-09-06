package handlers

import (
	"context"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/openapi"
)

func init() {
	callapi.RegisterHandler("get_group_member_blacklist", GetGroupMemberBlacklist)
}

// BlacklistUserItem OneBot 群黑名单条目
type BlacklistUserItem struct {
	UserID       int64  `json:"user_id"`       // 虚拟用户 ID
	UnionOpenID  string `json:"union_openid"`  // 官方 union_openid 原样透传
	MemberOpenID string `json:"member_openid"` // 官方 member_openid 原样透传
	Username     string `json:"username"`
	BannedAt     string `json:"banned_at"` // RFC3339 字符串原样透传
	Bot          bool   `json:"bot"`
}

// GroupMemberBlacklistResponse 群黑名单列表响应
type GroupMemberBlacklistResponse struct {
	Users      []BlacklistUserItem `json:"users"`
	NextCursor string              `json:"next_cursor"`
}

// GetGroupMemberBlacklist 拉取群黑名单列表（官方 member_blacklist，游标分页单页≤100）。
// params: group_id(虚拟/真实群ID), cursor(可选分页游标), limit(可选单页数量,默认 100)
func GetGroupMemberBlacklist(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	groupID, _ := message.Params.GroupID.(string)

	groupOpenID, err := resolveGroupOpenID(groupID)
	if err != nil {
		mylog.Printf("get_group_member_blacklist: 反查 group_openid 失败: %v", err)
		return sendActionResult(client, message, "无法反查群 OpenID", 100)
	}

	limit := message.Params.Limit
	if limit <= 0 {
		limit = 100 // 官方单页上限
	}

	list, err := apiv2.MemberBlacklistList(context.TODO(), groupOpenID, message.Params.Cursor, limit)
	if err != nil {
		mylog.Printf("get_group_member_blacklist: 拉取黑名单失败: %v", err)
		return sendActionResult(client, message, err.Error(), 100)
	}

	resp := GroupMemberBlacklistResponse{NextCursor: list.NextCursor, Users: []BlacklistUserItem{}}
	for _, user := range list.Users {
		// 黑名单成员 openid → 虚拟 user_id（与入站上报口径一致,可直接回传增删接口）
		userID, err := idmap.StoreIDv2(user.MemberOpenID)
		if err != nil {
			// 反查失败跳过该条目,避免 user_id=0 进入响应被误用
			mylog.Printf("get_group_member_blacklist: member_openid 转虚拟 ID 失败,跳过: %v", err)
			continue
		}
		resp.Users = append(resp.Users, BlacklistUserItem{
			UserID:       userID,
			UnionOpenID:  user.UnionOpenID,
			MemberOpenID: user.MemberOpenID,
			Username:     user.Username,
			BannedAt:     user.BannedAt,
			Bot:          user.Bot,
		})
	}
	mylog.Printf("get_group_member_blacklist: group=%s 共 %d 条记录", groupOpenID, len(resp.Users))
	return sendActionResultWithData(client, message, resp)
}

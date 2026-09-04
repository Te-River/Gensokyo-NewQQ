package handlers

import (
	"context"
	"strconv"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

func init() {
	callapi.RegisterHandler("set_group_kick", SetGroupKick)
}

// SetGroupKick 批量移出群成员（官方 batch_remove_members，单批≤20）。
// params: group_id(虚拟/真实群ID), user_id(单个,可选) 或 user_ids(批量数组,可选), add_blacklist(移出同时拉黑)
// user_id 与 user_ids 二选一，同时存在时合并去重，超 20 截断并警告。
func SetGroupKick(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	groupID, _ := message.Params.GroupID.(string)

	// 反查真实 OpenID（32 位原生 OpenID 直接使用）
	groupOpenID, err := resolveGroupOpenID(groupID)
	if err != nil {
		mylog.Printf("set_group_kick: 反查 group_openid 失败: %v", err)
		return sendActionResult(client, message, "无法反查群 OpenID", 100)
	}

	// 收集成员：user_id（单个）+ user_ids（批量），合并去重保序、截断
	var virtualIDs []string
	if uid, ok := message.Params.UserID.(string); ok && uid != "" {
		virtualIDs = append(virtualIDs, uid)
	}
	virtualIDs = append(virtualIDs, message.Params.UserIDs...)
	virtualIDs = dedupeTruncateUserIDs(virtualIDs)
	if len(virtualIDs) == 0 {
		mylog.Printf("set_group_kick: user_id 与 user_ids 不能同时为空")
		return sendActionResult(client, message, "user_id 或 user_ids 至少提供一个", 100)
	}

	// 逐个反查成员 OpenID，失败的跳过并日志、不中断整批
	var openIDs []string
	kicked := make([]interface{}, 0, len(virtualIDs))
	for _, vid := range virtualIDs {
		memberOpenID, err := resolveMemberOpenID(vid)
		if err != nil {
			mylog.Printf("set_group_kick: user_id=%s 反查失败,已跳过: %v", vid, err)
			continue
		}
		openIDs = append(openIDs, memberOpenID)
		if n, err := strconv.ParseInt(vid, 10, 64); err == nil {
			kicked = append(kicked, n)
		} else {
			kicked = append(kicked, vid)
		}
	}
	if len(openIDs) == 0 {
		mylog.Printf("set_group_kick: 所有用户反查 OpenID 失败")
		return sendActionResult(client, message, "所有用户反查 OpenID 失败", 100)
	}

	req := &dto.BatchRemoveMembersRequest{
		MemberOpenIDs:        openIDs,
		AddToMemberBlacklist: message.Params.AddBlacklist,
	}
	resp, err := apiv2.BatchRemoveMembers(context.TODO(), groupOpenID, req)
	if err != nil {
		mylog.Printf("set_group_kick: 批量移出失败: %v", err)
		return sendActionResult(client, message, err.Error(), 100)
	}

	failOpenids := resp.AddToMemberBlacklistFailOpenids
	if failOpenids == nil {
		failOpenids = []string{}
	}
	data := struct {
		RemoveMembersResult             string        `json:"remove_members_result"`
		AddToMemberBlacklistFailOpenids []string      `json:"add_to_member_blacklist_fail_openids"`
		Kicked                          []interface{} `json:"kicked"`
	}{
		RemoveMembersResult:             resp.RemoveMembersResult,
		AddToMemberBlacklistFailOpenids: failOpenids,
		Kicked:                          kicked,
	}
	mylog.Printf("set_group_kick: 已提交移出 group=%s 共 %d 人(add_blacklist=%v)", groupOpenID, len(openIDs), message.Params.AddBlacklist)
	return sendActionResultWithData(client, message, data)
}

// dedupeTruncateUserIDs 合并去重保序、过滤空项，超官方上限 20 截断并警告。
func dedupeTruncateUserIDs(ids []string) []string {
	seen := make(map[string]struct{}, len(ids))
	result := make([]string, 0, len(ids))
	for _, id := range ids {
		if id == "" {
			continue
		}
		if _, ok := seen[id]; ok {
			continue
		}
		seen[id] = struct{}{}
		result = append(result, id)
	}
	if len(result) > 20 {
		mylog.Printf("成员数 %d 超过官方单批上限 20,已截断", len(result))
		result = result[:20]
	}
	return result
}

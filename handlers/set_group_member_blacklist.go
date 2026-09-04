package handlers

import (
	"context"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

func init() {
	callapi.RegisterHandler("set_group_member_blacklist", SetGroupMemberBlacklist)
}

// SetGroupMemberBlacklist 群黑名单增删（官方 member_blacklist，op: add|del，单批≤20）。
// params: group_id(虚拟/真实群ID), op("add"|"del"), user_id(单个,可选) 或 user_ids(批量数组,可选)
// user_id 与 user_ids 二选一，同时存在时合并去重，超 20 截断并警告。
func SetGroupMemberBlacklist(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	groupID, _ := message.Params.GroupID.(string)

	if message.Params.Op != "add" && message.Params.Op != "del" {
		mylog.Printf("set_group_member_blacklist: op 参数无效: %s", message.Params.Op)
		return sendActionResult(client, message, "op 必须为 add 或 del", 100)
	}

	groupOpenID, err := resolveGroupOpenID(groupID)
	if err != nil {
		mylog.Printf("set_group_member_blacklist: 反查 group_openid 失败: %v", err)
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
		mylog.Printf("set_group_member_blacklist: user_id 与 user_ids 不能同时为空")
		return sendActionResult(client, message, "user_id 或 user_ids 至少提供一个", 100)
	}

	// 逐个反查成员 OpenID，失败的跳过并日志、不中断整批
	var openIDs []string
	for _, vid := range virtualIDs {
		memberOpenID, err := resolveMemberOpenID(vid)
		if err != nil {
			mylog.Printf("set_group_member_blacklist: user_id=%s 反查失败,已跳过: %v", vid, err)
			continue
		}
		openIDs = append(openIDs, memberOpenID)
	}
	if len(openIDs) == 0 {
		mylog.Printf("set_group_member_blacklist: 所有用户反查 OpenID 失败")
		return sendActionResult(client, message, "所有用户反查 OpenID 失败", 100)
	}

	req := &dto.MemberBlacklistRequest{
		Op:            message.Params.Op,
		MemberOpenIDs: openIDs,
	}
	resp, err := apiv2.UpdateMemberBlacklist(context.TODO(), groupOpenID, req)
	if err != nil {
		mylog.Printf("set_group_member_blacklist: 黑名单操作失败 op=%s: %v", message.Params.Op, err)
		return sendActionResult(client, message, err.Error(), 100)
	}

	failOpenids := resp.FailOpenids
	if failOpenids == nil {
		failOpenids = []string{}
	}
	data := struct {
		FailOpenids []string `json:"fail_openids"`
	}{
		FailOpenids: failOpenids,
	}
	mylog.Printf("set_group_member_blacklist: 已提交黑名单 op=%s group=%s 共 %d 人,失败 %d 人", message.Params.Op, groupOpenID, len(openIDs), len(failOpenids))
	return sendActionResultWithData(client, message, data)
}

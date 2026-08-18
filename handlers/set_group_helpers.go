package handlers

import (
	"context"
	"fmt"
	"time"

	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

// ---------- set_group 系列共享底层 ----------
// 被 [CQ:set_group,action=...] CQ 码动作与 set_group_ban /
// set_group_whole_ban / set_group_add_request handler 共用，
// 保证 CQ 码路径与 API 路径行为完全一致。

// resolveGroupOpenID 将虚拟群 ID 反查为真实群 OpenID（32 位原生 OpenID 直接使用）
func resolveGroupOpenID(groupID string) (string, error) {
	if len(groupID) == 32 {
		return groupID, nil
	}
	realID, err := idmap.RetrieveRowByIDv2(groupID)
	if err != nil || realID == "" {
		return "", fmt.Errorf("反查 group_openid 失败: %w", err)
	}
	return realID, nil
}

// resolveMemberOpenID 将虚拟用户 ID 反查为真实用户 OpenID（32 位原生 OpenID 直接使用）
func resolveMemberOpenID(userID string) (string, error) {
	if len(userID) == 32 {
		return userID, nil
	}
	realID, err := idmap.RetrieveRowByIDv2(userID)
	if err != nil || realID == "" {
		return "", fmt.Errorf("反查 member_openid 失败: %w", err)
	}
	return realID, nil
}

// applyRestrictChatSetting 查询当前群禁言设置并应用变更后提交。
// memberOpenID 非空时按 duration 增/删该成员禁言条目（duration<=0 表示解除）；
// allMute 非 nil 时切换全员禁言开关；两者可同时生效，均保留对方已有设置。
func applyRestrictChatSetting(apiv2 openapi.OpenAPI, groupOpenID string, memberOpenID string, duration int, allMute *bool) error {
	setting := &dto.RestrictChatSetting{GroupOpenID: groupOpenID}
	// 查询当前设置作为基础，避免覆盖已有禁言条目
	if cur, err := apiv2.RestrictChatSetting(context.TODO(), groupOpenID); err == nil {
		setting.MemberRestrict = cur.MemberRestrict
	} else {
		mylog.Printf("set_group: 查询禁言状态失败: %v", err)
	}
	if allMute != nil {
		setting.AllMute = *allMute
	}
	if memberOpenID != "" {
		// 先移除该成员旧条目，避免重复
		var kept []dto.MemberRestrict
		for _, m := range setting.MemberRestrict {
			if m.MemberOpenID != memberOpenID {
				kept = append(kept, m)
			}
		}
		setting.MemberRestrict = kept
		if duration > 0 {
			setting.MemberRestrict = append(setting.MemberRestrict, dto.MemberRestrict{
				MemberOpenID:  memberOpenID,
				RestrictUntil: time.Now().Unix() + int64(duration),
			})
		}
	}
	return apiv2.SetRestrictChatSetting(context.TODO(), groupOpenID, setting)
}

// approveJoinRequest 审批入群申请（approve=true 通过 / false 拒绝）。
// reason 为拒绝理由；blacklist 非 nil 时同时将申请人加入群黑名单。
func approveJoinRequest(apiv2 openapi.OpenAPI, groupOpenID, memberOpenID, flag string, approve bool, reason string, blacklist *bool) error {
	op := "decline"
	if approve {
		op = "approve"
	}
	req := &dto.ApprovalJoinRequest{
		Op:            op,
		JoinRequestID: flag,
		RejectReason:  reason,
	}
	if blacklist != nil {
		req.AddToMemberBlacklist = *blacklist
	}
	return apiv2.ApprovalJoinRequest(context.TODO(), groupOpenID, memberOpenID, req)
}

// buildSetGroupCQCode 将消息段 data 字段拼装为 [CQ:set_group,key=value,...] 字符串，
// 供消息段数组（[]interface{}）与 TRSS（map）路径还原 CQ 码使用；
// 字段顺序固定，解析端 cqParseParams 顺序无关。无任何参数时返回空串。
func buildSetGroupCQCode(dataMap map[string]interface{}) string {
	cq := "[CQ:set_group"
	for _, k := range []string{"action", "group_id", "user_id", "duration", "enable", "approve", "flag", "reason", "add_to_member_blacklist", "strategy_id"} {
		if v, ok := dataMap[k].(string); ok && v != "" {
			cq += "," + k + "=" + v
		}
	}
	cq += "]"
	if cq == "[CQ:set_group]" {
		return "" // 无任何参数，丢弃
	}
	return cq
}

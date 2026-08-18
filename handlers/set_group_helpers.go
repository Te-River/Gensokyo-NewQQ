package handlers

import (
	"context"
	"fmt"
	"strconv"
	"strings"
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
// 解除禁言依赖当前设置列表，查询失败时中止提交，避免空列表清空全员禁言。
func applyRestrictChatSetting(apiv2 openapi.OpenAPI, groupOpenID string, memberOpenID string, duration int, allMute *bool) error {
	setting := &dto.RestrictChatSetting{GroupOpenID: groupOpenID}
	// 查询当前设置作为基础，避免覆盖已有禁言条目
	if cur, err := apiv2.RestrictChatSetting(context.TODO(), groupOpenID); err == nil {
		setting.MemberRestrict = cur.MemberRestrict
	} else if memberOpenID != "" && duration <= 0 {
		// 解禁路径依赖当前列表（先移除该成员条目），查询失败时无法安全提交，
		// 空列表会清空群里所有已设置的禁言，直接中止并返回错误
		return fmt.Errorf("查询禁言状态失败，无法安全解除禁言: %w", err)
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
// 字段顺序固定，解析端 cqParseParams 顺序无关。
// 支持 string / 数字 / 布尔值，值中的逗号与右括号做 CQ 转义（&#44;/&#93;），
// 避免含逗号的值（如 reason）在 cqParseParams 按逗号切分时被截断。
// 无任何参数时返回空串。
func buildSetGroupCQCode(dataMap map[string]interface{}) string {
	cq := "[CQ:set_group"
	for _, k := range []string{"action", "group_id", "user_id", "duration", "enable", "approve", "flag", "reason", "add_to_member_blacklist", "strategy_id"} {
		v, ok := dataMap[k]
		if !ok {
			continue
		}
		var sv string
		switch tv := v.(type) {
		case string:
			sv = tv
		case float64: // JSON 数字
			sv = strconv.FormatFloat(tv, 'f', -1, 64)
		case bool:
			sv = strconv.FormatBool(tv)
		default:
			continue // 其他类型忽略
		}
		if sv == "" {
			continue
		}
		// 转义逗号与右括号，防止解析时截断；& 一并转义避免二次解析歧义
		sv = strings.ReplaceAll(sv, "&", "&amp;")
		sv = strings.ReplaceAll(sv, ",", "&#44;")
		sv = strings.ReplaceAll(sv, "]", "&#93;")
		cq += "," + k + "=" + sv
	}
	cq += "]"
	if cq == "[CQ:set_group]" {
		return "" // 无任何参数，丢弃
	}
	return cq
}

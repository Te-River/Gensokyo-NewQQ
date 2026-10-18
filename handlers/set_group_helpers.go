package handlers

import (
	"context"
	"errors"
	"fmt"
	"strconv"
	"strings"
	"time"

	"github.com/hoshinonyaruko/gensokyo/idmap"
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

// errWholeBanUnsupported 官方 POST /restrict_chat_setting 请求体只有 members，
// 无全员禁言写入字段（global_rule.mode 仅出现在查询响应中）
var errWholeBanUnsupported = errors.New("官方未提供全员禁言写入字段，仅支持按成员禁言")

// applyRestrictChatSetting 按官方增量语义提交群成员禁言变更。
// memberOpenID 非空时按 duration 增/删该成员禁言条目：
// duration>0 → op=add + mute_expire_at=now+duration（RFC3339）；
// duration<=0 → op=del + 空 mute_expire_at（立即解除）。
// 官方 POST 按成员 op 增量生效，无需先查询当前设置（旧实现"查询失败即中止"的保护不再必要）。
// allMute 非 nil 表示调用方要求切换全员禁言：官方无对应写入字段，固定返回 errWholeBanUnsupported。
func applyRestrictChatSetting(apiv2 openapi.OpenAPI, groupOpenID string, memberOpenID string, duration int, allMute *bool) error {
	if allMute != nil {
		return errWholeBanUnsupported
	}
	item := dto.MemberMuteState{MemberOpenID: memberOpenID}
	if duration > 0 {
		item.Op = "add"
		item.MuteExpireAt = time.Now().Add(time.Duration(duration) * time.Second).Format(time.RFC3339)
	} else {
		item.Op = "del"
	}
	return apiv2.SetRestrictChatSetting(context.TODO(), groupOpenID, &dto.RestrictChatSetting{
		Members: []dto.MemberMuteState{item},
	})
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
	for _, k := range []string{"action", "group_id", "user_id", "user_ids", "duration", "enable", "approve", "flag", "reason", "add_to_member_blacklist", "add_blacklist", "strategy_id"} {
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
		case []interface{}: // 消息段数组路径的列表值(如 user_ids),逗号拼接还原为 CQ 批量参数
			var parts []string
			for _, item := range tv {
				switch iv := item.(type) {
				case string:
					parts = append(parts, iv)
				case float64:
					parts = append(parts, strconv.FormatFloat(iv, 'f', -1, 64))
				}
			}
			sv = strings.Join(parts, ",")
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

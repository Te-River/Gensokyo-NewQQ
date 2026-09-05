package handlers

// cqparse_exec.go：cqparse Deps 的 handlers 侧实现（架构设计 §3/§6.3/§10 任务 4）。
// - 动作执行器由 cqcode.go 的 cq*Action 系列迁移而来，行为位等价；
//   签名改为接收 cqparse.PendingAction（参数来源 Pending.Params、日志原文 Pending.Raw、
//   缺省群 Pending.DefaultGroupID），apiv2 经闭包注入。
// - 修 M3：remove 失败路径不再 return match 泄漏（统一"无论成败移除+日志"）；
//   新架构下码在 Splice 阶段已移除，执行器只负责日志。
// - 修 M4：whole_ban enable 非法 → 日志 + 跳过执行，码不回填正文。
// - avatar 反查失败兜底：不再产出破损 URL（修 avatar.go:101,132 掩盖问题）。

import (
	"context"
	"fmt"
	"strconv"
	"strings"

	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/handlers/cqparse"
	"github.com/hoshinonyaruko/gensokyo/echo"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

// DefaultDeps 构造注入 apiv2 的执行依赖（每次发送调用构造，闭包携带 apiv2）。
func DefaultDeps(apiv2 openapi.OpenAPI) *cqparse.Deps {
	return &cqparse.Deps{
		GroupInfo: apiv2GroupInfoFetcher{apiv2: apiv2},
		AvatarURL: defaultAvatarURL,
		RunAction: func(p cqparse.PendingAction, eventID *string) cqparse.ExecOutcome {
			return runPendingAction(p, apiv2, eventID)
		},
	}
}

// apiv2GroupInfoFetcher 复用 get_group_info.go 同源取数（apiv2.GroupInfo）。
type apiv2GroupInfoFetcher struct{ apiv2 openapi.OpenAPI }

func (f apiv2GroupInfoFetcher) GroupInfo(ctx context.Context, groupOpenID string) (*cqparse.GroupInfoData, error) {
	// cqparse 侧 resolveGroupOpenID 已统一反查，此处 32 位/虚拟 ID 双格式
	// 守卫保留为兜底（防御性，与 get_group_info.go 同源）
	if len(groupOpenID) != 32 {
		realID, err := idmap.RetrieveRowByIDv2(groupOpenID)
		if err != nil || realID == "" {
			return nil, fmt.Errorf("反查 group_openid 失败: %w", err)
		}
		groupOpenID = realID
	}
	info, err := f.apiv2.GroupInfo(ctx, groupOpenID)
	if err != nil {
		return nil, err
	}
	return &cqparse.GroupInfoData{
		Name:        info.GroupName,
		Memo:        info.GroupFingerMemo,
		MemberCount: info.GroupMemberNum,
	}, nil
}

// defaultAvatarURL 头像 URL 生成（ProcessCQAvatar/GetAvatarCQCode 同源逻辑收敛）。
// 返回值剥离 scheme，与 foundItems url_images 消费方（自动补 https:// 前缀）兼容。
func defaultAvatarURL(qq, groupID string, hasGroup bool) (string, error) {
	var originalUserID string
	var err error
	// "690426430" 为全仓既有 idmap 哨兵群号（avatar.go:27 同源），
	// 跨群反查失败/私聊路径以它作 idmap-pro 的回退 key（历史数据兼容）
	const sentinelGroupID = "690426430"
	if config.GetIdmapPro() {
		if hasGroup && groupID != "" {
			_, originalUserID, err = idmap.RetrieveRowByIDv2Pro(groupID, qq)
			if err != nil {
				// 跨群反查失败回退私聊路径
				_, originalUserID, err = idmap.RetrieveRowByIDv2Pro(sentinelGroupID, qq)
			}
		} else {
			_, originalUserID, err = idmap.RetrieveRowByIDv2Pro(sentinelGroupID, qq)
		}
	} else {
		originalUserID, err = idmap.RetrieveRowByIDv2(qq)
	}
	if err != nil || originalUserID == "" {
		return "", fmt.Errorf("avatar 反查 user_id=%s 失败: %w", qq, err)
	}
	avatarURL, err := GenerateAvatarURLV2(originalUserID)
	if err != nil {
		return "", err
	}
	avatarURL = strings.TrimPrefix(avatarURL, "https://")
	avatarURL = strings.TrimPrefix(avatarURL, "http://")
	return avatarURL, nil
}

// pickLastRealGroupID 返回动作产物中最后一个非空 RealGroupID（last-wins）。
// 对齐 legacy ProcessOutboundCQCodes 逐码覆写 realGroupID 的语义（cqcode.go:268）。
func pickLastRealGroupID(outs []cqparse.ExecOutcome) string {
	var realGroupID string
	for _, o := range outs {
		if o.RealGroupID != "" {
			realGroupID = o.RealGroupID
		}
	}
	return realGroupID
}

// runPendingAction 按动作分发执行（与 ProcessOutboundCQCodes 分发一致）。
func runPendingAction(p cqparse.PendingAction, apiv2 openapi.OpenAPI, eventID *string) cqparse.ExecOutcome {
	switch p.Action {
	case "member":
		return cqMemberExec(p, eventID)
	case "remove":
		return cqRemoveExec(p, apiv2)
	case "set_group":
		return cqSetGroupExec(p, apiv2)
	default:
		mylog.Printf("[cqparse] 未知动作 %s: %s", p.Action, p.Raw)
		return cqparse.ExecOutcome{}
	}
}

// cqMemberExec 处理 member type=add/remove（迁移自 cqMemberAction，行为位等价）。
func cqMemberExec(p cqparse.PendingAction, eventID *string) cqparse.ExecOutcome {
	params := p.Params
	cqGroupID := params["group_id"]
	cqUserID := params["user_id"]
	memberType := params["type"]

	// 将虚拟 user_id 反向转换为 OpenID（用于日志）
	openID, err := idmap.RetrieveRowByIDv2(cqUserID)
	if err != nil || openID == "" {
		mylog.Printf("[CQ:member] user_id=%s 转换为 OpenID 失败: %v", cqUserID, err)
	} else {
		mylog.Printf("[CQ:member] user_id=%s → OpenID=%s", cqUserID, openID)
	}

	// 缺省 group_id 回退当前会话群（Q1：空参数省略后按 key 缺失回退）
	if cqGroupID == "" {
		cqGroupID = p.DefaultGroupID
	}

	// 将虚拟 group_id 转为真实 OpenID（作为目标群）
	realGroupOpenID, err := idmap.RetrieveRowByIDv2(cqGroupID)
	if err != nil || realGroupOpenID == "" {
		mylog.Printf("[CQ:member] groupID=%s 转换为 OpenID 失败: %v", cqGroupID, err)
		realGroupOpenID = cqGroupID
	} else {
		mylog.Printf("[CQ:member] groupID=%s → OpenID=%s", cqGroupID, realGroupOpenID)
	}

	switch memberType {
	case "add":
		appID := config.GetAppIDStr()
		key := appID + "_" + realGroupOpenID
		storedEventID := echo.GetEventIDByKey(key)
		if storedEventID != "" {
			if eventID != nil {
				*eventID = storedEventID
			}
			mylog.Printf("[CQ:member] 入群回复: 使用 event_id=%s (group->%s, user->%s)", storedEventID, realGroupOpenID, openID)
		} else {
			mylog.Printf("[CQ:member] 入群回复: 未找到 event_id (group=%s)", cqGroupID)
		}
	case "remove":
		if eventID != nil {
			*eventID = ""
		}
		mylog.Printf("[CQ:member] 退群消息: 转为主动推送 (group_id=%s, user->%s)", cqGroupID, openID)
	}

	out := cqparse.ExecOutcome{RealGroupID: realGroupOpenID}
	if eventID != nil {
		out.EventID = *eventID
	}
	return out
}

// cqRemoveExec 处理 remove 撤回（迁移自 cqRemoveAction）。
// 修 M3：撤回成败只影响日志，码不回填正文。
func cqRemoveExec(p cqparse.PendingAction, apiv2 openapi.OpenAPI) cqparse.ExecOutcome {
	groupID := cqResolveGroupID(p.DefaultGroupID)
	userID := p.Params["user_id"]
	msgID := p.Params["msg_id"]

	if userID == "" || msgID == "" {
		if userID != "" && msgID == "" {
			// 缺 msg_id 时自动查该用户最新消息
			realUserID, err := idmap.RetrieveRowByIDv2(userID)
			if err != nil {
				mylog.Printf("[CQ:remove] 解析 user_id=%s 失败: %v", userID, err)
				return cqparse.ExecOutcome{}
			}
			latestRealMsgID, err := idmap.GetLatestMsgID(groupID, realUserID)
			if err != nil {
				mylog.Printf("[CQ:remove] 获取用户 %s 最新消息失败: %v", userID, err)
				return cqparse.ExecOutcome{}
			}
			mylog.Printf("[CQ:remove] 自动获取用户 %s 最新消息: %s", userID, latestRealMsgID)
			if err := apiv2.RetractGroupMessage(context.TODO(), groupID, latestRealMsgID); err != nil {
				mylog.Printf("[CQ:remove] 撤回消息失败 group=%s msg=%s: %v", groupID, latestRealMsgID, err)
			} else {
				mylog.Printf("[CQ:remove] 已撤回消息 group=%s msg=%s", groupID, latestRealMsgID)
			}
			return cqparse.ExecOutcome{}
		}
		mylog.Printf("[CQ:remove] user_id 或 msg_id 为空: %s", p.Raw)
		return cqparse.ExecOutcome{}
	}

	// 解析虚拟 user_id 为真实 OpenID（仅用于校验）
	if _, err := idmap.RetrieveRowByIDv2(userID); err != nil {
		mylog.Printf("[CQ:remove] 解析 user_id=%s 失败: %v", userID, err)
		return cqparse.ExecOutcome{}
	}

	// 解析虚拟 msg_id 为真实 message_id
	realMsgID, err := idmap.RetrieveRowByCachev2(msgID)
	if err != nil {
		mylog.Printf("[CQ:remove] 解析 msg_id=%s 失败: %v", msgID, err)
		return cqparse.ExecOutcome{}
	}
	// RetrieveRowByCachev2 返回格式 "groupID msgID"，取后半段
	parts := strings.Split(realMsgID, " ")
	realMsgID = parts[len(parts)-1]

	if err := apiv2.RetractGroupMessage(context.TODO(), groupID, realMsgID); err != nil {
		mylog.Printf("[CQ:remove] 撤回消息失败 group=%s msg=%s: %v", groupID, realMsgID, err)
	} else {
		mylog.Printf("[CQ:remove] 已撤回消息 group=%s msg=%s", groupID, realMsgID)
	}
	return cqparse.ExecOutcome{}
}

// cqSetGroupExec 统一分发 set_group 系列动作（迁移自 cqSetGroupAction）。
// 未知 action 在解析阶段已保留原文，不会到达这里。
func cqSetGroupExec(p cqparse.PendingAction, apiv2 openapi.OpenAPI) cqparse.ExecOutcome {
	params := p.Params
	switch params["action"] {
	case "ban":
		return cqSetGroupBanExec(params, p.DefaultGroupID, apiv2)
	case "whole_ban":
		return cqSetGroupWholeBanExec(params, p.DefaultGroupID, apiv2)
	case "add_request":
		return cqSetGroupAddRequestExec(params, p.DefaultGroupID, apiv2)
	case "strategy_execute":
		return cqSetGroupStrategyExec(params, apiv2, true)
	case "strategy_delete":
		return cqSetGroupStrategyExec(params, apiv2, false)
	case "kick":
		return cqSetGroupKickExec(params, p.DefaultGroupID, p.Raw, apiv2)
	case "blacklist_add", "blacklist_del":
		return cqSetGroupBlacklistExec(params, p.DefaultGroupID, p.Raw, apiv2, params["action"])
	default:
		mylog.Printf("[CQ:set_group] 未知 action=%s: %s", params["action"], p.Raw)
		return cqparse.ExecOutcome{}
	}
}

// cqSetGroupBanExec 成员禁言（duration=0 解除）。缺参数日志 + 跳过（码已移除，M4 语义）。
func cqSetGroupBanExec(params map[string]string, defaultGroupID string, apiv2 openapi.OpenAPI) cqparse.ExecOutcome {
	userID := params["user_id"]
	groupID := params["group_id"]
	if groupID == "" {
		groupID = defaultGroupID
	}
	if groupID == "" || userID == "" {
		mylog.Printf("[CQ:set_group] ban: group_id 或 user_id 为空")
		return cqparse.ExecOutcome{}
	}
	duration, err := strconv.Atoi(params["duration"])
	if err != nil {
		duration = 0
	}
	groupOpenID, err := resolveGroupOpenID(groupID)
	if err != nil {
		mylog.Printf("[CQ:set_group] ban: 群 OpenID 反查失败: %v", err)
		return cqparse.ExecOutcome{}
	}
	memberOpenID, err := resolveMemberOpenID(userID)
	if err != nil {
		mylog.Printf("[CQ:set_group] ban: user_id=%s 反查失败: %v", userID, err)
		return cqparse.ExecOutcome{}
	}
	if err := applyRestrictChatSetting(apiv2, groupOpenID, memberOpenID, duration, nil); err != nil {
		mylog.Printf("[CQ:set_group] ban: 设置禁言失败: %v", err)
	} else {
		mylog.Printf("[CQ:set_group] ban: 已设置禁言 group=%s user=%s duration=%d", groupOpenID, memberOpenID, duration)
	}
	return cqparse.ExecOutcome{}
}

// cqSetGroupWholeBanExec 全员禁言开关。修 M4：enable 非法 → 日志 + 跳过，不再泄漏。
func cqSetGroupWholeBanExec(params map[string]string, defaultGroupID string, apiv2 openapi.OpenAPI) cqparse.ExecOutcome {
	groupID := params["group_id"]
	if groupID == "" {
		groupID = defaultGroupID
	}
	if groupID == "" {
		mylog.Printf("[CQ:set_group] whole_ban: group_id 为空")
		return cqparse.ExecOutcome{}
	}
	enable, err := strconv.ParseBool(params["enable"])
	if err != nil {
		mylog.Printf("[CQ:set_group] whole_ban: enable 参数无效: %s", params["enable"])
		return cqparse.ExecOutcome{}
	}
	groupOpenID, err := resolveGroupOpenID(groupID)
	if err != nil {
		mylog.Printf("[CQ:set_group] whole_ban: 群 OpenID 反查失败: %v", err)
		return cqparse.ExecOutcome{}
	}
	allMute := enable
	if err := applyRestrictChatSetting(apiv2, groupOpenID, "", 0, &allMute); err != nil {
		mylog.Printf("[CQ:set_group] whole_ban: 设置全员禁言失败: %v", err)
	} else {
		mylog.Printf("[CQ:set_group] whole_ban: 已设置全员禁言 group=%s enable=%v", groupOpenID, enable)
	}
	return cqparse.ExecOutcome{}
}

// cqSetGroupAddRequestExec 审批入群申请（可带 reason / add_to_member_blacklist）。
func cqSetGroupAddRequestExec(params map[string]string, defaultGroupID string, apiv2 openapi.OpenAPI) cqparse.ExecOutcome {
	userID := params["user_id"]
	flag := params["flag"]
	groupID := params["group_id"]
	if groupID == "" {
		groupID = defaultGroupID
	}
	if groupID == "" || userID == "" || flag == "" {
		mylog.Printf("[CQ:set_group] add_request: group_id/user_id/flag 不能为空")
		return cqparse.ExecOutcome{}
	}
	approve := true // 省略时默认通过
	if params["approve"] != "" {
		parsed, err := strconv.ParseBool(params["approve"])
		if err != nil {
			mylog.Printf("[CQ:set_group] add_request: approve 参数无效: %s", params["approve"])
			return cqparse.ExecOutcome{}
		}
		approve = parsed
	}
	groupOpenID, err := resolveGroupOpenID(groupID)
	if err != nil {
		mylog.Printf("[CQ:set_group] add_request: 群 OpenID 反查失败: %v", err)
		return cqparse.ExecOutcome{}
	}
	memberOpenID, err := resolveMemberOpenID(userID)
	if err != nil {
		mylog.Printf("[CQ:set_group] add_request: user_id=%s 反查失败: %v", userID, err)
		return cqparse.ExecOutcome{}
	}
	var blacklist *bool
	if b, err := strconv.ParseBool(params["add_to_member_blacklist"]); err == nil {
		blacklist = &b
	}
	if err := approveJoinRequest(apiv2, groupOpenID, memberOpenID, flag, approve, params["reason"], blacklist); err != nil {
		mylog.Printf("[CQ:set_group] add_request: 审批失败: %v", err)
	} else {
		op := "decline"
		if approve {
			op = "approve"
		}
		mylog.Printf("[CQ:set_group] add_request: 已审批 group=%s user=%s op=%s", groupOpenID, memberOpenID, op)
	}
	return cqparse.ExecOutcome{}
}

// cqSetGroupStrategyExec 执行/删除入群自动审批策略。
func cqSetGroupStrategyExec(params map[string]string, apiv2 openapi.OpenAPI, execute bool) cqparse.ExecOutcome {
	strategyID := params["strategy_id"]
	if strategyID == "" {
		mylog.Printf("[CQ:set_group] strategy: strategy_id 为空")
		return cqparse.ExecOutcome{}
	}
	if execute {
		if err := apiv2.ExecuteJoinApprovalStrategy(context.TODO(), strategyID); err != nil {
			mylog.Printf("[CQ:set_group] strategy: 执行策略失败: %v", err)
		} else {
			mylog.Printf("[CQ:set_group] strategy: 已执行策略 %s（异步约10分钟）", strategyID)
		}
	} else {
		if err := apiv2.DeleteJoinApprovalStrategy(context.TODO(), strategyID); err != nil {
			mylog.Printf("[CQ:set_group] strategy: 删除策略失败: %v", err)
		} else {
			mylog.Printf("[CQ:set_group] strategy: 已删除策略 %s", strategyID)
		}
	}
	return cqparse.ExecOutcome{}
}

// cqSetGroupKickExec 单个/批量移出群成员（≤20，单个也走官方批量接口）。
func cqSetGroupKickExec(params map[string]string, defaultGroupID, raw string, apiv2 openapi.OpenAPI) cqparse.ExecOutcome {
	groupID := params["group_id"]
	if groupID == "" {
		groupID = defaultGroupID
	}
	ids := cqSetGroupUserIDs(params, raw)
	if groupID == "" || len(ids) == 0 {
		mylog.Printf("[CQ:set_group] kick: group_id 或 user_id/user_ids 为空")
		return cqparse.ExecOutcome{}
	}
	groupOpenID, err := resolveGroupOpenID(groupID)
	if err != nil {
		mylog.Printf("[CQ:set_group] kick: 群 OpenID 反查失败: %v", err)
		return cqparse.ExecOutcome{}
	}
	openIDs := cqSetGroupResolveMembers(ids, "kick")
	if len(openIDs) == 0 {
		return cqparse.ExecOutcome{} // 全部反查失败,无成员可操作
	}
	addBlacklist := false
	if params["add_blacklist"] != "" {
		b, err := strconv.ParseBool(params["add_blacklist"])
		if err != nil {
			mylog.Printf("[CQ:set_group] kick: add_blacklist 参数无效,按 false 处理: %s", params["add_blacklist"])
		} else {
			addBlacklist = b
		}
	}
	req := &dto.BatchRemoveMembersRequest{
		MemberOpenIDs:        openIDs,
		AddToMemberBlacklist: addBlacklist,
	}
	resp, err := apiv2.BatchRemoveMembers(context.TODO(), groupOpenID, req)
	if err != nil {
		mylog.Printf("[CQ:set_group] kick: 批量移出失败: %v", err)
	} else {
		mylog.Printf("[CQ:set_group] kick: 已提交移出 group=%s 共 %d 人(add_blacklist=%v, result=%s)", groupOpenID, len(openIDs), addBlacklist, resp.RemoveMembersResult)
	}
	return cqparse.ExecOutcome{}
}

// cqSetGroupBlacklistExec 群黑名单增删（≤20）。
func cqSetGroupBlacklistExec(params map[string]string, defaultGroupID, raw string, apiv2 openapi.OpenAPI, action string) cqparse.ExecOutcome {
	op := "add"
	if action == "blacklist_del" {
		op = "del"
	}
	groupID := params["group_id"]
	if groupID == "" {
		groupID = defaultGroupID
	}
	ids := cqSetGroupUserIDs(params, raw)
	if groupID == "" || len(ids) == 0 {
		mylog.Printf("[CQ:set_group] %s: group_id 或 user_id/user_ids 为空", action)
		return cqparse.ExecOutcome{}
	}
	groupOpenID, err := resolveGroupOpenID(groupID)
	if err != nil {
		mylog.Printf("[CQ:set_group] %s: 群 OpenID 反查失败: %v", action, err)
		return cqparse.ExecOutcome{}
	}
	openIDs := cqSetGroupResolveMembers(ids, action)
	if len(openIDs) == 0 {
		return cqparse.ExecOutcome{} // 全部反查失败,无成员可操作
	}
	req := &dto.MemberBlacklistRequest{
		Op:            op,
		MemberOpenIDs: openIDs,
	}
	resp, err := apiv2.UpdateMemberBlacklist(context.TODO(), groupOpenID, req)
	if err != nil {
		mylog.Printf("[CQ:set_group] %s: 黑名单操作失败: %v", action, err)
	} else if len(resp.FailOpenids) > 0 {
		mylog.Printf("[CQ:set_group] %s: 部分失败 %d 人: %v", action, len(resp.FailOpenids), resp.FailOpenids)
	} else {
		mylog.Printf("[CQ:set_group] %s: 已提交黑名单 op=%s group=%s 共 %d 人", action, op, groupOpenID, len(openIDs))
	}
	return cqparse.ExecOutcome{}
}

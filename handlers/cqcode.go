package handlers

import (
	"context"
	"encoding/base64"
	"regexp"
	"runtime"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/echo"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

// ---------- 包级正则表达式（避免重复编译） ----------

var (
	httpUrlImagePattern  = regexp.MustCompile(`\[CQ:image,file=http://(.+?)\]`)
	httpsUrlImagePattern = regexp.MustCompile(`\[CQ:image,file=https://(.+?)\]`)
	base64ImagePattern   = regexp.MustCompile(`\[CQ:image,file=base64://(.+?)\]`)
	base64RecordPattern  = regexp.MustCompile(`\[CQ:record,file=base64://(.+?)\]`)
	httpUrlRecordPattern = regexp.MustCompile(`\[CQ:record,file=http://(.+?)\]`)
	httpsUrlRecordPattern = regexp.MustCompile(`\[CQ:record,file=https://(.+?)\]`)
	httpUrlVideoPattern  = regexp.MustCompile(`\[CQ:video,file=http://(.+?)\]`)
	httpsUrlVideoPattern = regexp.MustCompile(`\[CQ:video,file=https://(.+?)\]`)
	base64VideoPattern   = regexp.MustCompile(`\[CQ:video,file=base64://(.+?)\]`)
	mdPattern            = regexp.MustCompile(`\[CQ:markdown,data=base64://(.+?)\]`)
	mdJSONPattern        = regexp.MustCompile(`\[CQ:markdown,data=(\{.*\})\]`)
	qqMusicPattern       = regexp.MustCompile(`\[CQ:music,type=qq,id=(\d+)\]`)
	cardPattern          = regexp.MustCompile(`\[CQ:card[^\]]*\]`)
	inputNotifyPattern   = regexp.MustCompile(`\[CQ:input_notify,type=(\d+)(?:,second=(\d+))?\]`)
	streamPattern        = regexp.MustCompile(`\[CQ:stream,type:(\w+),qq:(\d+)\]`)
	keyboardPattern      = regexp.MustCompile(`\[CQ:keyboard,data=base64://(.+?)\]`)
	keyboardJSONPattern  = regexp.MustCompile(`\[CQ:keyboard,data=(\{.*\})\]`)
	replyRe              = regexp.MustCompile(`\[CQ:reply,id=(\d+)\]`)
	localImagePattern    *regexp.Regexp
	localRecordPattern   *regexp.Regexp
	localVideoPattern    *regexp.Regexp
	compilePatternsOnce  sync.Once
)

// initPlatformPatterns 初始化平台相关的正则表达式（Windows vs Unix 路径前缀差异）
func initPlatformPatterns() {
	if runtime.GOOS == "windows" {
		localImagePattern = regexp.MustCompile(`\[CQ:image,file=file:///([^\]]+?)\]`)
		localRecordPattern = regexp.MustCompile(`\[CQ:record,file=file:///([^\]]+?)\]`)
		localVideoPattern = regexp.MustCompile(`\[CQ:video,file=file:///([^\]]+?)\]`)
	} else {
		localImagePattern = regexp.MustCompile(`\[CQ:image,file=file://([^\]]+?)\]`)
		localRecordPattern = regexp.MustCompile(`\[CQ:record,file=file://([^\]]+?)\]`)
		localVideoPattern = regexp.MustCompile(`\[CQ:video,file=file://([^\]]+?)\]`)
	}
}

// ProcessCQFile 解析 [CQ:file,file=xxx,file_name=yyy] 并移除
// file_name 为可选参数，不填时由后续处理自动从路径/URL 提取文件名
func ProcessCQFile(text string, foundItems map[string][]string) string {
	re := regexp.MustCompile(`\[CQ:file,([^\]]*)\]`)
	text = re.ReplaceAllStringFunc(text, func(match string) string {
		inner := match[len("[CQ:file,") : len(match)-1]
		var filePath, fileName string
		for _, part := range strings.Split(inner, ",") {
			kv := strings.SplitN(part, "=", 2)
			if len(kv) == 2 {
				switch strings.TrimSpace(kv[0]) {
				case "file":
					filePath = strings.TrimSpace(kv[1])
				case "file_name":
					fileName = strings.TrimSpace(kv[1])
				}
			}
		}
		if filePath == "" {
			return match
		}
		// 根据 file 前缀确定类型
		var itemKey, cleanValue string
		switch {
		case strings.HasPrefix(filePath, "file://"):
			itemKey = "local_file"
			safePath, err := resolveLocalMedia(filePath)
			if err != nil {
				mylog.Printf("安全校验失败，跳过本地文件: %v", err)
				return match
			}
			cleanValue = safePath
		case strings.HasPrefix(filePath, "http://"):
			itemKey = "url_file"
			cleanValue = strings.TrimPrefix(filePath, "http://")
		case strings.HasPrefix(filePath, "https://"):
			itemKey = "url_files"
			cleanValue = strings.TrimPrefix(filePath, "https://")
		case strings.HasPrefix(filePath, "base64://"):
			itemKey = "base64_file"
			cleanValue = strings.TrimPrefix(filePath, "base64://")
		default:
			return match
		}
		foundItems[itemKey] = append(foundItems[itemKey], cleanValue)
		if fileName != "" {
			foundItems["file_name"] = append(foundItems["file_name"], fileName)
		}
		return ""
	})
	return text
}

// ProcessCQKeyboard 解析 [CQ:keyboard,data=base64://...] 或 [CQ:keyboard,data={json}] 并移除
// 与 [CQ:markdown] 保持一致：data 支持 base64 编码或原始 JSON 两种形式，
// 解码后的键盘 JSON（结构同 parseMDData 的 keyboard 字段）存入 foundItems["keyboard"]
func ProcessCQKeyboard(text string, foundItems map[string][]string) string {
	// 处理 base64 格式
	text = keyboardPattern.ReplaceAllStringFunc(text, func(match string) string {
		if submatch := keyboardPattern.FindStringSubmatch(match); len(submatch) > 1 {
			decoded, err := base64.StdEncoding.DecodeString(submatch[1])
			if err != nil {
				mylog.Printf("[CQ:keyboard] base64 解码失败: %v", err)
				return ""
			}
			foundItems["keyboard"] = append(foundItems["keyboard"], string(decoded))
		}
		return ""
	})
	// 处理原始 JSON 格式
	text = keyboardJSONPattern.ReplaceAllStringFunc(text, func(match string) string {
		if submatch := keyboardJSONPattern.FindStringSubmatch(match); len(submatch) > 1 {
			foundItems["keyboard"] = append(foundItems["keyboard"], submatch[1])
		}
		return ""
	})
	return text
}

// ProcessCQActive 解析 [CQ:active] 或 [CQ:active,type=xxx,sub_type=yyy] 并移除
func ProcessCQActive(text string, foundItems map[string][]string) string {
	// 先匹配带参数的 [CQ:active,type=xxx,sub_type=yyy]
	re := regexp.MustCompile(`\[CQ:active,([^\]]*)\]`)
	text = re.ReplaceAllStringFunc(text, func(match string) string {
		inner := match[1 : len(match)-1]
		if idx := strings.Index(inner, ","); idx >= 0 {
			paramsStr := inner[idx+1:]
			for _, part := range strings.Split(paramsStr, ",") {
				kv := strings.SplitN(part, "=", 2)
				if len(kv) == 2 {
					switch strings.TrimSpace(kv[0]) {
					case "type":
						foundItems["active_type"] = append(foundItems["active_type"], strings.TrimSpace(kv[1]))
					case "sub_type":
						foundItems["active_sub_type"] = append(foundItems["active_sub_type"], strings.TrimSpace(kv[1]))
					}
				}
			}
		}
		foundItems["active"] = []string{"true"}
		return ""
	})
	// 再匹配裸 [CQ:active]（无参数）
	bareRe := regexp.MustCompile(`\[CQ:active\]`)
	if bareRe.MatchString(text) {
		foundItems["active"] = []string{"true"}
	}
	text = bareRe.ReplaceAllString(text, "")
	return text
}

// ---------- 出站动作型 CQ 码（单次扫描 + 类型分发） ----------

// ProcessOutboundCQCodes 统一处理出站动作型 CQ 码。
// 单次正则扫描全文，按类型分发执行动作并从文本移除；
// 未知类型（at/image 等标准 CQ 码）原样保留。
// 返回 (清理后的文本, member 跨群路由的 realGroupID)。
func ProcessOutboundCQCodes(text, defaultGroupID string, eventID *string, apiv2 openapi.OpenAPI) (string, string) {
	re := regexp.MustCompile(`\[CQ:([a-z_]+),([^\]]*)\]`)
	var realGroupID string
	result := re.ReplaceAllStringFunc(text, func(match string) string {
		inner := match[len("[CQ:") : len(match)-1]
		idx := strings.Index(inner, ",")
		if idx < 0 {
			return match // 无参数形式（如 [CQ:at]），防御性保留
		}
		cqType := inner[:idx]
		paramsStr := inner[idx+1:]
		switch cqType {
		case "member":
			return cqMemberAction(paramsStr, match, eventID, defaultGroupID, apiv2, &realGroupID)
		case "remove":
			return cqRemoveAction(paramsStr, match, defaultGroupID, apiv2)
		case "set_group_ban":
			return cqSetGroupBanAction(paramsStr, match, defaultGroupID, apiv2)
		case "set_group_whole_ban":
			return cqSetGroupWholeBanAction(paramsStr, match, defaultGroupID, apiv2)
		case "set_group_add_request":
			return cqSetGroupAddRequestAction(paramsStr, match, defaultGroupID, apiv2)
		case "strategy":
			return cqStrategyAction(paramsStr, match, apiv2)
		default:
			return match // 非动作 CQ 码原样保留
		}
	})
	return result, realGroupID
}

// cqResolveGroupID 将默认群 ID 反查为真实 OpenID（32 位原生 OpenID 直接使用）
func cqResolveGroupID(groupID string) string {
	if len(groupID) != 32 {
		if realGroupID, err := idmap.RetrieveRowByIDv2(groupID); err == nil && realGroupID != "" {
			return realGroupID
		}
	}
	return groupID
}

// cqMemberAction 处理 [CQ:member,type=add/remove,group_id=虚拟群ID,user_id=虚拟用户ID]
// type=add: 使用存储的 event_id 进行被动回复；type=remove: 转为主动消息发送
func cqMemberAction(paramsStr, match string, eventID *string, defaultGroupID string, apiv2 openapi.OpenAPI, realGroupID *string) string {
	var cqGroupID, cqUserID, memberType string
	for _, part := range strings.Split(paramsStr, ",") {
		kv := strings.SplitN(part, "=", 2)
		if len(kv) != 2 {
			continue
		}
		switch strings.TrimSpace(kv[0]) {
		case "type":
			memberType = strings.TrimSpace(kv[1])
		case "group_id":
			cqGroupID = strings.TrimSpace(kv[1])
		case "user_id":
			cqUserID = strings.TrimSpace(kv[1])
		}
	}

	// 将虚拟 user_id 反向转换为 OpenID（用于日志）
	openID, err := idmap.RetrieveRowByIDv2(cqUserID)
	if err != nil || openID == "" {
		mylog.Printf("[CQ:member] user_id=%s 转换为 OpenID 失败: %v", cqUserID, err)
	} else {
		mylog.Printf("[CQ:member] user_id=%s → OpenID=%s", cqUserID, openID)
	}

	// 从 CQ 码中取 group_id，优先于入参
	if cqGroupID == "" {
		cqGroupID = defaultGroupID
	}

	// 将 CQ 码中的虚拟 group_id 转为真实 OpenID（作为目标群）
	realGroupOpenID, err := idmap.RetrieveRowByIDv2(cqGroupID)
	if err != nil || realGroupOpenID == "" {
		mylog.Printf("[CQ:member] groupID=%s 转换为 OpenID 失败: %v", cqGroupID, err)
		realGroupOpenID = cqGroupID
	} else {
		mylog.Printf("[CQ:member] groupID=%s → OpenID=%s", cqGroupID, realGroupOpenID)
	}
	*realGroupID = realGroupOpenID

	switch memberType {
	case "add":
		appID := config.GetAppIDStr()
		key := appID + "_" + realGroupOpenID
		storedEventID := echo.GetEventIDByKey(key)
		if storedEventID != "" {
			*eventID = storedEventID
			mylog.Printf("[CQ:member] 入群回复: 使用 event_id=%s (group->%s, user->%s)", storedEventID, realGroupOpenID, openID)
		} else {
			mylog.Printf("[CQ:member] 入群回复: 未找到 event_id (group=%s)", cqGroupID)
		}
	case "remove":
		*eventID = ""
		mylog.Printf("[CQ:member] 退群消息: 转为主动推送 (group_id=%s, user->%s)", cqGroupID, openID)
	}

	return ""
}

// cqRemoveAction 处理 [CQ:remove,user_id=虚拟ID,msg_id=虚拟msg_id]
// 通过 QQ API 撤回指定消息，并从 messageText 中移除 CQ 码
func cqRemoveAction(paramsStr, match, defaultGroupID string, apiv2 openapi.OpenAPI) string {
	groupID := cqResolveGroupID(defaultGroupID)
	var userID, msgID string
	for _, part := range strings.Split(paramsStr, ",") {
		kv := strings.SplitN(part, "=", 2)
		if len(kv) == 2 {
			switch strings.TrimSpace(kv[0]) {
			case "user_id":
				userID = strings.TrimSpace(kv[1])
			case "msg_id":
				msgID = strings.TrimSpace(kv[1])
			}
		}
	}
	if userID == "" || msgID == "" {
		if userID != "" && msgID == "" {
			// 缺 msg_id 时自动查该用户最新消息
			realUserID, err := idmap.RetrieveRowByIDv2(userID)
			if err != nil {
				mylog.Printf("[CQ:remove] 解析 user_id=%s 失败: %v", userID, err)
				return match
			}
			latestRealMsgID, err := idmap.GetLatestMsgID(groupID, realUserID)
			if err != nil {
				mylog.Printf("[CQ:remove] 获取用户 %s 最新消息失败: %v", userID, err)
				return match
			}
			mylog.Printf("[CQ:remove] 自动获取用户 %s 最新消息: %s", userID, latestRealMsgID)
			if err := apiv2.RetractGroupMessage(context.TODO(), groupID, latestRealMsgID); err != nil {
				mylog.Printf("[CQ:remove] 撤回消息失败 group=%s msg=%s: %v", groupID, latestRealMsgID, err)
			} else {
				mylog.Printf("[CQ:remove] 已撤回消息 group=%s msg=%s", groupID, latestRealMsgID)
			}
			return ""
		}
		mylog.Printf("[CQ:remove] user_id 或 msg_id 为空: %s", match)
		return match
	}

	// 解析虚拟 user_id 为真实 OpenID（仅用于校验）
	_, err := idmap.RetrieveRowByIDv2(userID)
	if err != nil {
		mylog.Printf("[CQ:remove] 解析 user_id=%s 失败: %v", userID, err)
		return ""
	}

	// 解析虚拟 msg_id 为真实 message_id
	realMsgID, err := idmap.RetrieveRowByCachev2(msgID)
	if err != nil {
		mylog.Printf("[CQ:remove] 解析 msg_id=%s 失败: %v", msgID, err)
		return ""
	}
	// RetrieveRowByCachev2 返回格式 "groupID msgID"，取后半段
	parts := strings.Split(realMsgID, " ")
	realMsgID = parts[len(parts)-1]

	// 调用撤回 API
	if err := apiv2.RetractGroupMessage(context.TODO(), groupID, realMsgID); err != nil {
		mylog.Printf("[CQ:remove] 撤回消息失败 group=%s msg=%s: %v", groupID, realMsgID, err)
	} else {
		mylog.Printf("[CQ:remove] 已撤回消息 group=%s msg=%s", groupID, realMsgID)
	}

	return "" // 从 messageText 中移除 CQ 码，无论成败都不发送原文
}

// cqSetGroupBanAction 处理 [CQ:set_group_ban,group_id=虚拟群ID,user_id=虚拟用户ID,duration=秒]
// 设置成员禁言（duration=0 解除）
func cqSetGroupBanAction(paramsStr, match, defaultGroupID string, apiv2 openapi.OpenAPI) string {
	var cqGroupID, userID, durationStr string
	for _, part := range strings.Split(paramsStr, ",") {
		kv := strings.SplitN(part, "=", 2)
		if len(kv) != 2 {
			continue
		}
		switch strings.TrimSpace(kv[0]) {
		case "group_id":
			cqGroupID = strings.TrimSpace(kv[1])
		case "user_id":
			userID = strings.TrimSpace(kv[1])
		case "duration":
			durationStr = strings.TrimSpace(kv[1])
		}
	}
	// 群 ID 缺失时回退发送目标群
	groupID := cqResolveGroupID(cqGroupID)
	if groupID == "" {
		groupID = cqResolveGroupID(defaultGroupID)
	}
	if groupID == "" || userID == "" {
		mylog.Printf("[CQ:set_group_ban] group_id 或 user_id 为空: %s", match)
		return match
	}
	duration, err := strconv.Atoi(durationStr)
	if err != nil {
		duration = 0
	}

	// 反查真实 OpenID（32 位原生 OpenID 直接使用）
	groupOpenID := cqResolveGroupID(groupID)
	memberOpenID := userID
	if len(userID) != 32 {
		realUserID, err := idmap.RetrieveRowByIDv2(userID)
		if err != nil || realUserID == "" {
			mylog.Printf("[CQ:set_group_ban] user_id=%s 反查失败: %v", userID, err)
			return ""
		}
		memberOpenID = realUserID
	}

	setting := &dto.RestrictChatSetting{GroupOpenID: groupOpenID}
	if duration > 0 {
		setting.MemberRestrict = []dto.MemberRestrict{{
			MemberOpenID:  memberOpenID,
			RestrictUntil: time.Now().Unix() + int64(duration),
		}}
	} else {
		// 解除禁言：查询当前设置, 移除该成员后提交
		cur, err := apiv2.RestrictChatSetting(context.TODO(), groupOpenID)
		if err != nil {
			mylog.Printf("[CQ:set_group_ban] 查询禁言状态失败: %v", err)
			return ""
		}
		for _, m := range cur.MemberRestrict {
			if m.MemberOpenID != memberOpenID {
				setting.MemberRestrict = append(setting.MemberRestrict, m)
			}
		}
	}
	if err := apiv2.SetRestrictChatSetting(context.TODO(), groupOpenID, setting); err != nil {
		mylog.Printf("[CQ:set_group_ban] 设置禁言失败: %v", err)
	} else {
		mylog.Printf("[CQ:set_group_ban] 已设置禁言 group=%s user=%s duration=%d", groupOpenID, memberOpenID, duration)
	}
	return "" // 无论成败都不发送原文
}

// cqSetGroupWholeBanAction 处理 [CQ:set_group_whole_ban,group_id=虚拟群ID,enable=true/false]
// 切换全员禁言
func cqSetGroupWholeBanAction(paramsStr, match, defaultGroupID string, apiv2 openapi.OpenAPI) string {
	var cqGroupID, enableStr string
	for _, part := range strings.Split(paramsStr, ",") {
		kv := strings.SplitN(part, "=", 2)
		if len(kv) != 2 {
			continue
		}
		switch strings.TrimSpace(kv[0]) {
		case "group_id":
			cqGroupID = strings.TrimSpace(kv[1])
		case "enable":
			enableStr = strings.TrimSpace(kv[1])
		}
	}
	// 群 ID 缺失时回退发送目标群
	groupID := cqResolveGroupID(cqGroupID)
	if groupID == "" {
		groupID = cqResolveGroupID(defaultGroupID)
	}
	if groupID == "" {
		mylog.Printf("[CQ:set_group_whole_ban] group_id 为空: %s", match)
		return match
	}
	enable, err := strconv.ParseBool(enableStr)
	if err != nil {
		mylog.Printf("[CQ:set_group_whole_ban] enable 参数无效: %s", match)
		return match
	}

	groupOpenID := cqResolveGroupID(groupID)
	setting := &dto.RestrictChatSetting{GroupOpenID: groupOpenID, AllMute: enable}
	if cur, err := apiv2.RestrictChatSetting(context.TODO(), groupOpenID); err == nil {
		setting.MemberRestrict = cur.MemberRestrict
	} else {
		mylog.Printf("[CQ:set_group_whole_ban] 查询禁言状态失败: %v", err)
	}
	if err := apiv2.SetRestrictChatSetting(context.TODO(), groupOpenID, setting); err != nil {
		mylog.Printf("[CQ:set_group_whole_ban] 设置全员禁言失败: %v", err)
	} else {
		mylog.Printf("[CQ:set_group_whole_ban] 已设置全员禁言 group=%s enable=%v", groupOpenID, enable)
	}
	return ""
}

// cqSetGroupAddRequestAction 处理 [CQ:set_group_add_request,group_id=虚拟群ID,user_id=虚拟用户ID,flag=申请ID,approve=true/false]
// 审批入群申请（可带 reason / add_to_member_blacklist）
func cqSetGroupAddRequestAction(paramsStr, match, defaultGroupID string, apiv2 openapi.OpenAPI) string {
	var cqGroupID, userID, flag, approveStr, reason, blacklistStr string
	for _, part := range strings.Split(paramsStr, ",") {
		kv := strings.SplitN(part, "=", 2)
		if len(kv) != 2 {
			continue
		}
		switch strings.TrimSpace(kv[0]) {
		case "group_id":
			cqGroupID = strings.TrimSpace(kv[1])
		case "user_id":
			userID = strings.TrimSpace(kv[1])
		case "flag":
			flag = strings.TrimSpace(kv[1])
		case "approve":
			approveStr = strings.TrimSpace(kv[1])
		case "reason":
			reason = strings.TrimSpace(kv[1])
		case "add_to_member_blacklist":
			blacklistStr = strings.TrimSpace(kv[1])
		}
	}
	// 群 ID 缺失时回退发送目标群
	groupID := cqResolveGroupID(cqGroupID)
	if groupID == "" {
		groupID = cqResolveGroupID(defaultGroupID)
	}
	if groupID == "" || userID == "" || flag == "" {
		mylog.Printf("[CQ:set_group_add_request] group_id/user_id/flag 不能为空: %s", match)
		return match
	}
	approve, err := strconv.ParseBool(approveStr)
	if err != nil {
		mylog.Printf("[CQ:set_group_add_request] approve 参数无效: %s", match)
		return match
	}

	groupOpenID := cqResolveGroupID(groupID)
	memberOpenID := userID
	if len(userID) != 32 {
		realUserID, err := idmap.RetrieveRowByIDv2(userID)
		if err != nil || realUserID == "" {
			mylog.Printf("[CQ:set_group_add_request] user_id=%s 反查失败: %v", userID, err)
			return ""
		}
		memberOpenID = realUserID
	}

	op := "decline"
	if approve {
		op = "approve"
	}
	req := &dto.ApprovalJoinRequest{
		Op:            op,
		JoinRequestID: flag,
		RejectReason:  reason,
	}
	if blacklistStr != "" {
		if blacklist, err := strconv.ParseBool(blacklistStr); err == nil {
			req.AddToMemberBlacklist = blacklist
		}
	}
	if err := apiv2.ApprovalJoinRequest(context.TODO(), groupOpenID, memberOpenID, req); err != nil {
		mylog.Printf("[CQ:set_group_add_request] 审批失败: %v", err)
	} else {
		mylog.Printf("[CQ:set_group_add_request] 已审批 group=%s user=%s op=%s", groupOpenID, memberOpenID, op)
	}
	return ""
}

// cqStrategyAction 处理 [CQ:strategy,action=execute/delete,strategy_id=策略ID]
// 执行或删除入群自动审批策略；未知 action 原样保留
func cqStrategyAction(paramsStr, match string, apiv2 openapi.OpenAPI) string {
	var action, strategyID string
	for _, part := range strings.Split(paramsStr, ",") {
		kv := strings.SplitN(part, "=", 2)
		if len(kv) != 2 {
			continue
		}
		switch strings.TrimSpace(kv[0]) {
		case "action":
			action = strings.TrimSpace(kv[1])
		case "strategy_id":
			strategyID = strings.TrimSpace(kv[1])
		}
	}
	if strategyID == "" {
		mylog.Printf("[CQ:strategy] strategy_id 为空: %s", match)
		return match
	}
	switch action {
	case "execute":
		if err := apiv2.ExecuteJoinApprovalStrategy(context.TODO(), strategyID); err != nil {
			mylog.Printf("[CQ:strategy] 执行策略失败: %v", err)
		} else {
			mylog.Printf("[CQ:strategy] 已执行策略 %s（异步约10分钟）", strategyID)
		}
	case "delete":
		if err := apiv2.DeleteJoinApprovalStrategy(context.TODO(), strategyID); err != nil {
			mylog.Printf("[CQ:strategy] 删除策略失败: %v", err)
		} else {
			mylog.Printf("[CQ:strategy] 已删除策略 %s", strategyID)
		}
	default:
		mylog.Printf("[CQ:strategy] 未知 action=%s: %s", action, match)
		return match // 未知 action 原样保留
	}
	return ""
}

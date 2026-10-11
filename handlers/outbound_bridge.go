package handlers

import (
	"context"
	"encoding/json"
	"strings"
	"time"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/echo"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/internal/application/outbound"
	"github.com/hoshinonyaruko/gensokyo/internal/domain/identity"
	"github.com/hoshinonyaruko/gensokyo/internal/domain/message"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/dto/keyboard"
	"github.com/tencent-connect/botgo/openapi"
)

// 本文件是 A 包（出站接缝）的桥：把 legacy 产物（messageText + foundItems）
// 反向组装为 outbound.OutboundCommand，并按 arch_mode 在 handler 内分支。
// legacy 一行不删；new 走统一出站服务；shadow 执行 new 并 diff 上报。

// outboundSvc 由 main.go bootstrap 注入；nil 时 new 路径回退 legacy。
var outboundSvc *outbound.OutboundService

// SetOutboundService 注入统一出站服务（main.go 装配调用）。
func SetOutboundService(s *outbound.OutboundService) { outboundSvc = s }

// OutboundService 返回当前注入的出站服务（测试/诊断用）。
func OutboundService() *outbound.OutboundService { return outboundSvc }

// outboundMsgCtx 携带 new 链解析 msg_id/event_id 所需的 legacy 上下文：
// lazy 池以虚拟群号/虚拟用户号为键（Processor/ProcessGroupMessage.go:293），
// echo 串用于 GetMsgIDByKey 兜底（legacy send_group_msg.go:213-218）。
// eventID 为 C1 预执行的动作码产物（[CQ:member,type=add]），注入群 event_id 解析；
// hasEventID 表示 CQ 显式决定过 eventID（[CQ:member,type=remove] 为显式清空）。
// origGroupOpenID 为原群真实 OpenID：跨群路由时目标群被 CQ 的 group_id 覆盖，
// 但 msg_id/event_id 查缓存与 SSM 补发仍用原群（legacy send_group_msg.go:283/290/299/322）。
type outboundMsgCtx struct {
	virtualGroupID  string
	virtualUserID   string
	echo            string
	eventID         string
	hasEventID      bool
	origGroupOpenID string
}

type outboundMsgCtxKey struct{}

// withOutboundMsgCtx 把 legacy 上下文随 ctx 传入出站发送链。
func withOutboundMsgCtx(ctx context.Context, c outboundMsgCtx) context.Context {
	return context.WithValue(ctx, outboundMsgCtxKey{}, c)
}

// outboundMsgCtxFrom 读取出站上下文；直接调用 SendFoundItems*（无 ctx）时返回 false。
func outboundMsgCtxFrom(ctx context.Context) (outboundMsgCtx, bool) {
	c, ok := ctx.Value(outboundMsgCtxKey{}).(outboundMsgCtx)
	return c, ok
}

// outboundMsgCtxFor 从 ActionMessage 构造 legacy 上下文（虚拟 ID 与 echo 均取自请求参数）。
func outboundMsgCtxFor(msg callapi.ActionMessage) outboundMsgCtx {
	c := outboundMsgCtx{}
	if gid, ok := msg.Params.GroupID.(string); ok && gid != "" && gid != "0" {
		c.virtualGroupID = gid
	}
	if uid, ok := msg.Params.UserID.(string); ok && uid != "" && uid != "0" {
		c.virtualUserID = uid
	}
	if echoStr, ok := msg.Echo.(string); ok {
		c.echo = echoStr
	}
	return c
}

// ArchMode 返回当前分层架构模式（legacy|shadow|new），逐请求读、热重载生效。
func ArchMode() string { return config.GetArchMode() }

// wakeupMarker 是 bridge 编码 wakeup 语义的 Parts 标记。
// outbound.QQSender.Send 只接收 (target, msg)，DeliveryPolicy 不经其传递，
// 故 wakeup 以 UnknownPart 标记随 Parts 携带，由 adapter 识别。
const wakeupMarker = "wakeup"

// BuildOutboundCommand 把 legacy 产物反向组装为 OutboundCommand。
func BuildOutboundCommand(messageText string, foundItems map[string][]string, target identity.ResolvedTarget, delivery outbound.DeliveryPolicy) outbound.OutboundCommand {
	parts := foundItemsToParts(messageText, foundItems)
	var reply *outbound.ReplyRef
	if ids := foundItems["reply_msg_id"]; len(ids) > 0 && ids[0] != "" {
		reply = &outbound.ReplyRef{MessageID: ids[0]}
	}
	if delivery.Wakeup {
		parts = append(parts, message.UnknownPart{Type: wakeupMarker})
	}
	return outbound.OutboundCommand{
		Target:  target,
		Message: outbound.OutboundMessage{Parts: parts, Reply: reply},
		Delivery: delivery,
	}
}

// ResolveOutboundTarget 从 ActionMessage 解析发送目标（虚拟 ID → 真实 OpenID）。
func ResolveOutboundTarget(msg callapi.ActionMessage, kind identity.TargetKind) (identity.ResolvedTarget, bool) {
	if kind == identity.TargetGroup {
		gid, ok := msg.Params.GroupID.(string)
		if !ok || gid == "" || gid == "0" {
			return identity.ResolvedTarget{}, false
		}
		open := resolveOpenID(gid)
		return identity.ResolvedTarget{
			Kind:  identity.TargetGroup,
			Group: &identity.ResolvedGroup{OpenID: identity.OpenGroupID(open), VirtualGroupID: identity.VirtualGroupID(gid)},
		}, true
	}
	uid, ok := msg.Params.UserID.(string)
	if !ok || uid == "" || uid == "0" {
		return identity.ResolvedTarget{}, false
	}
	open := resolveOpenID(uid)
	return identity.ResolvedTarget{
		Kind: identity.TargetPrivate,
		User: &identity.ResolvedUser{OpenID: identity.OpenID(open), VirtualUserID: identity.VirtualUserID(uid)},
	}, true
}

// resolveOpenID 已是 OpenID 则直通，否则经 idmap 反查；失败时保留原值。
func resolveOpenID(id string) string {
	if identity.IsOpenID(id) {
		return id
	}
	if real, err := idmap.RetrieveRowByIDv2(id); err == nil && real != "" {
		return real
	}
	return id
}

// TryOutbound 在 arch_mode=new/shadow 时尝试走统一出站服务。
// 返回 (handled, retmsg)：new 成功时 handled=true；shadow 的 legacy 执行返回 false（legacy 继续）。
// 回执复用 legacy 发送核心（SendResponse/SendC2CResponse）主动回发，保证 echo/group_id/虚拟 message_id
// 与 StoreCachev2/botstats/自动撤回副作用与 legacy 等价。
func TryOutbound(client callapi.Client, api, apiv2 openapi.OpenAPI, msg callapi.ActionMessage, messageText string, foundItems map[string][]string, kind identity.TargetKind, wakeup bool) (bool, string) {
	return TryOutboundWithCQ(client, api, apiv2, msg, messageText, foundItems, kind, wakeup, "", "", false)
}

// TryOutboundWithCQ 在 TryOutbound 基础上透传 C1 预执行的 CQ 产物：
// realGroupID 覆盖群发送目标（[CQ:member] 跨群路由，legacy send_group_msg.go:546-550）；
// eventID 注入群 msg_id/event_id 解析（member add 的被动回复，legacy cqcode.go:270-280）；
// eventIDSet 标记 CQ 是否显式决定过 eventID（remove 为显式清空，禁止 2000 分支回填）。
func TryOutboundWithCQ(client callapi.Client, api, apiv2 openapi.OpenAPI, msg callapi.ActionMessage, messageText string, foundItems map[string][]string, kind identity.TargetKind, wakeup bool, realGroupID, eventID string, eventIDSet bool) (bool, string) {
	mode := ArchMode()
	if mode == "legacy" || outboundSvc == nil {
		return false, ""
	}
	target, ok := ResolveOutboundTarget(msg, kind)
	if !ok {
		return false, ""
	}
	// C1：CQ 码携带的 group_id 优先作为目标群（legacy send_group_msg.go:546-550 跨群路由）
	// 原群在覆写前捕获：msg_id/event_id 查缓存与 SSM 补发仍属原群（legacy send_group_msg.go:283/290/299/322）。
	origGroupOpenID := ""
	if kind == identity.TargetGroup && target.Group != nil {
		origGroupOpenID = string(target.Group.OpenID)
	}
	if kind == identity.TargetGroup && realGroupID != "" && target.Group != nil {
		target.Group.OpenID = identity.OpenGroupID(realGroupID)
	}
	delivery := outbound.DeliveryPolicy{Wakeup: wakeup}
	if _, ok := foundItems["active"]; ok {
		delivery.Mode = message.ModeActive
	}
	if mode == "shadow" {
		// shadow：new 对比执行只 diff、不发送（避免与 legacy 双发）；legacy 执行不发送（legacy 自行发送）。
		// dry-run 为 per-request（msg.ShadowDryRun 由 dispatch_bridge 从 ctx 回填），并发请求互不串读。
		if msg.ShadowDryRun {
			cmd := BuildOutboundCommand(messageText, cloneFoundItems(foundItems), target, delivery)
			runOutboundShadow(messageText, foundItems, cmd)
			return true, ""
		}
		return false, ""
	}

	// legacy 上下文（虚拟 ID/echo/预执行 event_id/原群）随 ctx 传入发送链，供 msg_id/event_id 解析复用 legacy 语义。
	c := outboundMsgCtxFor(msg)
	c.eventID = eventID
	c.hasEventID = eventIDSet
	c.origGroupOpenID = origGroupOpenID
	sendCtx := withOutboundMsgCtx(context.Background(), c)

	// 私聊：input_notify / stream 在 legacy 中先于正文发送（stream 发送后不再走正文）。
	if kind == identity.TargetPrivate && target.User != nil {
		userOpenID := string(target.User.OpenID)
		messageID, _ := resolvePrivateMessageContext(sendCtx, userOpenID, foundItems)
		if wakeup {
			messageID = ""
		}
		if ret, present := sendStream(client, &msg, messageID, userOpenID, messageText, foundItems, apiv2); present {
			return true, ret
		}
		if ret, handled := sendInputNotify(client, &msg, messageID, userOpenID, foundItems, apiv2); handled {
			_ = ret
		}
	}

	// 群聊：lazy_message_id 下补发栈内失败消息（legacy 语义）。
	// 补发属原群的栈（legacy send_group_msg.go:283 用 message.Params.GroupID），跨群路由时不用目标群。
	if kind == identity.TargetGroup && target.Group != nil && config.GetLazyMessageId() {
		flushSSM(apiv2, msg, origGroupOpenID)
	}

	cmd := BuildOutboundCommand(messageText, foundItems, target, delivery)
	res, err := outboundSvc.Send(sendCtx, cmd)
	if err != nil {
		mylog.Printf("[outbound] 发送失败: %v", err)
	}
	return true, sendOutboundReceipt(client, api, apiv2, msg, target, res.MessageID, err)
}

// flushSSM 在 lazy_message_id 下补发栈内失败消息（对齐 legacy send_group_msg.go:267-271）。
func flushSSM(apiv2 openapi.OpenAPI, msg callapi.ActionMessage, groupOpenID string) {
	gid, ok := msg.Params.GroupID.(string)
	if !ok || gid == "" {
		return
	}
	lazyID := echo.GetLazyMessagesId(gid)
	if uid, ok := msg.Params.UserID.(string); ok && uid != "" && uid != "0" {
		lazyID = echo.GetLazyMessagesIdv2(gid, uid)
	}
	if lazyID == "2000" {
		return
	}
	SendStackMessages(apiv2, lazyID, groupOpenID)
}

// sendOutboundReceipt 复用 legacy 发送核心产出回执：含 echo/group_id(或 user_id)/虚拟 message_id，
// 并保留 StoreCachev2/botstats/自动撤回副作用（对齐 message_parser.go SendResponse/SendC2CResponse）。
func sendOutboundReceipt(client callapi.Client, api, apiv2 openapi.OpenAPI, msg callapi.ActionMessage, target identity.ResolvedTarget, realID string, sendErr error) string {
	// 群发送后记录"机器人最新消息"（delete_group_msg 依赖；legacy send_group_msg.go:492 等 5 处）。
	// 放在 NoRetMsg 早退之前，保证关闭回执时副作用仍在。
	if target.Kind == identity.TargetGroup && target.Group != nil {
		// legacy 在回执前把 GroupID 覆写为真实 OpenID（send_group_msg.go:262）
		msg.Params.GroupID = string(target.Group.OpenID)
		if realID != "" {
			rememberLatestBotGroupMessageInGroup(string(target.Group.OpenID), &dto.GroupMessageResponse{Message: &dto.Message{ID: realID}})
		}
	}
	if config.GetNoRetMsg() {
		return ""
	}
	switch target.Kind {
	case identity.TargetGroup:
		var resp *dto.GroupMessageResponse
		if realID != "" {
			resp = &dto.GroupMessageResponse{Message: &dto.Message{ID: realID}}
		}
		if config.GetThreadsRetMsg() {
			go func() {
				if config.GetStringOb11() {
					SendResponseSB(client, sendErr, &msg, resp, api, apiv2)
				} else {
					SendResponse(client, sendErr, &msg, resp, api, apiv2)
				}
			}()
			return ""
		}
		if config.GetStringOb11() {
			ret, _ := SendResponseSB(client, sendErr, &msg, resp, api, apiv2)
			return ret
		}
		ret, _ := SendResponse(client, sendErr, &msg, resp, api, apiv2)
		return ret
	case identity.TargetPrivate:
		var resp *dto.C2CMessageResponse
		if realID != "" {
			resp = &dto.C2CMessageResponse{Message: &dto.Message{ID: realID}}
		}
		ret, _ := SendC2CResponse(client, sendErr, &msg, resp)
		return ret
	}
	return buildOutboundResponse(outbound.SendResult{MessageID: realID})
}

// buildOutboundResponse 构造与 OneBot 兼容的最小回执。
func buildOutboundResponse(res outbound.SendResult) string {
	resp := map[string]interface{}{
		"status":  "ok",
		"retcode": 0,
		"data":    map[string]interface{}{"message_id": res.MessageID},
	}
	b, err := json.Marshal(resp)
	if err != nil {
		return `{"status":"ok","retcode":0}`
	}
	return string(b)
}

// runOutboundShadow 把 new 路径的产物（Parts 往返）与 legacy foundItems diff 上报。
func runOutboundShadow(messageText string, foundItems map[string][]string, cmd outbound.OutboundCommand) {
	rtText, rtFound := PartsToFoundItems(cmd.Message.Parts)
	if rtText != messageText {
		mylog.Printf("[outbound-shadow] text diff: legacy=%q new=%q", messageText, rtText)
	}
	for k, v := range foundItems {
		if k == "active" || k == "active_type" || k == "active_sub_type" || k == "wakeup" {
			continue
		}
		nv, ok := rtFound[k]
		if !ok {
			mylog.Printf("[outbound-shadow] key %q 在 new 产物中缺失 (legacy=%v)", k, v)
			continue
		}
		if strings.Join(v, "\x00") != strings.Join(nv, "\x00") {
			mylog.Printf("[outbound-shadow] key %q 值不一致: legacy=%v new=%v", k, v, nv)
		}
	}
	for k := range rtFound {
		if _, ok := foundItems[k]; !ok {
			mylog.Printf("[outbound-shadow] key %q 在 legacy 中不存在 (new=%v)", k, rtFound[k])
		}
	}
}

// ---- foundItems ↔ Parts 反向转换（ToLegacyFoundItems 只有正向） ----

// foundItemsToParts 把 legacy foundItems + messageText 转为类型化 Parts。
func foundItemsToParts(messageText string, foundItems map[string][]string) []message.MessagePart {
	var parts []message.MessagePart
	if messageText != "" {
		parts = append(parts, message.TextPart{Text: messageText})
	}
	// 媒体：单数 key（local_/url_/base64_）与复数 url_ 形式
	for _, m := range []struct {
		suffix string
		plural string
		mk     func(message.MediaSource, string) message.MessagePart
	}{
		{"image", "url_images", func(s message.MediaSource, f string) message.MessagePart { return message.ImagePart{Source: s, File: f} }},
		{"record", "url_records", func(s message.MediaSource, f string) message.MessagePart { return message.AudioPart{Source: s, File: f} }},
		{"video", "url_videos", func(s message.MediaSource, f string) message.MessagePart { return message.VideoPart{Source: s, File: f} }},
	} {
		for _, prefix := range []string{"local_", "url_", "base64_"} {
			for _, v := range foundItems[prefix+m.suffix] {
				parts = append(parts, m.mk(mediaSourceFromLegacy(prefix, v), v))
			}
		}
		for _, v := range foundItems[m.plural] {
			parts = append(parts, m.mk(message.MediaSource{Kind: message.MediaRemoteURL, URL: v}, v))
		}
	}
	// 文件（含 file_name）
	fileNames := foundItems["file_name"]
	fi := 0
	for _, prefix := range []string{"local_", "url_", "base64_"} {
		for _, v := range foundItems[prefix+"file"] {
			fp := message.FilePart{Source: mediaSourceFromLegacy(prefix, v), File: v}
			if fi < len(fileNames) {
				fp.Filename = fileNames[fi]
			}
			fi++
			parts = append(parts, fp)
		}
	}
	for _, v := range foundItems["url_files"] {
		fp := message.FilePart{Source: message.MediaSource{Kind: message.MediaRemoteURL, URL: v}, File: v}
		if fi < len(fileNames) {
			fp.Filename = fileNames[fi]
		}
		fi++
		parts = append(parts, fp)
	}
	for _, v := range foundItems["markdown"] {
		parts = append(parts, message.MarkdownPart{Content: v})
	}
	for _, v := range foundItems["keyboard"] {
		parts = append(parts, message.KeyboardPart{Content: v})
	}
	for _, v := range foundItems["qqmusic"] {
		parts = append(parts, message.QQMusicPart{ID: v})
	}
	// 其余控制/扩展 key（card/input_notify/stream/active* 等）以 UnknownPart 保留原值
	for _, k := range []string{"card", "input_notify", "stream", "active", "active_type", "active_sub_type"} {
		for _, v := range foundItems[k] {
			parts = append(parts, message.UnknownPart{Type: k, Data: map[string]string{"data": v}})
		}
	}
	return parts
}

// PartsToFoundItems 把类型化 Parts 转回 legacy foundItems（供 adapter/qq 复用）。
func PartsToFoundItems(parts []message.MessagePart) (string, map[string][]string) {
	var text string
	found := map[string][]string{}
	for _, p := range parts {
		switch v := p.(type) {
		case message.TextPart:
			text += v.Text
		case message.MentionPart:
			text += "[CQ:at,qq=" + v.User + "]"
		case message.ReplyPart:
			found["reply_msg_id"] = append(found["reply_msg_id"], v.MessageID)
		case message.ImagePart:
			k, val := legacyMediaKey(v.Source)
			found[k+"image"] = append(found[k+"image"], val)
		case message.AudioPart:
			k, val := legacyMediaKey(v.Source)
			found[k+"record"] = append(found[k+"record"], val)
		case message.VideoPart:
			k, val := legacyMediaKey(v.Source)
			found[k+"video"] = append(found[k+"video"], val)
		case message.FilePart:
			k, val := legacyMediaKey(v.Source)
			found[k+"file"] = append(found[k+"file"], val)
			if v.Filename != "" {
				found["file_name"] = append(found["file_name"], v.Filename)
			}
		case message.MarkdownPart:
			found["markdown"] = append(found["markdown"], v.Content)
		case message.KeyboardPart:
			found["keyboard"] = append(found["keyboard"], v.Content)
		case message.QQMusicPart:
			found["qqmusic"] = append(found["qqmusic"], v.ID)
		case message.UnknownPart:
			if v.Type == wakeupMarker {
				continue
			}
			val := ""
			if v.Data != nil {
				val = v.Data["data"]
			}
			found[v.Type] = append(found[v.Type], val)
		}
	}
	return text, found
}

// mediaSourceFromLegacy 按 legacy key 前缀还原 MediaSource。
func mediaSourceFromLegacy(prefix, v string) message.MediaSource {
	switch prefix {
	case "local_":
		return message.MediaSource{Kind: message.MediaLocalFile, Path: v}
	case "url_":
		return message.MediaSource{Kind: message.MediaRemoteURL, URL: v}
	case "base64_":
		return message.MediaSource{Kind: message.MediaBase64, Data: []byte(v)}
	default:
		return message.MediaSource{Kind: message.MediaLocalFile, Path: v}
	}
}

// legacyMediaKey 与 message.compat.mediaKey 语义一致。
func legacyMediaKey(s message.MediaSource) (string, string) {
	switch s.Kind {
	case message.MediaLocalFile:
		return "local_", s.Path
	case message.MediaRemoteURL:
		return "url_", s.URL
	case message.MediaBase64:
		return "base64_", string(s.Data)
	default:
		return "unknown_", ""
	}
}

// ---- 供 adapter/qq 复用的 legacy 发送实现（复用 generateGroupMessage/generatePrivateMessage） ----

// SendFoundItemsGroup 按 legacy 语义把 foundItems 发送到群，返回 QQ 消息 ID。
func SendFoundItemsGroup(ctx context.Context, groupOpenID, messageText string, foundItems map[string][]string, apiv2 openapi.OpenAPI) (string, error) {
	messageID, eventID := resolveGroupMessageContext(ctx, groupOpenID, foundItems)
	var lastID string

	// 单图文混合（msg_type 7）
	imageType, imageURL, imageCount := pickSingleImage(foundItems)
	if imageCount == 1 && messageText != "" {
		singleItem := map[string][]string{imageType: {imageURL}}
		msgseq := nextMappingSeq(messageID)
		reply := generateGroupMessage(messageID, eventID, singleItem, "", msgseq, apiv2, groupOpenID)
		if rich, ok := reply.(*dto.RichMediaMessage); ok {
			fileInfo, err := uploadMedia(ctx, groupOpenID, rich, apiv2)
			if err != nil {
				return lastID, err
			}
			messageText = resolvePlainTextAtMentions(messageText)
			msgseq = nextMappingSeq(messageID)
			gm := &dto.MessageToCreate{
				Content: messageText,
				Media:   &dto.Media{FileInfo: fileInfo},
				MsgID:   messageID,
				EventID: eventID,
				MsgSeq:  msgseq,
				MsgType: 7,
			}
			gm.Timestamp = time.Now().Unix()
			applyGroupReply(gm, foundItems)
			resp, err := postGroupMessageWithEventIDFallback(ctx, apiv2, groupOpenID, gm)
			if resp != nil && resp.Message != nil {
				lastID = resp.Message.ID
			}
			pushSSMOn22009(err, groupOpenID, gm)
			if err != nil {
				return lastID, err
			}
			delete(foundItems, imageType)
			messageText = ""
		}
	}

	// 文本 / markdown / keyboard
	if strings.TrimSpace(messageText) != "" || len(foundItems["markdown"]) > 0 || len(foundItems["keyboard"]) > 0 {
		msgseq := nextMappingSeq(messageID)
		reply := generateGroupMessage(messageID, eventID, nil, messageText, msgseq, apiv2, groupOpenID)
		if gm, ok := reply.(*dto.MessageToCreate); ok {
			gm.Timestamp = time.Now().Unix()
			applyGroupMarkdownKeyboard(gm, foundItems, apiv2)
			applyGroupCard(gm, foundItems)
			applyGroupReply(gm, foundItems)
			resp, err := postGroupMessageWithEventIDFallback(ctx, apiv2, groupOpenID, gm)
			if resp != nil && resp.Message != nil {
				lastID = resp.Message.ID
			}
			pushSSMOn22009(err, groupOpenID, gm)
			if err != nil {
				return lastID, err
			}
		}
	}

	// 纯卡片（无文本）
	if cardItems, ok := foundItems["card"]; ok && len(cardItems) > 0 {
		if gm := buildGroupCard(cardItems[0], messageID, eventID); gm != nil {
			delete(foundItems, "card")
			resp, err := postGroupMessageWithEventIDFallback(ctx, apiv2, groupOpenID, gm)
			if resp != nil && resp.Message != nil {
				lastID = resp.Message.ID
			}
			pushSSMOn22009(err, groupOpenID, gm)
			if err != nil {
				return lastID, err
			}
		}
	}

	// 媒体循环
	for key, urls := range foundItems {
		if isControlKey(key) {
			continue
		}
		for i, url := range urls {
			singleItem := map[string][]string{key: {url}}
			if fileNames, ok := foundItems["file_name"]; ok && i < len(fileNames) {
				singleItem["file_name"] = []string{fileNames[i]}
			}
			msgseq := nextMappingSeq(messageID)
			reply := generateGroupMessage(messageID, eventID, singleItem, "", msgseq, apiv2, groupOpenID)
			if rich, ok := reply.(*dto.RichMediaMessage); ok {
				mr, err := apiv2.PostGroupMessage(ctx, groupOpenID, rich)
				if err == nil && mr != nil && mr.MediaResponse != nil && mr.MediaResponse.FileInfo != "" {
					msgseq = nextMappingSeq(messageID)
					content := rich.Content
					if content == "" {
						content = " "
					}
					gm := &dto.MessageToCreate{
						Content: content,
						MsgID:   messageID,
						EventID: eventID,
						MsgSeq:  msgseq,
						MsgType: 7,
						Media:   &dto.Media{FileInfo: mr.MediaResponse.FileInfo},
					}
					gm.Timestamp = time.Now().Unix()
					applyGroupReply(gm, foundItems)
					resp, err := postGroupMessageWithEventIDFallback(ctx, apiv2, groupOpenID, gm)
					if resp != nil && resp.Message != nil {
						lastID = resp.Message.ID
					}
					pushSSMOn22009(err, groupOpenID, gm)
					if err != nil {
						return lastID, err
					}
				}
			} else if gm, ok := reply.(*dto.MessageToCreate); ok {
				gm.Timestamp = time.Now().Unix()
				applyGroupReply(gm, foundItems)
				resp, err := postGroupMessageWithEventIDFallback(ctx, apiv2, groupOpenID, gm)
				if resp != nil && resp.Message != nil {
					lastID = resp.Message.ID
				}
				pushSSMOn22009(err, groupOpenID, gm)
				if err != nil {
					return lastID, err
				}
			}
		}
	}
	return lastID, nil
}

// SendFoundItemsPrivate 按 legacy 语义把 foundItems 发送到私聊，返回 QQ 消息 ID。
// wakeup=true 时使用 IsWakeup 语义（MsgID/EventID 置空）。
func SendFoundItemsPrivate(ctx context.Context, userOpenID, messageText string, foundItems map[string][]string, wakeup bool, apiv2 openapi.OpenAPI) (string, error) {
	messageID, eventID := resolvePrivateMessageContext(ctx, userOpenID, foundItems)
	if wakeup {
		messageID, eventID = "", ""
	}
	var lastID string

	imageType, imageURL, imageCount := pickSingleImage(foundItems)
	if imageCount == 1 && messageText != "" {
		singleItem := map[string][]string{imageType: {imageURL}}
		msgseq := nextMappingSeq(messageID)
		reply := generatePrivateMessage(messageID, eventID, singleItem, "", msgseq, apiv2, userOpenID)
		if rich, ok := reply.(*dto.RichMediaMessage); ok {
			fileInfo, err := uploadMediaPrivate(ctx, userOpenID, rich, apiv2)
			if err != nil {
				return lastID, err
			}
			messageText = resolvePlainTextAtMentions(messageText)
			msgseq = nextMappingSeq(messageID)
			gm := &dto.MessageToCreate{
				Content: messageText,
				Media:   &dto.Media{FileInfo: fileInfo},
				MsgID:   messageID,
				EventID: eventID,
				MsgSeq:  msgseq,
				MsgType: 7,
			}
			gm.Timestamp = time.Now().Unix()
			applyPrivateReply(gm, foundItems["reply_msg_id"], userOpenID)
			applyWakeup(gm, wakeup)
			resp, err := postC2CMessageWithEventIDFallback(ctx, apiv2, userOpenID, gm)
			if resp != nil && resp.Message != nil {
				lastID = resp.Message.ID
			}
			if err != nil {
				return lastID, err
			}
			delete(foundItems, imageType)
			messageText = ""
		}
	}

	if strings.TrimSpace(messageText) != "" || len(foundItems["markdown"]) > 0 || len(foundItems["keyboard"]) > 0 {
		msgseq := nextMappingSeq(messageID)
		reply := generatePrivateMessage(messageID, eventID, nil, messageText, msgseq, apiv2, userOpenID)
		if gm, ok := reply.(*dto.MessageToCreate); ok {
			gm.Timestamp = time.Now().Unix()
			applyPrivateMarkdownKeyboard(gm, foundItems, userOpenID, apiv2)
			applyPrivateReply(gm, foundItems["reply_msg_id"], userOpenID)
			applyWakeup(gm, wakeup)
			resp, err := postC2CMessageWithEventIDFallback(ctx, apiv2, userOpenID, gm)
			if resp != nil && resp.Message != nil {
				lastID = resp.Message.ID
			}
			if err != nil {
				return lastID, err
			}
		}
	}

	for key, urls := range foundItems {
		if isControlKey(key) {
			continue
		}
		for i, url := range urls {
			singleItem := map[string][]string{key: {url}}
			if fileNames, ok := foundItems["file_name"]; ok && i < len(fileNames) {
				singleItem["file_name"] = []string{fileNames[i]}
			}
			msgseq := nextMappingSeq(messageID)
			reply := generatePrivateMessage(messageID, eventID, singleItem, "", msgseq, apiv2, userOpenID)
			if rich, ok := reply.(*dto.RichMediaMessage); ok {
				mr, err := apiv2.PostC2CMessage(ctx, userOpenID, rich)
				if err == nil && mr != nil && mr.MediaResponse != nil && mr.MediaResponse.FileInfo != "" {
					msgseq = nextMappingSeq(messageID)
					content := rich.Content
					if content == "" {
						content = " "
					}
					gm := &dto.MessageToCreate{
						Content: content,
						MsgID:   messageID,
						EventID: eventID,
						MsgSeq:  msgseq,
						MsgType: 7,
						Media:   &dto.Media{FileInfo: mr.MediaResponse.FileInfo},
					}
					gm.Timestamp = time.Now().Unix()
					applyPrivateReply(gm, foundItems["reply_msg_id"], userOpenID)
					applyWakeup(gm, wakeup)
					resp, err := postC2CMessageWithEventIDFallback(ctx, apiv2, userOpenID, gm)
					if resp != nil && resp.Message != nil {
						lastID = resp.Message.ID
					}
					if err != nil {
						return lastID, err
					}
				}
			} else if gm, ok := reply.(*dto.MessageToCreate); ok {
				gm.Timestamp = time.Now().Unix()
				applyPrivateReply(gm, foundItems["reply_msg_id"], userOpenID)
				applyWakeup(gm, wakeup)
				resp, err := postC2CMessageWithEventIDFallback(ctx, apiv2, userOpenID, gm)
				if resp != nil && resp.Message != nil {
					lastID = resp.Message.ID
				}
				if err != nil {
					return lastID, err
				}
			}
		}
	}
	return lastID, nil
}

// ---- 内部辅助 ----

// resolveGroupMessageContext 解析群发送的 msg_id/event_id（legacy send_group_msg.go:193-314 语义）：
// lazy 池以虚拟群号（+虚拟用户号）为键（池键由 Processor/ProcessGroupMessage.go:293 写入）；
// echo 键兜底；无 user 上下文时清空缓存 msg_id（主动推送）。
func resolveGroupMessageContext(ctx context.Context, groupOpenID string, foundItems map[string][]string) (string, string) {
	c, hasCtx := outboundMsgCtxFrom(ctx)
	lazyGroupKey := groupOpenID
	if hasCtx && c.virtualGroupID != "" {
		lazyGroupKey = c.virtualGroupID
	}
	// 跨群路由时 groupOpenID 已是 CQ 目标群，但 msg_id/event_id 缓存与 SSM 属原群
	// （legacy send_group_msg.go:290/299/322/324 用 message.Params.GroupID）。
	lookupGroup := groupOpenID
	if hasCtx && c.origGroupOpenID != "" {
		lookupGroup = c.origGroupOpenID
	}
	messageID := ""
	if config.GetLazyMessageId() {
		if hasCtx && c.virtualUserID != "" {
			messageID = echo.GetLazyMessagesIdv2(lazyGroupKey, c.virtualUserID)
		} else {
			messageID = echo.GetLazyMessagesId(lazyGroupKey)
		}
	}
	if messageID == "" && hasCtx && c.echo != "" {
		messageID = echo.GetMsgIDByKey(c.echo)
	}
	// 逐用户兜底（legacy send_group_msg.go:286-295）：legacy 此处传真实 OpenID，
	// 由 StoreIDv2 反查回虚拟 ID 命中 AddMsgIDv2 写入的 (群,用户) 键。
	if messageID == "" && hasCtx && c.virtualUserID != "" {
		realUser := c.virtualUserID
		if real, err := idmap.RetrieveRowByIDv2(c.virtualUserID); err == nil && real != "" {
			realUser = real
		}
		messageID = GetMessageIDByUseridAndGroupid(config.GetAppIDStr(), realUser, lookupGroup)
	}
	if messageID == "" {
		messageID = GetMessageIDByUseridOrGroupid(config.GetAppIDStr(), lookupGroup)
	}
	// 主动推送：没有用户上下文时，缓存的 msg_id 已过期，清掉（legacy send_group_msg.go:289-293）
	if messageID != "" && (!hasCtx || c.virtualUserID == "") {
		messageID = ""
	}
	// C1：动作码预执行的 event_id 优先（legacy 中 CQ 执行晚于 2000 分支，CQ 值覆盖）
	eventID := ""
	if hasCtx {
		eventID = c.eventID
	}
	// CQ 显式决定过 eventID（member remove 清空）时不得回填缓存值：
	// legacy 中 CQ 执行在 2000 分支之后，清空结果即最终值。
	cqDecidedEventID := hasCtx && c.hasEventID
	if messageID == "2000" {
		messageID = ""
		if eventID == "" && !cqDecidedEventID {
			if !config.GetStringOb11() {
				eventID = GetEventIDByUseridOrGroupid(config.GetAppIDStr(), lookupGroup)
			} else {
				eventID = GetEventIDByUseridOrGroupidv2(config.GetAppIDStr(), lookupGroup)
			}
		}
	}
	if _, ok := foundItems["active"]; ok {
		messageID, eventID = "", ""
	}
	return messageID, eventID
}

// resolvePrivateMessageContext 解析私聊发送的 msg_id/event_id（legacy send_private_msg.go:323-349 语义）：
// lazy 池以虚拟用户号为键（group_private 时优先 (虚拟群号,虚拟用户号)）；echo 键兜底。
func resolvePrivateMessageContext(ctx context.Context, userOpenID string, foundItems map[string][]string) (string, string) {
	c, hasCtx := outboundMsgCtxFrom(ctx)
	lazyUserKey := userOpenID
	if hasCtx && c.virtualUserID != "" {
		lazyUserKey = c.virtualUserID
	}
	messageID := ""
	if config.GetLazyMessageId() {
		if hasCtx && c.virtualGroupID != "" && c.virtualUserID != "" {
			messageID = echo.GetLazyMessagesIdv2(c.virtualGroupID, c.virtualUserID)
		} else {
			messageID = echo.GetLazyMessagesId(lazyUserKey)
		}
	}
	if messageID == "" && hasCtx && c.echo != "" {
		messageID = echo.GetMsgIDByKey(c.echo)
	}
	if messageID == "" {
		messageID = GetMessageIDByUseridOrGroupid(config.GetAppIDStr(), userOpenID)
	}
	var eventID string
	if messageID == "2000" {
		messageID = ""
		eventID = GetEventIDByUseridOrGroupid(config.GetAppIDStr(), userOpenID)
	}
	if _, ok := foundItems["active"]; ok {
		messageID, eventID = "", ""
	}
	return messageID, eventID
}

// cloneFoundItems 深拷贝 foundItems（slice 独立底层数组）。
func cloneFoundItems(foundItems map[string][]string) map[string][]string {
	out := make(map[string][]string, len(foundItems))
	for k, v := range foundItems {
		out[k] = append([]string(nil), v...)
	}
	return out
}

func pickSingleImage(foundItems map[string][]string) (string, string, int) {
	for _, k := range []string{"local_image", "url_image", "url_images", "base64_image"} {
		if v, ok := foundItems[k]; ok && len(v) == 1 {
			return k, v[0], 1
		}
	}
	return "", "", 0
}

func isControlKey(key string) bool {
	switch key {
	case "active", "active_type", "active_sub_type", "reply_msg_id", "file_name", "wakeup":
		return true
	}
	return false
}

// pushSSMOn22009 在群消息因 22009（主动频控）失败时入队，等待下次被动回复补发（对齐 legacy send_group_msg.go:678-685）。
func pushSSMOn22009(err error, groupOpenID string, gm *dto.MessageToCreate) {
	if !IsQQError(err, 22009) {
		return
	}
	cid := echo.NextSSMCorrelationID(groupOpenID)
	mylog.Printf("[SSM][%s] 信息发送失败,加入到队列中,下次被动信息进行发送", cid)
	var pair echo.MessageGroupPair
	pair.Group = groupOpenID
	pair.GroupMessage = gm
	pair.CorrelationID = cid
	echo.PushGlobalStack(pair)
}

// postGroupMessageWithEventIDFallback 发送群消息；40034025（event_id 失效）时清空 EventID 重发一次
// （对齐 legacy send_group_msg.go:474-487/686-697）。
func postGroupMessageWithEventIDFallback(ctx context.Context, apiv2 openapi.OpenAPI, groupOpenID string, gm *dto.MessageToCreate) (*dto.GroupMessageResponse, error) {
	resp, err := apiv2.PostGroupMessage(ctx, groupOpenID, gm)
	if IsQQError(err, 40034025) {
		gm.EventID = ""
		resp, err = apiv2.PostGroupMessage(ctx, groupOpenID, gm)
	}
	return resp, err
}

// postC2CMessageWithEventIDFallback 发送私聊消息；40034025（event_id 失效）时清空 EventID 重发一次
// （对齐 legacy send_private_msg.go:459-465/575-582）。
func postC2CMessageWithEventIDFallback(ctx context.Context, apiv2 openapi.OpenAPI, userOpenID string, gm *dto.MessageToCreate) (*dto.C2CMessageResponse, error) {
	resp, err := apiv2.PostC2CMessage(ctx, userOpenID, gm)
	if IsQQError(err, 40034025) {
		gm.EventID = ""
		resp, err = apiv2.PostC2CMessage(ctx, userOpenID, gm)
	}
	return resp, err
}

func applyGroupReply(gm *dto.MessageToCreate, foundItems map[string][]string) {
	replyIDs, ok := foundItems["reply_msg_id"]
	if !ok || len(replyIDs) == 0 {
		return
	}
	realReplyID, err := idmap.RetrieveRowByCachev2(replyIDs[0])
	if err != nil || realReplyID == "" {
		return
	}
	parts := strings.Split(realReplyID, " ")
	refID := parts[len(parts)-1]
	gm.MessageReference = &dto.MessageReference{MessageID: ResolveReplyRefID(refID), IgnoreGetMessageError: false}
	gm.MsgID = refID
	gm.EventID = ""
}

func applyGroupMarkdownKeyboard(gm *dto.MessageToCreate, foundItems map[string][]string, apiv2 openapi.OpenAPI) {
	var md *dto.Markdown
	var kb *keyboard.MessageKeyboard
	if mdItems, ok := foundItems["markdown"]; ok && len(mdItems) > 0 {
		md, kb = parseMarkdownFromMessage(mdItems[0])
		if md != nil && md.Content != "" {
			md.Content = ResolveMarkdownAtMentions(md.Content)
			md.Content = ResolveMarkdownImages(md.Content, apiv2)
		}
		if kb != nil {
			ResolveKeyboardImages(kb, apiv2)
		}
		if md != nil {
			gm.Markdown = md
			gm.Keyboard = kb
			gm.MsgType = 2
			gm.Content = ""
			delete(foundItems, "markdown")
		}
	}
	if gm.Keyboard == nil {
		if kbItems, ok := foundItems["keyboard"]; ok && len(kbItems) > 0 {
			kb, err := parseKeyboardData([]byte(kbItems[0]))
			if err == nil && kb != nil {
				ResolveKeyboardImages(kb, apiv2)
				gm.Keyboard = kb
				delete(foundItems, "keyboard")
			}
		}
	}
}

func applyPrivateMarkdownKeyboard(gm *dto.MessageToCreate, foundItems map[string][]string, userOpenID string, apiv2 openapi.OpenAPI) {
	var md *dto.Markdown
	var kb *keyboard.MessageKeyboard
	if mdItems, ok := foundItems["markdown"]; ok && len(mdItems) > 0 {
		md, kb = parseMarkdownFromMessage(mdItems[0])
		if md != nil && md.Content != "" {
			md.Content = ResolveMarkdownAtMentions(md.Content)
			md.Content = ResolveMarkdownImages(md.Content, apiv2)
		}
		if kb != nil {
			ResolvePlaceholderUserIDs(kb, idmap.ResolveOriginalID(userOpenID))
			ResolveKeyboardImages(kb, apiv2)
		}
		if md != nil {
			gm.Markdown = md
			gm.Keyboard = kb
			gm.MsgType = 2
			gm.Content = ""
			delete(foundItems, "markdown")
		}
	}
	if gm.Keyboard == nil {
		if kbItems, ok := foundItems["keyboard"]; ok && len(kbItems) > 0 {
			kb, err := parseKeyboardData([]byte(kbItems[0]))
			if err == nil && kb != nil {
				ResolvePlaceholderUserIDs(kb, idmap.ResolveOriginalID(userOpenID))
				ResolveKeyboardImages(kb, apiv2)
				gm.Keyboard = kb
				delete(foundItems, "keyboard")
			}
		}
	}
}

func applyGroupCard(gm *dto.MessageToCreate, foundItems map[string][]string) {
	cardItems, ok := foundItems["card"]
	if !ok || len(cardItems) == 0 || gm.MsgType == 2 {
		return
	}
	var cardData map[string]string
	if err := json.Unmarshal([]byte(cardItems[0]), &cardData); err != nil {
		return
	}
	if cardData["pic"] == "" || cardData["url"] == "" {
		delete(foundItems, "card")
		return
	}
	gm.MsgType = 8
	gm.Content = ""
	gm.Card = &dto.GroupCard{
		Type: "tuwen",
		Content: &dto.GroupCardContent{
			Title:       cardData["title"],
			Description: cardData["desc"],
			PicURL:      resolveCardPic(cardData["pic"]),
			URL:         cardData["url"],
		},
	}
	delete(foundItems, "card")
}

func buildGroupCard(cardJSON, messageID, eventID string) *dto.MessageToCreate {
	var cardData map[string]string
	if err := json.Unmarshal([]byte(cardJSON), &cardData); err != nil {
		return nil
	}
	if cardData["pic"] == "" || cardData["url"] == "" {
		return nil
	}
	gm := &dto.MessageToCreate{
		MsgType: 8,
		MsgID:   messageID,
		EventID: eventID,
		MsgSeq:  echo.GetMappingSeq(messageID),
		Card: &dto.GroupCard{
			Type: "tuwen",
			Content: &dto.GroupCardContent{
				Title:       cardData["title"],
				Description: cardData["desc"],
				PicURL:      resolveCardPic(cardData["pic"]),
				URL:         cardData["url"],
			},
		},
	}
	gm.Timestamp = time.Now().Unix()
	return gm
}

func applyWakeup(gm *dto.MessageToCreate, wakeup bool) {
	if !wakeup {
		return
	}
	gm.IsWakeup = true
	gm.MsgID = ""
	gm.EventID = ""
}

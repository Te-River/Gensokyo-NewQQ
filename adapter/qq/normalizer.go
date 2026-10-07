package qq

import (
	"fmt"
	"regexp"
	"strings"
	"time"

	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/handlers"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/internal/domain/event"
	"github.com/hoshinonyaruko/gensokyo/internal/domain/identity"
	"github.com/hoshinonyaruko/gensokyo/internal/domain/message"
	"github.com/tencent-connect/botgo/dto"
)

// Normalizer 把 botgo DTO 归一化为 DomainEvent（实现 inbound.EventNormalizer）。
//
// 覆盖范围与 event.EventSource 的 7 个取值对齐；DTO 无法判别或未覆盖的事件
// （inline search / join request / msg reject / friend add-del 等）返回 error，
// 由 Processor 侧回退 legacy 广播。
type Normalizer struct{}

// NewNormalizer 创建 QQ 入站归一化器。
func NewNormalizer() *Normalizer { return &Normalizer{} }

// MemberEvent 包装 GroupMemberEvent 与其事件类型。
// dto.GroupMemberEvent 同时承载 GROUP_MEMBER_ADD / GROUP_MEMBER_REMOVE，
// 自身无判别字段，故由调用方（Processor）显式携带 eventType。
type MemberEvent struct {
	Data      *dto.GroupMemberEvent
	EventType string
}

// Normalize 实现 inbound.EventNormalizer。
func (n *Normalizer) Normalize(raw interface{}) (event.DomainEvent, error) {
	switch d := raw.(type) {
	case *dto.WSGroupATMessageData:
		return groupMessageEvent((*dto.Message)(d), event.SourceGroupAtMessage, raw), nil
	case *dto.WSGroupMessageData:
		return groupMessageEvent((*dto.Message)(d), event.SourceGroupMessage, raw), nil
	case *dto.WSC2CMessageData:
		return c2cMessageEvent((*dto.Message)(d), raw), nil
	case *MemberEvent:
		return groupMemberEvent(d)
	default:
		return event.DomainEvent{}, fmt.Errorf("qq: unsupported inbound event type %T", raw)
	}
}

func groupMessageEvent(m *dto.Message, src event.EventSource, raw interface{}) event.DomainEvent {
	ev := event.DomainEvent{
		ID:     m.ID,
		Time:   eventTime(m.Timestamp),
		Source: src,
		Target: identity.ResolvedTarget{
			Kind:  identity.TargetGroup,
			Group: &identity.ResolvedGroup{OpenID: identity.OpenGroupID(m.GroupID)},
		},
		Message: textMessage(m.Content),
		Raw:     raw,
	}
	if m.Author != nil {
		ev.Actor = identity.ResolvedUser{OpenID: identity.OpenID(m.Author.ID)}
	}
	return ev
}

func c2cMessageEvent(m *dto.Message, raw interface{}) event.DomainEvent {
	ev := event.DomainEvent{
		ID:      m.ID,
		Time:    eventTime(m.Timestamp),
		Source:  event.SourceC2CMessage,
		Target:  identity.ResolvedTarget{Kind: identity.TargetPrivate},
		Message: textMessage(m.Content),
		Raw:     raw,
	}
	if m.Author != nil {
		u := identity.ResolvedUser{OpenID: identity.OpenID(m.Author.ID)}
		ev.Actor = u
		ev.Target.User = &u
	}
	return ev
}

func groupMemberEvent(d *MemberEvent) (event.DomainEvent, error) {
	if d == nil || d.Data == nil {
		return event.DomainEvent{}, fmt.Errorf("qq: nil group member event")
	}
	var src event.EventSource
	switch d.EventType {
	case "GROUP_MEMBER_ADD":
		src = event.SourceGroupMemberAdd
	case "GROUP_MEMBER_REMOVE":
		src = event.SourceGroupMemberRemove
	default:
		return event.DomainEvent{}, fmt.Errorf("qq: unsupported group member event type %q", d.EventType)
	}
	return event.DomainEvent{
		ID:     d.Data.ID,
		Time:   eventTimeAny(d.Data.Timestamp),
		Source: src,
		Actor:  identity.ResolvedUser{OpenID: identity.OpenID(d.Data.MemberOpenID)},
		Target: identity.ResolvedTarget{
			Kind:  identity.TargetGroup,
			Group: &identity.ResolvedGroup{OpenID: identity.OpenGroupID(d.Data.GroupOpenID)},
		},
		Raw: d.Data,
	}, nil
}

// incomingAtRe 匹配入站 <@OpenID>（与 handlers 的 at 正则一致）。
var incomingAtRe = regexp.MustCompile(`<@!?([0-9A-Fa-f]+)>`)

// textMessage 把原始正文归一化为单一 TextPart，并做 <@OpenID>→[CQ:at,qq=虚拟ID] 转换，
// 与 legacy 入站（handlers.RevertTransformedText）保持一致。
func textMessage(content string) message.ParsedMessage {
	return message.ParsedMessage{Parts: []message.MessagePart{message.TextPart{Text: convertIncomingAt(content)}}}
}

// convertIncomingAt 把 <@OpenID> 映射为 [CQ:at,qq=虚拟ID]。
// @bot 复用 handlers.IsSelfAtID 判定；其余 OpenID 经 idmap 反查虚拟 ID。
func convertIncomingAt(content string) string {
	if !strings.Contains(content, "<@") {
		return content
	}
	return incomingAtRe.ReplaceAllStringFunc(content, func(m string) string {
		sub := incomingAtRe.FindStringSubmatch(m)
		if len(sub) < 2 {
			return m
		}
		id := sub[1]
		if handlers.IsSelfAtID(id) {
			return "[CQ:at,qq=" + selfAtTargetID() + "]"
		}
		if !config.GetConvertOtherAt() {
			return m
		}
		_, virtualID, err := idmap.RetrieveVirtualValuev2(id)
		if err != nil || virtualID == "" {
			return m
		}
		return "[CQ:at,qq=" + virtualID + "]"
	})
}

// selfAtTargetID 返回 @bot 自身映射到的 OneBot qq 值（与 handlers.selfAtTargetID 一致）。
func selfAtTargetID() string {
	if config.GetUseUin() {
		return config.GetUinStr()
	}
	if handlers.AppID != "" {
		return handlers.AppID
	}
	return config.GetAppIDStr()
}

// eventTime 解析 dto.Timestamp；缺失/非法时回退当前时间。
func eventTime(ts dto.Timestamp) time.Time {
	if ts == "" {
		return time.Now()
	}
	if t, err := ts.Time(); err == nil {
		return t
	}
	return time.Now()
}

// eventTimeAny 解析 interface{} 形态的时间戳（GroupMemberEvent.Timestamp）。
func eventTimeAny(v interface{}) time.Time {
	switch t := v.(type) {
	case string:
		if ts, err := dto.Timestamp(t).Time(); err == nil {
			return ts
		}
	case int64:
		return time.Unix(t, 0)
	case float64:
		return time.Unix(int64(t), 0)
	}
	return time.Now()
}

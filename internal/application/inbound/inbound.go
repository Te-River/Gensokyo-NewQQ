// Package inbound 定义入站事件管线：QQ Adapter → 归一化 → DomainEvent → OneBot Publisher。
package inbound

import (
	"context"

	"github.com/hoshinonyaruko/gensokyo/internal/domain/event"
	"github.com/hoshinonyaruko/gensokyo/internal/domain/message"
)

// EventNormalizer 把 QQ SDK DTO 归一化为 DomainEvent。
// 只有 QQ Adapter 允许 import botgo；本接口在 application 层。
type EventNormalizer interface {
	Normalize(raw interface{}) (event.DomainEvent, error)
}

// EventPublisher 把 DomainEvent 发布给 OneBot 客户端。
type EventPublisher interface {
	Publish(ctx context.Context, ev event.DomainEvent) error
}

// IsSelfMention 判断 @ 目标是否为机器人自身（@Bot 的唯一 canonical 实现）。
// selfIDs 传入机器人自身的所有表示（self OpenID、AppID 字符串等）。
func IsSelfMention(user string, selfIDs ...string) bool {
	if user == "" {
		return false
	}
	for _, s := range selfIDs {
		if user == s {
			return true
		}
	}
	return false
}

// NormalizeMentions 把 mentions 中表示机器人自身的条目统一为 selfID 表示。
// 防止 String serializer 修一次、Array serializer 再修一次（P8.3）。
func NormalizeMentions(selfID string, mentions []message.MentionPart, selfIDs ...string) []message.MentionPart {
	if selfID == "" {
		return mentions
	}
	out := make([]message.MentionPart, 0, len(mentions))
	for _, m := range mentions {
		if IsSelfMention(m.User, selfIDs...) {
			m.User = selfID
		}
		out = append(out, m)
	}
	return out
}

// Package event 定义入站领域事件（QQ Adapter → Inbound Application → OneBot Publisher）。
package event

import (
	"time"

	"github.com/hoshinonyaruko/gensokyo/internal/domain/identity"
	"github.com/hoshinonyaruko/gensokyo/internal/domain/message"
)

// EventSource 事件来源类型。
type EventSource uint8

const (
	SourceGroupMessage EventSource = iota + 1 // 普通群消息（无需 @）
	SourceGroupAtMessage                     // 群 @ 消息
	SourceC2CMessage                         // 私聊消息
	SourceFriendAdd                          // 添加好友
	SourceFriendDel                          // 删除好友
	SourceGroupMemberAdd                     // 群成员新增
	SourceGroupMemberRemove                  // 群成员移除
)

// DomainEvent 归一化后的入站领域事件。
// Raw 携带原始 QQ DTO（仅 QQ Adapter 消费，业务层不依赖）。
type DomainEvent struct {
	ID      string
	Time    time.Time
	Source  EventSource
	Actor   identity.ResolvedUser
	Target  identity.ResolvedTarget
	Message message.ParsedMessage

	Raw interface{}
}

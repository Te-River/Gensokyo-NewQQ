package qq

import (
	"context"
	"fmt"

	"github.com/hoshinonyaruko/gensokyo/handlers"
	"github.com/hoshinonyaruko/gensokyo/internal/application/outbound"
	"github.com/hoshinonyaruko/gensokyo/internal/domain/identity"
	"github.com/hoshinonyaruko/gensokyo/internal/domain/message"
	"github.com/tencent-connect/botgo/openapi"
)

// QQSender 是 outbound.QQSender 的 QQ 实现：
// 把类型化 OutboundMessage 转换为 botgo DTO 并投递。
// 复用 handlers 现有 msg_type 7/8、markdown、card、keyboard、wakeup、reply 语义，
// 保证与 legacy 出站逐字节兼容。
type QQSender struct {
	api   openapi.OpenAPI
	apiv2 openapi.OpenAPI
}

// NewQQSender 创建 QQ 出站适配器。
func NewQQSender(api, apiv2 openapi.OpenAPI) *QQSender {
	return &QQSender{api: api, apiv2: apiv2}
}

// Send 实现 outbound.QQSender。
func (s *QQSender) Send(ctx context.Context, target identity.ResolvedTarget, msg outbound.QQMessage) (outbound.QQSendResult, error) {
	text, found := handlers.PartsToFoundItems(msg.Parts)
	if msg.Reply != nil && msg.Reply.MessageID != "" {
		found["reply_msg_id"] = []string{msg.Reply.MessageID}
	}
	wakeup := hasWakeupMarker(msg.Parts)

	switch target.Kind {
	case identity.TargetGroup:
		if target.Group == nil {
			return outbound.QQSendResult{}, fmt.Errorf("qq: group target missing")
		}
		id, err := handlers.SendFoundItemsGroup(ctx, target.Group.OpenID.String(), text, found, s.apiv2)
		return outbound.QQSendResult{MessageID: id}, err
	case identity.TargetPrivate:
		if target.User == nil {
			return outbound.QQSendResult{}, fmt.Errorf("qq: private target missing")
		}
		id, err := handlers.SendFoundItemsPrivate(ctx, target.User.OpenID.String(), text, found, wakeup, s.apiv2)
		return outbound.QQSendResult{MessageID: id}, err
	default:
		return outbound.QQSendResult{}, fmt.Errorf("qq: unsupported target kind %d", target.Kind)
	}
}

// hasWakeupMarker 检测 bridge 编码的 wakeup 标记。
func hasWakeupMarker(parts []message.MessagePart) bool {
	for _, p := range parts {
		if u, ok := p.(message.UnknownPart); ok && u.Type == "wakeup" {
			return true
		}
	}
	return false
}

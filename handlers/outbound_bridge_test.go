package handlers

import (
	"testing"

	"github.com/hoshinonyaruko/gensokyo/internal/application/outbound"
	"github.com/hoshinonyaruko/gensokyo/internal/domain/identity"
	"github.com/hoshinonyaruko/gensokyo/internal/domain/message"
)

// TestOutbound 验证 A 包出站桥：foundItems → OutboundCommand → Parts → foundItems
// 的往返一致性（shadow diff 的核心），以及 reply/wakeup 语义的保留。
func TestOutbound(t *testing.T) {
	target := identity.ResolvedTarget{
		Kind:  identity.TargetGroup,
		Group: &identity.ResolvedGroup{OpenID: "OPENID_GROUP", VirtualGroupID: "12345"},
	}

	foundItems := map[string][]string{
		"local_image": {"C:/tmp/a.png"},
		"url_image":   {"example.com/b.png"},
		"markdown":    {`{"content":"hi"}`},
		"keyboard":    {`{"rows":[]}`},
		"qqmusic":     {"songid"},
		"reply_msg_id": {"999"},
		"card":        {`{"title":"t","desc":"d","pic":"p","url":"u"}`},
		"active":      {""},
	}
	messageText := "hello [CQ:at,qq=1]"

	cmd := BuildOutboundCommand(messageText, foundItems, target, outbound.DeliveryPolicy{Mode: 1})

	// reply 语义
	if cmd.Message.Reply == nil || cmd.Message.Reply.MessageID != "999" {
		t.Fatalf("reply 未保留: %+v", cmd.Message.Reply)
	}
	// target 语义
	if cmd.Target.Kind != identity.TargetGroup || cmd.Target.Group.OpenID != "OPENID_GROUP" {
		t.Fatalf("target 未保留: %+v", cmd.Target)
	}

	// 往返：Parts → foundItems
	rtText, rtFound := PartsToFoundItems(cmd.Message.Parts)
	if rtText != messageText {
		t.Fatalf("text 往返不一致: got %q want %q", rtText, messageText)
	}
	for _, k := range []string{"local_image", "url_image", "markdown", "keyboard", "qqmusic", "card"} {
		if got, want := rtFound[k], foundItems[k]; len(got) != len(want) || (len(got) > 0 && got[0] != want[0]) {
			t.Fatalf("key %q 往返不一致: got %v want %v", k, got, want)
		}
	}
	// active 控制型 key 必须随 Parts 往返保留（否则 new 路径主动推送退化为被动）
	if got, ok := rtFound["active"]; !ok || len(got) != 1 || got[0] != "" {
		t.Fatalf("active 应随往返保留: got %v ok=%v", got, ok)
	}
}

// TestOutboundWakeupMarker 验证 wakeup 标记随 Parts 携带且不污染 foundItems。
func TestOutboundWakeupMarker(t *testing.T) {
	target := identity.ResolvedTarget{
		Kind: identity.TargetPrivate,
		User: &identity.ResolvedUser{OpenID: "OPENID_USER", VirtualUserID: "42"},
	}
	cmd := BuildOutboundCommand("hi", map[string][]string{}, target, outbound.DeliveryPolicy{Wakeup: true})
	marked := false
	for _, p := range cmd.Message.Parts {
		if u, ok := p.(message.UnknownPart); ok && u.Type == wakeupMarker {
			marked = true
		}
	}
	if !marked {
		t.Fatalf("wakeup 标记缺失")
	}
	_, found := PartsToFoundItems(cmd.Message.Parts)
	if _, ok := found["wakeup"]; ok {
		t.Fatalf("wakeup 标记不应进入 foundItems")
	}
}

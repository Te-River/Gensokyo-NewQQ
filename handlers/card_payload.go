package handlers

import (
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"unicode/utf8"

	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/dto"
)

// ---------- 图文卡片（msg_type=8）单点校验 ----------
// 官方 2026-10 消息接口页：title 1~50 字符、description 1~200、
// url 与 pic_url 必须 https 且不超过 1024。群聊与单聊载荷同构：
// card{type:"tuwen", content{title, description, pic_url, url}}。

const (
	cardTitleMaxRunes = 50
	cardDescMaxRunes  = 200
	cardURLMaxLen     = 1024
)

// buildTuwenCard 是所有 [CQ:card] 构造点的唯一实现与唯一校验点。
// 官方要求 content 必须同时包含 title / description / url / pic_url 四项，缺一即 1400；
// title / description 去首尾空白后判空并按字符计长，url / pic_url 必须 https 且按字节计长
// （与 Rust 侧保持同一口径：字节更保守）。返回错误时调用方统一回 retcode=1400。
func buildTuwenCard(title, desc, pic, url string) (*dto.GroupCard, error) {
	title = strings.TrimSpace(title)
	if title == "" {
		return nil, errors.New("card.title 缺失：图文卡片必须提供 title（1~50 字符）")
	}
	if n := utf8.RuneCountInString(title); n > cardTitleMaxRunes {
		return nil, fmt.Errorf("card.title 长度 %d 超上限 %d", n, cardTitleMaxRunes)
	}

	desc = strings.TrimSpace(desc)
	if desc == "" {
		return nil, errors.New("card.description 缺失：图文卡片必须提供 description（1~200 字符）")
	}
	if n := utf8.RuneCountInString(desc); n > cardDescMaxRunes {
		return nil, fmt.Errorf("card.description 长度 %d 超上限 %d", n, cardDescMaxRunes)
	}

	pic = strings.TrimSpace(pic)
	if err := validateCardURL("pic_url", pic); err != nil {
		return nil, err
	}

	url = strings.TrimSpace(url)
	if err := validateCardURL("url", url); err != nil {
		return nil, err
	}

	return &dto.GroupCard{
		Type: "tuwen",
		Content: &dto.GroupCardContent{
			Title:       title,
			Description: desc,
			PicURL:      pic,
			URL:         url,
		},
	}, nil
}

// validateCardURL 校验卡片链接：四项必填故空值也算缺失，且必须 https、按字节不超 1024。
// 本地路径与 http 链接一律拒绝（需要图床请先走现有上传接口）。
func validateCardURL(field, raw string) error {
	if raw == "" {
		return fmt.Errorf("card.%s 缺失：图文卡片必须提供 %s（https，≤%d 字节）", field, field, cardURLMaxLen)
	}
	if len(raw) > cardURLMaxLen {
		return fmt.Errorf("card.%s 长度 %d 字节超上限 %d", field, len(raw), cardURLMaxLen)
	}
	if !strings.HasPrefix(strings.ToLower(raw), "https://") {
		return fmt.Errorf("card.%s 必须是 https 链接（收到 %q）", field, raw)
	}
	return nil
}

// cardFieldsFromMap 从 [CQ:card] 参数表取四字段，兼容别名
// （desc/description、pic/pic_url）。
func cardFieldsFromMap(cardData map[string]string) (title, desc, pic, url string) {
	title = cardData["title"]
	desc = firstNonEmptyCardValue(cardData["desc"], cardData["description"])
	pic = firstNonEmptyCardValue(cardData["pic"], cardData["pic_url"])
	url = cardData["url"]
	return
}

func firstNonEmptyCardValue(values ...string) string {
	for _, v := range values {
		if strings.TrimSpace(v) != "" {
			return v
		}
	}
	return ""
}

// buildCardFromJSON 解析 [CQ:card] 的 JSON 参数并构造卡片，解析失败也回中文原因。
func buildCardFromJSON(cardJSON string) (*dto.GroupCard, error) {
	var cardData map[string]string
	if err := json.Unmarshal([]byte(cardJSON), &cardData); err != nil {
		mylog.Warnf("[CQ:card] 参数解析失败，原始值: %q", truncateCardLogValue(cardJSON))
		return nil, fmt.Errorf("card 参数解析失败: %w", err)
	}
	title, desc, pic, url := cardFieldsFromMap(cardData)
	card, err := buildTuwenCard(title, desc, pic, url)
	if err != nil {
		// 四字段必填是相对旧行为的破坏性变更：拒绝时留痕原始值便于排查调用方
		mylog.Warnf("[CQ:card] 拒绝卡片 title=%q description=%q url=%q pic_url=%q 原因: %v",
			truncateCardLogValue(title), truncateCardLogValue(desc),
			truncateCardLogValue(url), truncateCardLogValue(pic), err)
		return nil, err
	}
	return card, nil
}

// truncateCardLogValue 把原始字段截断到 64 字符，避免超长 URL / 内嵌 base64 刷屏日志。
func truncateCardLogValue(v string) string {
	const maxRunes = 64
	runes := []rune(v)
	if len(runes) <= maxRunes {
		return v
	}
	return string(runes[:maxRunes]) + "..."
}

// applyCardToMessage 把校验通过的卡片挂到消息上（msg_type=8、content 清空），
// 并从 foundItems 摘除 card，避免后续媒体循环重复发送。
func applyCardToMessage(gm *dto.MessageToCreate, cardItems []string) error {
	card, err := buildCardFromJSON(cardItems[0])
	if err != nil {
		return err
	}
	if len(cardItems) > 1 {
		mylog.Printf("[CQ:card] 收到 %d 份卡片，仅发送第一份", len(cardItems))
	}
	gm.MsgType = 8
	gm.Content = ""
	gm.Card = card
	return nil
}

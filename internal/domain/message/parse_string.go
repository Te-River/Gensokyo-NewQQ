package message

import "strings"

// ParseOneBotString 解析 OneBot 字符串消息（含 CQ 码）为类型化消息。
// 纯函数：不产生任何 IO。
func ParseOneBotString(s string) (ParsedMessage, error) {
	var parts []MessagePart
	var reply *ReplyPart
	var text strings.Builder

	i := 0
	for i < len(s) {
		start := strings.Index(s[i:], "[CQ:")
		if start < 0 {
			text.WriteString(s[i:])
			break
		}
		start += i

		// [CQ: 之前的普通文本（不做 CQ 反转义），按位置 flush 为 TextPart
		if start > i {
			text.WriteString(s[i:start])
		}
		if text.Len() > 0 {
			parts = append(parts, TextPart{Text: text.String()})
			text.Reset()
		}

		end := strings.IndexByte(s[start:], ']')
		if end < 0 {
			// malformed：无结束括号，剩余按文本处理
			text.WriteString(s[start:])
			break
		}
		end += start

		body := s[start+4 : end] // 去掉 [CQ:
		colon := strings.IndexByte(body, ',')
		var cqType string
		var paramsPart string
		if colon < 0 {
			cqType = body
		} else {
			cqType = body[:colon]
			paramsPart = body[colon+1:]
		}
		// 统一契约 §1：段名之后、`,` 或 `]` 之前允许任意空白（段名前空白不放宽，维持现状）
		// 空白集为 ASCII 五元集 {空格, \t, \n, \f, \r}，与 legacy 正则 `\s` 及 Rust is_cq_space 同口径（G-A）
		cqType = strings.TrimRight(cqType, " \t\r\n\f")

		part := cqTypeToPart(cqType, parseCQParams(paramsPart))
		if rp, ok := part.(ReplyPart); ok {
			reply = &rp
		}
		parts = append(parts, part)
		i = end + 1
	}

	if text.Len() > 0 {
		parts = append(parts, TextPart{Text: text.String()})
	}
	return ParsedMessage{Parts: parts, Reply: reply}, nil
}

// cqTypeToPart 把 CQ 码类型与参数映射为消息段。
func cqTypeToPart(t string, params map[string]string) MessagePart {
	switch t {
	case "text":
		return TextPart{Text: params["text"]}
	case "at":
		return MentionPart{User: params["qq"]}
	case "image":
		return ImagePart{Source: MediaSourceFromFile(params["file"]), File: params["file"]}
	case "record":
		return AudioPart{Source: MediaSourceFromFile(params["file"]), File: params["file"]}
	case "video":
		return VideoPart{Source: MediaSourceFromFile(params["file"]), File: params["file"]}
	case "file":
		return FilePart{
			Source:   MediaSourceFromFile(params["file"]),
			Filename: params["name"],
			File:     params["file"],
		}
	case "reply":
		return ReplyPart{MessageID: params["id"]}
	case "markdown":
		return MarkdownPart{Content: params["data"]}
	case "keyboard":
		return KeyboardPart{Content: params["data"]}
	case "music":
		return QQMusicPart{ID: params["id"]}
	default:
		return UnknownPart{Type: t, Data: params}
	}
}

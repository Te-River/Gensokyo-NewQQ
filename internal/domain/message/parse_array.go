package message

// ParseOneBotSegments 解析 OneBot 消息段数组（array 形式）为类型化消息。
// 纯函数：不产生任何 IO。
func ParseOneBotSegments(segs []Segment) (ParsedMessage, error) {
	var parts []MessagePart
	var reply *ReplyPart

	for _, seg := range segs {
		part := segmentToPart(seg)
		if rp, ok := part.(ReplyPart); ok {
			reply = &rp
		}
		parts = append(parts, part)
	}
	return ParsedMessage{Parts: parts, Reply: reply}, nil
}

// segmentToPart 把 OneBot 消息段映射为消息段。
func segmentToPart(seg Segment) MessagePart {
	switch seg.Type {
	case "text":
		return TextPart{Text: seg.Data["text"]}
	case "at":
		return MentionPart{User: seg.Data["qq"]}
	case "image":
		return ImagePart{Source: MediaSourceFromFile(seg.Data["file"]), File: seg.Data["file"]}
	case "record":
		return AudioPart{Source: MediaSourceFromFile(seg.Data["file"]), File: seg.Data["file"]}
	case "video":
		return VideoPart{Source: MediaSourceFromFile(seg.Data["file"]), File: seg.Data["file"]}
	case "file":
		return FilePart{
			Source:   MediaSourceFromFile(seg.Data["file"]),
			Filename: seg.Data["name"],
			File:     seg.Data["file"],
		}
	case "reply":
		return ReplyPart{MessageID: seg.Data["id"]}
	case "markdown":
		return MarkdownPart{Content: seg.Data["data"]}
	case "keyboard":
		return KeyboardPart{Content: seg.Data["data"]}
	case "qqmusic":
		return QQMusicPart{ID: seg.Data["id"]}
	default:
		return UnknownPart{Type: seg.Type, Data: seg.Data}
	}
}

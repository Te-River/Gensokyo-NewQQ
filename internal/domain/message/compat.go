package message

// 本文件是迁移期 compat bridge：typed ParsedMessage <-> 旧结构。
// 仅用于双轨迁移对比，新业务不得依赖；最终随 legacy 一起删除（P13）。

// Canonicalize 把类型化消息转回 OneBot 消息段数组（纯数据）。
// 用于 String/Array 两种解析入口的一致性比较与旧格式回写。
func Canonicalize(pm ParsedMessage) []Segment {
	var out []Segment
	for _, p := range pm.Parts {
		switch v := p.(type) {
		case TextPart:
			out = append(out, Segment{Type: "text", Data: map[string]string{"text": v.Text}})
		case MentionPart:
			out = append(out, Segment{Type: "at", Data: map[string]string{"qq": v.User}})
		case ImagePart:
			out = append(out, Segment{Type: "image", Data: map[string]string{"file": v.File}})
		case AudioPart:
			out = append(out, Segment{Type: "record", Data: map[string]string{"file": v.File}})
		case VideoPart:
			out = append(out, Segment{Type: "video", Data: map[string]string{"file": v.File}})
		case FilePart:
			d := map[string]string{"file": v.File}
			if v.Filename != "" {
				d["name"] = v.Filename
			}
			out = append(out, Segment{Type: "file", Data: d})
		case ReplyPart:
			out = append(out, Segment{Type: "reply", Data: map[string]string{"id": v.MessageID}})
		case MarkdownPart:
			out = append(out, Segment{Type: "markdown", Data: map[string]string{"data": v.Content}})
		case KeyboardPart:
			out = append(out, Segment{Type: "keyboard", Data: map[string]string{"data": v.Content}})
		case QQMusicPart:
			out = append(out, Segment{Type: "qqmusic", Data: map[string]string{"id": v.ID}})
		case UnknownPart:
			out = append(out, Segment{Type: v.Type, Data: v.Data})
		}
	}
	return out
}

// mediaKey 按媒体来源返回 legacy foundItems key。
func mediaKey(media MediaSource) (string, string) {
	switch media.Kind {
	case MediaLocalFile:
		return "local_", media.Path
	case MediaRemoteURL:
		return "url_", media.URL
	case MediaBase64:
		return "base64_", string(media.Data)
	default:
		return "unknown_", ""
	}
}

// ToLegacyFoundItems 把类型化消息转换为旧的 foundItems map 结构。
// messageText 累积纯文本与文本型 CQ 码（at 等）。
func (pm ParsedMessage) ToLegacyFoundItems() (string, map[string][]string) {
	var text string
	found := map[string][]string{}

	for _, p := range pm.Parts {
		switch v := p.(type) {
		case TextPart:
			text += v.Text
		case MentionPart:
			text += "[CQ:at,qq=" + v.User + "]"
		case ReplyPart:
			found["reply_msg_id"] = append(found["reply_msg_id"], v.MessageID)
		case ImagePart:
			key, val := mediaKey(v.Source)
			found[key+"image"] = append(found[key+"image"], val)
		case AudioPart:
			key, val := mediaKey(v.Source)
			found[key+"record"] = append(found[key+"record"], val)
		case VideoPart:
			key, val := mediaKey(v.Source)
			found[key+"video"] = append(found[key+"video"], val)
		case FilePart:
			key, val := mediaKey(v.Source)
			found[key+"file"] = append(found[key+"file"], val)
			if v.Filename != "" {
				found["file_name"] = append(found["file_name"], v.Filename)
			}
		case MarkdownPart:
			found["markdown"] = append(found["markdown"], v.Content)
		case KeyboardPart:
			found["keyboard"] = append(found["keyboard"], v.Content)
		case QQMusicPart:
			found["qqmusic"] = append(found["qqmusic"], v.ID)
		case UnknownPart:
			found["unknown_"+v.Type] = append(found["unknown_"+v.Type], "")
		}
	}
	return text, found
}

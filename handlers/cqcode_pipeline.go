package handlers

import (
	"encoding/base64"
	"encoding/json"
	"regexp"
)

// ---------- 统一 CQ 码解析管道 ----------

// ProcessCQCodePipeline 统一解析消息文本中的所有 CQ 码字符串。
// 无论消息来自 string 还是消息段数组（[]interface{}）路径，均经过本管道：
//   - 媒体类（image/record/video/file）：存入 foundItems 对应 key（url/base64/本地）
//   - 控制类（markdown/card/input_notify/stream/active/keyboard/qqmusic/reply/avatar）：存入对应 key
//   - 未知类型（at 等）原样保留，由下游 transformMessageTextAt 等继续处理
//
// 返回剔除已解析 CQ 码后的文本。
func ProcessCQCodePipeline(text string, foundItems map[string][]string, groupID interface{}) string {
	// 初始化平台相关正则（Windows/Unix 本地路径前缀差异）
	compilePatternsOnce.Do(initPlatformPatterns)

	// 1. 媒体类：image / record / video（URL / base64 / 本地路径，单次逐正则提取）
	text = processCQMediaPipeline(text, foundItems)

	// 2. Markdown：base64 直接存入；JSON 编码后存入
	text = processCQMarkdownPipeline(text, foundItems)

	// 3. 卡片消息 [CQ:card,...]：参数提取为 JSON 存入
	text = processCQCardPipeline(text, foundItems)

	// 4. 输入状态 [CQ:input_notify,...]
	text = processCQInputNotifyPipeline(text, foundItems)

	// 5. 流式消息 [CQ:stream,...]
	text = processCQStreamPipeline(text, foundItems)

	// 6. QQ 音乐 [CQ:music,type=qq,id=...]
	text = processCQQQMusicPipeline(text, foundItems)

	// 7. active / file / keyboard（复用既有集中函数）
	text = ProcessCQActive(text, foundItems)
	text = ProcessCQFile(text, foundItems)
	text = ProcessCQKeyboard(text, foundItems)

	// 8. 回复引用 [CQ:reply,id=数字]
	text = processCQReplyPipeline(text, foundItems)

	// 9. 头像 [CQ:avatar,qq=...]（字符串形式，与消息段 avatar 等价）
	if groupID == nil {
		text = ProcessCQAvatarNoGroupID(text)
	} else {
		text = ProcessCQAvatar(groupID.(string), text)
	}

	return text
}

// processCQMediaPipeline 媒体类 CQ 码（image/record/video）统一提取
func processCQMediaPipeline(text string, foundItems map[string][]string) string {
	patterns := []struct {
		key     string
		pattern *regexp.Regexp
	}{
		{"local_image", localImagePattern},
		{"url_image", httpUrlImagePattern},
		{"url_images", httpsUrlImagePattern},
		{"base64_image", base64ImagePattern},
		{"base64_record", base64RecordPattern},
		{"local_record", localRecordPattern},
		{"url_record", httpUrlRecordPattern},
		{"url_records", httpsUrlRecordPattern},
		{"url_video", httpUrlVideoPattern},
		{"url_videos", httpsUrlVideoPattern},
		{"base64_video", base64VideoPattern},
		{"local_video", localVideoPattern},
	}
	for _, p := range patterns {
		matches := p.pattern.FindAllStringSubmatch(text, -1)
		for _, m := range matches {
			if len(m) > 1 {
				foundItems[p.key] = append(foundItems[p.key], m[1])
			}
		}
		text = p.pattern.ReplaceAllString(text, "")
	}
	return text
}

// processCQMarkdownPipeline [CQ:markdown,data=...] 统一提取
// base64 形式直接存入；JSON 形式 base64 编码后存入
func processCQMarkdownPipeline(text string, foundItems map[string][]string) string {
	text = mdJSONPattern.ReplaceAllStringFunc(text, func(match string) string {
		if submatch := mdJSONPattern.FindStringSubmatch(match); len(submatch) > 1 {
			encoded := base64.StdEncoding.EncodeToString([]byte(submatch[1]))
			foundItems["markdown"] = append(foundItems["markdown"], encoded)
		}
		return ""
	})
	text = mdPattern.ReplaceAllStringFunc(text, func(match string) string {
		if submatch := mdPattern.FindStringSubmatch(match); len(submatch) > 1 {
			foundItems["markdown"] = append(foundItems["markdown"], submatch[1])
		}
		return ""
	})
	return text
}

// processCQCardPipeline [CQ:card,...]：参数顺序无关，JSON 编码存入
func processCQCardPipeline(text string, foundItems map[string][]string) string {
	return cardPattern.ReplaceAllStringFunc(text, func(match string) string {
		cardData := make(map[string]string)
		kvRe := regexp.MustCompile(`(\w+)=([^,\]]+)`)
		for _, kv := range kvRe.FindAllStringSubmatch(match, -1) {
			if len(kv) == 3 {
				cardData[kv[1]] = kv[2]
			}
		}
		if len(cardData) > 0 {
			encoded, err := json.Marshal(cardData)
			if err == nil {
				foundItems["card"] = append(foundItems["card"], string(encoded))
			}
		}
		return ""
	})
}

// processCQInputNotifyPipeline [CQ:input_notify,type=...,second=...]
func processCQInputNotifyPipeline(text string, foundItems map[string][]string) string {
	return inputNotifyPattern.ReplaceAllStringFunc(text, func(match string) string {
		if submatch := inputNotifyPattern.FindStringSubmatch(match); len(submatch) > 1 {
			notifyData := map[string]string{
				"type": submatch[1],
			}
			if len(submatch) > 2 && submatch[2] != "" {
				notifyData["second"] = submatch[2]
			}
			encoded, err := json.Marshal(notifyData)
			if err == nil {
				foundItems["input_notify"] = append(foundItems["input_notify"], string(encoded))
			}
		}
		return ""
	})
}

// processCQStreamPipeline [CQ:stream,type:xxx,qq:xxx]
func processCQStreamPipeline(text string, foundItems map[string][]string) string {
	return streamPattern.ReplaceAllStringFunc(text, func(match string) string {
		if submatch := streamPattern.FindStringSubmatch(match); len(submatch) > 2 {
			streamData := map[string]string{
				"type": submatch[1],
				"qq":   submatch[2],
			}
			encoded, err := json.Marshal(streamData)
			if err == nil {
				foundItems["stream"] = append(foundItems["stream"], string(encoded))
			}
		}
		return ""
	})
}

// processCQQQMusicPipeline [CQ:music,type=qq,id=...]
func processCQQQMusicPipeline(text string, foundItems map[string][]string) string {
	return qqMusicPattern.ReplaceAllStringFunc(text, func(match string) string {
		if submatch := qqMusicPattern.FindStringSubmatch(match); len(submatch) > 1 {
			foundItems["qqmusic"] = append(foundItems["qqmusic"], submatch[1])
		}
		return ""
	})
}

// processCQReplyPipeline [CQ:reply,id=数字] → reply_msg_id（构建 message_reference）
func processCQReplyPipeline(text string, foundItems map[string][]string) string {
	for _, matches := range replyRe.FindAllStringSubmatch(text, -1) {
		if len(matches) > 1 {
			foundItems["reply_msg_id"] = append(foundItems["reply_msg_id"], matches[1])
		}
	}
	return replyRe.ReplaceAllString(text, "")
}
package handlers

import (
	"encoding/base64"
	"encoding/json"
	"strings"
)

// ---------- 统一 CQ 码解析管道 ----------

// ProcessCQCodePipeline 统一解析消息文本中的所有 CQ 码字符串。
// 无论消息来自 string 还是消息段数组（[]interface{}）路径，均经过本管道：
//   - 媒体类（image/record/video/file）：存入 foundItems 对应 key（url/base64/本地）
//   - 控制类（markdown/card/input_notify/stream/active/keyboard/qqmusic/reply/avatar）：存入对应 key
//   - 入站专用（ark / reply,msg_idx=）：出站无消费点，整段剥离
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

	// 2.1 markdown 载荷内的 @全员 归一：载荷在这里已 base64，文本侧扫描覆盖不到（F6）
	normalizeAtEveryoneInFoundItems(foundItems)

	// 2.5 @全员归一：[CQ:at,qq=all|everyone] → 官方 <qqbot-at-everyone />
	// 先于 card 阶段，保证文本与 markdown 两条出站路径同源（P-G2）
	text = NormalizeAtEveryone(text)

	// 3. 卡片消息 [CQ:card,...]：参数提取为 JSON 存入
	text = processCQCardPipeline(text, foundItems)

	// 4. 输入状态 [CQ:input_notify,...]
	text = processCQInputNotifyPipeline(text, foundItems)

	// 5. 流式消息 [CQ:stream,...]
	text = processCQStreamPipeline(text, foundItems)

	// 6. QQ 音乐 [CQ:music,type=qq,id=...]
	text = processCQQQMusicPipeline(text, foundItems)

	// 7. active / file / keyboard / wakeup（复用既有集中函数）
	text = ProcessCQActive(text, foundItems)
	text = ProcessCQFile(text, foundItems)
	text = ProcessCQKeyboard(text, foundItems)
	text = ProcessCQWakeup(text, foundItems)

	// 8. 回复引用 [CQ:reply,id=数字]
	text = processCQReplyPipeline(text, foundItems)

	// 9. 头像 [CQ:avatar,qq=...]（字符串形式，与消息段 avatar 等价）
	if groupID == nil {
		text = ProcessCQAvatarNoGroupID(text)
	} else {
		text = ProcessCQAvatar(groupID.(string), text)
	}

	// 10. 入站专用码 [CQ:ark...] / [CQ:reply,msg_idx=...]：出站无消费点，整段剥离。
	// 置于最后，保证前面各步的输入文本与新增剥离逻辑前完全一致
	text = ProcessCQInboundOnlyCodes(text)

	return text
}

// processCQMediaPipeline 媒体类 CQ 码（image/record/video）统一提取。
// 修 F1：整码匹配 + file 按键取值（段名整词命中、任意键序、键名与段名大小写不敏感、
// 段名前后容忍空白、参数区花括号配平），因此
// `[CQ:image,file=https://x/a.png,name=y]` 的尾键不再污染 URL；
// record 的 ,asr= 仍不落入值内（asr 自身成键，口径与原 (?:,asr=...)? 切分等价），
// 而值自带逗号（`file=https://x/a,b.wav`）经 C3 并段完整保留。
// 无 scheme / 未知 scheme 的 file 值不消费，整码字面透传（现状）。
func processCQMediaPipeline(text string, foundItems map[string][]string) string {
	for _, kind := range []string{"image", "record", "video"} {
		text = cqApplyNamedCodes(text, kind, false, func(match string, params map[string]string) string {
			key, value, ok := cqMediaItem(params["file"], kind)
			if !ok {
				// 无 scheme / 未知 scheme 的 file 值不消费，整码字面透传（现状，修 F1 边界）
				return match
			}
			foundItems[key] = append(foundItems[key], value)
			return ""
		})
	}
	return text
}

// processCQMarkdownPipeline [CQ:markdown,data=...] 统一提取：整码匹配 + data 按键取值
// （修 F1，尾键如 `,note=x` 不再污染载荷）。base64:// 形式直接存入；{...} 原始 JSON
// 形式 base64 编码后存入（两种形态口径与既有 mdPattern / mdJSONPattern 一致）。
func processCQMarkdownPipeline(text string, foundItems map[string][]string) string {
	return cqApplyNamedCodes(text, "markdown", false, func(match string, params map[string]string) string {
		data, ok := cqPayloadValue(params["data"])
		if !ok {
			return match // 载荷形态既非 base64:// 也非 {...}：整码字面透传（现状）
		}
		if strings.HasPrefix(params["data"], "base64://") {
			foundItems["markdown"] = append(foundItems["markdown"], data)
			return ""
		}
		foundItems["markdown"] = append(foundItems["markdown"], base64.StdEncoding.EncodeToString([]byte(data)))
		return ""
	})
}

// processCQCardPipeline [CQ:card,...]：整码匹配 + 按键取值，参数顺序无关，JSON 编码存入。
// 修 F1/F2：复用 cqparse.ScanCode 状态机（段名整词匹配 + 参数区花括号配平），
// 因此 `[CQ:card,title=A,name=x]` 尾键不再被吞、`[CQ:cardboard,...]` /
// `[CQ:CARDIGAN,...]` 不再被 card 前缀误吞（旧 `\[CQ:card[^\]]*\]` 无边界口径废弃）。
// 空白与大小写容忍（统一契约 §1~§4）：段名与键名大小写不敏感、段名前（`[CQ:` 后）
// 与段名后、键值两侧可带空白；键 trim 后统一小写入 JSON（title/description/url/pic_url
// 四字段本为小写规范名），值仅 trim 首尾、内部空白保留（如 title 值 "my title"）。
// 裸 [CQ:card] 无参数时码移除、无产物（与旧口径一致）。
func processCQCardPipeline(text string, foundItems map[string][]string) string {
	return cqApplyNamedCodes(text, "card", false, func(_ string, params map[string]string) string {
		if len(params) > 0 {
			encoded, err := json.Marshal(params)
			if err == nil {
				foundItems["card"] = append(foundItems["card"], string(encoded))
			}
		}
		return ""
	})
}

// processCQInputNotifyPipeline [CQ:input_notify,...]：整码匹配后按键取值
// （统一契约 §1~§4：段名与键名大小写不敏感、任意键序、段名前后可带空白）。
// 值的既有字符面不变：type 必须为纯数字，second 可选且必须为纯数字；输出 JSON
// 仍为规范小写键 {"type":...,"second":...}。
func processCQInputNotifyPipeline(text string, foundItems map[string][]string) string {
	return inputNotifyPattern.ReplaceAllStringFunc(text, func(match string) string {
		params := cqUnionParams(stripCQCodeShell(match), false)
		typ, okType := params["type"]
		if !okType || !cqDigitsRe.MatchString(typ) {
			return match // type 缺失/非数字：与今日正则口径一致保留原文
		}
		notifyData := map[string]string{"type": typ}
		if second, okSecond := params["second"]; okSecond {
			if !cqDigitsRe.MatchString(second) {
				return match // second 存在但非数字：今日整码不匹配，原样保留
			}
			notifyData["second"] = second
		}
		encoded, err := json.Marshal(notifyData)
		if err == nil {
			foundItems["input_notify"] = append(foundItems["input_notify"], string(encoded))
		}
		return ""
	})
}

// stripCQCodeShell 剥去整码匹配的 "[CQ:" 前缀与 "]" 后缀，返回参数区原文
// （段名前空白已含在内，由 cqUnionParams 按键 trim 消化）。
func stripCQCodeShell(match string) string {
	comma := strings.IndexByte(match, ',')
	if comma < 0 {
		return "" // 裸码无参数区
	}
	return match[comma+1 : len(match)-1]
}

// processCQStreamPipeline [CQ:stream,...]：整码匹配后按键取值，
// 冒号（type:start,qq:1）与等号（type=start,qq=1）双语法均识别、可混用、
// 任意键序、段名与键名大小写不敏感（统一契约 §1~§4，对齐 cqparse splitKV）。
// 值的既有字符面不变：type 为 \w+、qq 为纯数字，二者齐备才消费。
func processCQStreamPipeline(text string, foundItems map[string][]string) string {
	return streamPattern.ReplaceAllStringFunc(text, func(match string) string {
		params := cqUnionParams(stripCQCodeShell(match), true)
		typ := params["type"]
		qq := params["qq"]
		if typ == "" || !cqStreamTypeRe.MatchString(typ) || !cqDigitsRe.MatchString(qq) {
			return match // 与今日一致：type/qq 任一不满足字符面则整段保留
		}
		streamData := map[string]string{
			"type": typ,
			"qq":   qq,
		}
		encoded, err := json.Marshal(streamData)
		if err == nil {
			foundItems["stream"] = append(foundItems["stream"], string(encoded))
		}
		return ""
	})
}

// processCQQQMusicPipeline [CQ:music,type=qq,id=...]：整码匹配后按键取值，
// type/id 任意键序、段名与键名大小写不敏感（统一契约 §1~§4）。
// 值的既有字符面不变：type 取值仍为字面 qq、id 必须纯数字。
func processCQQQMusicPipeline(text string, foundItems map[string][]string) string {
	return qqMusicPattern.ReplaceAllStringFunc(text, func(match string) string {
		params := cqUnionParams(stripCQCodeShell(match), false)
		if params["type"] == "qq" && cqDigitsRe.MatchString(params["id"]) {
			foundItems["qqmusic"] = append(foundItems["qqmusic"], params["id"])
			return ""
		}
		return match
	})
}

// processCQReplyPipeline [CQ:reply,id=数字] → reply_msg_id（构建 message_reference）。
// 整码匹配后按键取值：id 任意参数位置、段名与键名大小写不敏感（统一契约 §1~§4）；
// id 缺失或非数字与今日一致保留原文（[CQ:reply,msg_idx=...] 由入站专用剥离步处理）。
func processCQReplyPipeline(text string, foundItems map[string][]string) string {
	return replyRe.ReplaceAllStringFunc(text, func(match string) string {
		params := cqUnionParams(stripCQCodeShell(match), false)
		id := params["id"]
		if id == "" || !cqDigitsRe.MatchString(id) {
			return match
		}
		foundItems["reply_msg_id"] = append(foundItems["reply_msg_id"], id)
		return ""
	})
}

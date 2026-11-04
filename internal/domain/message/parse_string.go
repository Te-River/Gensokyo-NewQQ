package message

import "strings"

// cqPrefixLiteral CQ 码字面前缀（`[CQ:` 区分大小写，统一契约边界）。
const cqPrefixLiteral = "[CQ:"

// cqSpaceChars ASCII 五元集 {空格, \t, \n, \f, \r}，与 legacy 正则 `\s`、Rust
// is_cq_space 及 handlers/cqparse 的 cqSpaceBytes 同口径（G-A）；全角空格与 `\v` 不算。
const cqSpaceChars = " \t\n\f\r"

// ParseOneBotString 解析 OneBot 字符串消息（含 CQ 码）为类型化消息。
// 纯函数：不产生任何 IO。
//
// 词法识别（修 F3/F7，对齐 Rust cqcode.rs find_code_end 状态机与 handlers/cqparse
// scanSegments 同口径）：
//   - 段名前（`[CQ:` 后）与段名之后、`,` 或 `]` 之前允许 ASCII 五元集空白；
//   - 段名为 [A-Za-z0-9_]+ 整词——含非法字符（全角空格、`\v` 等）时段名不成立；
//   - 参数区做花括号配平感知（JSON 值内的 `]`/`,` 不结束码/段）；
//   - 段名非法、空段名、无闭合 `]`、配平失败 → 该位置按文本透传，游标只前进一个
//     字节重扫（不吞后方合法码，与 cqparse.Tokenize 失败分支一致）。
func ParseOneBotString(s string) (ParsedMessage, error) {
	var parts []MessagePart
	var reply *ReplyPart
	var text strings.Builder

	i := 0
	for i < len(s) {
		start := strings.Index(s[i:], cqPrefixLiteral)
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

		cqType, paramsPart, end, ok := scanCQCode(s, start)
		if !ok {
			// 段名非法/未闭合/配平失败：跳过一个字符继续扫描，原文按文本透传（F3/F7）
			text.WriteByte(s[start])
			i = start + 1
			continue
		}

		// §3 stream 双语法：仅 stream 段在无 `=` 时兼容首个 `:` 切分
		// （对齐 Rust scan_params 的 allow_colon 与 Go cqparse splitKV 的 action=="stream" 判定）
		part := cqTypeToPart(cqType, parseCQParamsMode(paramsPart, strings.EqualFold(cqType, "stream")))
		if rp, ok := part.(ReplyPart); ok {
			reply = &rp
		}
		parts = append(parts, part)
		i = end
	}

	if text.Len() > 0 {
		parts = append(parts, TextPart{Text: text.String()})
	}
	return ParsedMessage{Parts: parts, Reply: reply}, nil
}

// isCQNameByte 段名字符集 [A-Za-z0-9_]（对齐 Rust find_code_end：字母/数字/下划线都收）。
func isCQNameByte(c byte) bool {
	return c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z' || c >= '0' && c <= '9' || c == '_'
}

// scanCQCode 从 s[start]（"[CQ:" 字面起点）扫描一个完整 CQ 码，返回段名原文
// （前后空白已剥离，内部保持原样）、参数区原文（首个 `,` 之后、收尾 `]` 之前，
// 未做实体解码）与 `]` 之后的偏移。
// 失败（ok=false）的场景：段名为空或含非法字符（F7）、段名去空白后不紧跟 `,` 或 `]`
// （整词匹配，修 F2 前缀吞段）、无闭合 `]`、参数区花括号配平失败（修 C1/F3）。
func scanCQCode(s string, start int) (cqType, paramsPart string, end int, ok bool) {
	p := start + len(cqPrefixLiteral)
	// 段名前空白（`[CQ:` 之后）容忍（统一契约 §4）
	for p < len(s) && strings.IndexByte(cqSpaceChars, s[p]) >= 0 {
		p++
	}
	nameStart := p
	for p < len(s) && isCQNameByte(s[p]) {
		p++
	}
	if p == nameStart {
		return "", "", 0, false // 空段名或首字符即非法
	}
	cqType = s[nameStart:p]
	// 段名与 `,` 或 `]` 之间允许任意空白（统一契约 §1）
	for p < len(s) && strings.IndexByte(cqSpaceChars, s[p]) >= 0 {
		p++
	}
	if p >= len(s) {
		return "", "", 0, false
	}
	switch s[p] {
	case ']':
		return cqType, "", p + 1, true // 裸码无参数区
	case ',':
		closeIdx, scanned := scanCQParamsEnd(s, p+1)
		if !scanned {
			return "", "", 0, false
		}
		return cqType, s[p+1 : closeIdx], closeIdx + 1, true
	default:
		// 段名整词之后紧跟非法字符（`\v`、全角空格等）→ 整码按文本透传（F7）
		return "", "", 0, false
	}
}

// scanCQParamsEnd 从参数区起点花括号配平地扫描到配对的 `]`，返回其下标。
// 状态机字段与 handlers/cqparse scanSegments 及 Rust scan_regions 一一对应：
// 值以 `{` 起（仅段首/`=` 后位置）进入 JSON 区，区内跟踪字符串引号与转义，
// `,`/`]` 不切段不结束；配平失败或到串尾仍开区间 → ok=false。
func scanCQParamsEnd(s string, from int) (close int, ok bool) {
	depth := 0
	inStr := false
	esc := false
	segHasEq := false    // 当前段是否已出现 =（其后为值区）
	atValueStart := true // 段首/等号后：下一个 { 视为 JSON 值起点
	for q := from; q < len(s); q++ {
		c := s[q]
		if depth > 0 {
			if inStr {
				switch {
				case esc:
					esc = false
				case c == '\\':
					esc = true
				case c == '"':
					inStr = false
				}
			} else {
				switch c {
				case '"':
					inStr = true
				case '{':
					depth++
				case '}':
					depth--
				}
			}
			continue
		}
		switch {
		case c == ']':
			return q, true
		case c == '{' && atValueStart:
			depth = 1
			inStr = false
			esc = false
			atValueStart = false
		case c == '=' && !segHasEq:
			segHasEq = true
			atValueStart = true
		case c == ',':
			segHasEq = false
			atValueStart = true
		case strings.IndexByte(cqSpaceChars, c) >= 0:
			// 空白不重置 JSON 值起点判定（`data = {...}` 与 `data={...}` 同口径，含 \f 五元集）
		default:
			atValueStart = false
		}
	}
	return 0, false
}

// cqTypeToPart 把 CQ 码类型与参数映射为消息段。
// 段名大小写不敏感（统一契约 §1）：分支用小写归一形，未知段名的
// UnknownPart.Type 保留原文（仅两侧空白已 trim）。
func cqTypeToPart(t string, params map[string]string) MessagePart {
	switch strings.ToLower(t) {
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
	case "stream":
		// §3 双语法收口：参数已按 allowColon 解析（type/qq 冒号、等号、混写同形），
		// 新链暂无 stream 段类型，按未知段保留原始参数不丢信息。
		return UnknownPart{Type: t, Data: params}
	default:
		return UnknownPart{Type: t, Data: params}
	}
}

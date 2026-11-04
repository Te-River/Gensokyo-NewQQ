package cqparse

import "strings"

// cqPrefix 仅识别大写 [CQ:（前缀字面区分大小写；`[CQ:` 之后段名大小写不敏感，
// 小写归一在 tryParseCode 完成——统一契约 §1）。
const cqPrefix = "[CQ:"

// cqSpaceBytes ASCII 五元集 {空格, \t, \n, \f, \r}，与 legacy 正则 `\s` 及 Rust
// is_cq_space 同口径（G-A）；全角空格与 `\v` 不算段名空白。
const cqSpaceBytes = " \t\n\f\r"

// Tokenize 将字符串消息扫描为 Token 流。
// 规则（架构设计 §4.3 + 统一最大兼容契约 §1~§4）：
//  1. 仅识别大写 [CQ:，`[CQ:` 之后允许任意空白再接段名；段名为 [A-Za-z0-9_]+（含
//     数字、首字符不限数字）并按小写归一（Image/IMAGE = image），其后、`,` 或 `]` 之前允许任意空白；
//  2. 参数区逐段扫 k=v，逗号切分；键与 `=`、`=` 与值两侧允许空白（buildParams 各自
//     trim 并把键统一小写——参数键大小写不敏感、按键取值、任意键序）；
//     无 = 的尾随段并入前一 key 的值（C3 修复）；
//  3. 段值以 { 开头时做花括号配平扫描（允许 JSON 内含 ]/,），修 C1；
//  4. 无闭合 ]、空 action、配平失败 → KindText（原样保留）。
func Tokenize(s string) Doc {
	doc := Doc{Source: s}
	var toks []Token
	textStart := 0
	i := 0
	for i < len(s) {
		j := strings.Index(s[i:], cqPrefix)
		if j < 0 {
			break
		}
		start := i + j
		tok, next, ok := tryParseCode(s, start)
		if !ok {
			// 非法码起点：跳过一个字符继续扫描，原文保持不动
			i = start + 1
			continue
		}
		if start > textStart {
			toks = append(toks, textToken(s[textStart:start]))
		}
		toks = append(toks, tok)
		i = next
		textStart = i
	}
	if textStart < len(s) {
		toks = append(toks, textToken(s[textStart:]))
	}
	doc.Tokens = toks
	return doc
}

func textToken(raw string) Token {
	return Token{Kind: KindText, Raw: raw}
}

// tryParseCode 尝试从 s[start:] 解析一个 [CQ:...] 码。
// 成功返回 Token 与码结束偏移；失败（ok=false）时该位置按文本处理。
// 词法步（段名识别 + 参数区切分）已上移到导出的 ScanCode，本函数只做 Token 装配。
func tryParseCode(s string, start int) (Token, int, bool) {
	action, segments, end, ok := ScanCode(s, start)
	if !ok {
		return Token{}, 0, false
	}
	return Token{
		Kind:   kindForAction(action),
		Action: action,
		Params: buildParams(action, segments),
		Raw:    s[start:end],
		Span:   Span{Start: start, End: end},
	}, end, true
}

// ScanCode 从 s[start]（须为 "[CQ:" 字面起点）扫描一个完整 CQ 码，返回小写归一的
// 段名、参数区各段原文与 `]` 之后的偏移；ok=false 时调用方按文本处理该位置。
//
// 判定口径与 Tokenize 完全一致（本函数即其词法步，供 legacy 正则管道复用同一状态机，
// 统一契约 §1~§4）：
//   - 段名前（`[CQ:` 后）与段名后、`,` 或 `]` 之前允许 ASCII 五元集空白；
//   - 段名为 [A-Za-z0-9_]+（含数字、首字符不限数字）并小写归一；段名后必须紧跟 `,` 或 `]`——即整段段名匹配，
//     不做前缀匹配（`[CQ:CARDIGAN,x=1]` 不会被 card 吞掉，修 F2）；
//   - 参数区由 scanSegments 花括号配平感知，配平失败/无闭合 `]` → ok=false；
//   - 返回的段原文**不做实体解码**（解码只在 buildParams 一侧），因此 legacy 的
//     cqUnionParams 取值口径（不解码）与新链的 buildParams 口径（解码）各取所需。
func ScanCode(s string, start int) (action string, segments []string, end int, ok bool) {
	p := start + len(cqPrefix)
	// 段名前（`[CQ:` 之后）允许任意空白（统一契约 §4；空白集为 ASCII 五元集）
	for p < len(s) && strings.IndexByte(cqSpaceBytes, s[p]) >= 0 {
		p++
	}
	aStart := p
	// 段名字符集 [A-Za-z0-9_]（含数字、首字符不限数字），与 internal 链
	// isCQNameByte 及 Rust find_code_end 并集口径一致（统一契约 §1）
	for p < len(s) && (s[p] >= 'a' && s[p] <= 'z' || s[p] >= 'A' && s[p] <= 'Z' || s[p] >= '0' && s[p] <= '9' || s[p] == '_') {
		p++
	}
	// 段名大小写不敏感：统一按小写归一（Image = image），Raw 保留原文
	action = strings.ToLower(s[aStart:p])
	if action == "" || p >= len(s) {
		return "", nil, 0, false
	}
	// 段名与 ',' 或 ']' 之间允许任意空白（统一契约 §1；键值两侧空白由 buildParams trim）
	// 空白集为 ASCII 五元集 {空格,\t,\n,\f,\r}，与 legacy 正则 `\s` 及 Rust is_cq_space 同口径（G-A）
	for p < len(s) && strings.IndexByte(cqSpaceBytes, s[p]) >= 0 {
		p++
	}
	if p >= len(s) {
		return "", nil, 0, false
	}
	switch {
	case s[p] == ']':
		return action, nil, p + 1, true
	case s[p] == ',':
		segs, e, scanned := scanSegments(s, p+1)
		if !scanned {
			return "", nil, 0, false
		}
		return action, segs, e, true
	default:
		return "", nil, 0, false
	}
}

// scanSegments 扫描参数区直到配对的 ']'。
// 返回各段原始子串（含花括号配平内的逗号不切分）与 ']' 之后的位置。
func scanSegments(s string, from int) (segments []string, end int, ok bool) {
	segStart := from
	q := from
	depth := 0
	inStr := false
	esc := false
	segHasEq := false    // 当前段是否已出现 =（其后为值区）
	atValueStart := true // 段首/等号后：下一个 { 视为 JSON 值起点
	for q < len(s) {
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
			q++
			continue
		}
		switch {
		case c == ']':
			segments = append(segments, s[segStart:q])
			return segments, q + 1, true
		case c == '{' && atValueStart:
			depth = 1
			inStr = false
			esc = false
			atValueStart = false
		case c == '=' && !segHasEq:
			segHasEq = true
			atValueStart = true
		case c == ',':
			segments = append(segments, s[segStart:q])
			segStart = q + 1
			segHasEq = false
			atValueStart = true
		case c == ' ' || c == '\t' || c == '\n' || c == '\f' || c == '\r':
			// 空白不重置 JSON 值起点判定：`data = {"k":1}` 与 `data={"k":1}` 同口径（统一契约 §2）
			// 空白集为 ASCII 五元集（含 \f），与 cqSpaceBytes / legacy `\s` / Rust is_cq_space 一致（修 F4）
		default:
			atValueStart = false
		}
		q++
	}
	return nil, 0, false // 无闭合 ]
}

// buildParams 将参数段切片解析为已解码的 key=value map。
//   - key/value 各自 TrimSpace 后解码实体（Q2 拍板：统一 TrimSpace）；
//   - key 统一转小写（统一契约 §1：参数键大小写不敏感，Image 段的 File= 与 file= 同键；
//     重复键后者覆盖；card 的全量参数序列化因此同为小写规范形）；
//   - 无 = 的尾随段并入前一 key 的值，以 , 连接（C3 修复：user_ids=1,2,3 三路径等价）；
//   - stream 兼容冒号键语法 type:start,qq:123（m2；与 = 语法可混用）。
func buildParams(action string, segments []string) map[string]string {
	params := make(map[string]string)
	lastKey := ""
	for _, seg := range segments {
		if k, v, ok := splitKV(action, seg); ok {
			key := strings.ToLower(strings.TrimSpace(k))
			if key == "" {
				continue
			}
			params[key] = DecodeValue(strings.TrimSpace(v))
			lastKey = key
			continue
		}
		t := strings.TrimSpace(seg)
		if lastKey != "" && t != "" {
			// 合并值同样解码实体，保证与前段解码语义一致
			params[lastKey] = params[lastKey] + "," + DecodeValue(t)
		}
	}
	return params
}

// splitKV 取段内首个 = 作为键值分隔；stream 在无 = 时兼容首个 : 。
func splitKV(action, seg string) (k, v string, ok bool) {
	if eq := strings.Index(seg, "="); eq >= 0 {
		return seg[:eq], seg[eq+1:], true
	}
	if action == "stream" {
		if colon := strings.Index(seg, ":"); colon >= 0 {
			return seg[:colon], seg[colon+1:], true
		}
	}
	return "", "", false
}

// kindForAction 按架构设计 §7 迁移映射表分配 Token 类别；
// at/face/未知码等一律 KindPassthrough（原样保留）。
func kindForAction(action string) Kind {
	switch action {
	case "image", "record", "voice", "video", "file":
		return KindMedia
	case "markdown", "keyboard", "card", "input_notify", "stream", "music", "avatar", "group_info":
		return KindContent
	case "ark":
		// 入站专用卡片码：出站无消费点，由 inboundOnlyHandler 纯剥离
		// （对齐 legacy ProcessCQInboundOnlyCodes）
		return KindContent
	case "reply", "active", "wakeup":
		return KindControl
	case "member", "remove", "set_group":
		return KindAction
	default:
		return KindPassthrough
	}
}

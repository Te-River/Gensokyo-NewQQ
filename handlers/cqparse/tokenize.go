package cqparse

import "strings"

// cqPrefix 仅识别大写 [CQ:（现状全部正则皆大写，小写维持字面泄漏——设计 §4.3）。
const cqPrefix = "[CQ:"

// Tokenize 将字符串消息扫描为 Token 流。
// 规则（架构设计 §4.3）：
//  1. 仅识别大写 [CQ:，action 为 [a-z_]+ 且后随 , 或 ]；
//  2. 参数区逐段扫 k=v，逗号切分；无 = 的尾随段并入前一 key 的值（C3 修复）；
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
func tryParseCode(s string, start int) (Token, int, bool) {
	p := start + len(cqPrefix)
	aStart := p
	for p < len(s) && (s[p] >= 'a' && s[p] <= 'z' || s[p] == '_') {
		p++
	}
	action := s[aStart:p]
	if action == "" || p >= len(s) {
		return Token{}, 0, false
	}
	var segments []string
	var end int
	switch {
	case s[p] == ']':
		end = p + 1
	case s[p] == ',':
		segs, e, ok := scanSegments(s, p+1)
		if !ok {
			return Token{}, 0, false
		}
		segments = segs
		end = e
	default:
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

// scanSegments 扫描参数区直到配对的 ']'。
// 返回各段原始子串（含花括号配平内的逗号不切分）与 ']' 之后的位置。
func scanSegments(s string, from int) (segments []string, end int, ok bool) {
	segStart := from
	q := from
	depth := 0
	inStr := false
	esc := false
	segHasEq := false      // 当前段是否已出现 =（其后为值区）
	atValueStart := true   // 段首/等号后：下一个 { 视为 JSON 值起点
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
		default:
			atValueStart = false
		}
		q++
	}
	return nil, 0, false // 无闭合 ]
}

// buildParams 将参数段切片解析为已解码的 key=value map。
// - key/value 各自 TrimSpace 后再解码实体（Q2 拍板：统一 TrimSpace）；
// - 无 = 的尾随段并入前一 key 的值，以 , 连接（C3 修复：user_ids=1,2,3 三路径等价）；
// - 重复 key 后者覆盖；
// - stream 兼容冒号键语法 type:start,qq:123（m2）。
func buildParams(action string, segments []string) map[string]string {
	params := make(map[string]string)
	lastKey := ""
	for _, seg := range segments {
		if k, v, ok := splitKV(action, seg); ok {
			key := strings.TrimSpace(k)
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
	case "reply", "active", "wakeup":
		return KindControl
	case "member", "remove", "set_group":
		return KindAction
	default:
		return KindPassthrough
	}
}

package message

import "strings"

// CQ 码格式：[CQ:type,key=value,key=value]
// 转义：& -> &amp;，[ -> &#91;，] -> &#93;，, -> &#44;

// EscapeCQ 对文本进行 CQ 转义（用于构造 CQ 码值）。
func EscapeCQ(s string) string {
	r := strings.NewReplacer("&", "&amp;", "[", "&#91;", "]", "&#93;", ",", "&#44;")
	return r.Replace(s)
}

// UnescapeCQ 反转义 CQ 码值。
func UnescapeCQ(s string) string {
	r := strings.NewReplacer("&#44;", ",", "&#91;", "[", "&#93;", "]", "&amp;", "&")
	return r.Replace(s)
}

// splitCQParams 按逗号拆分参数，感知 &#44; 转义（不拆分转义逗号）。
func splitCQParams(s string) []string {
	var out []string
	var cur strings.Builder
	i := 0
	for i < len(s) {
		if hasPrefix(s[i:], "&#44;") {
			cur.WriteString("&#44;")
			i += 5
			continue
		}
		if s[i] == ',' {
			out = append(out, cur.String())
			cur.Reset()
			i++
			continue
		}
		cur.WriteByte(s[i])
		i++
	}
	out = append(out, cur.String())
	return out
}

// parseCQParams 解析 CQ 码参数为 key->value。
// 统一契约 §1~§3：键与 `=`、`=` 与值两侧允许空白；键 trim、值仅 trim 首尾空白，
// 内部空白保留（如 file=my file.txt）；键名大小写不敏感——统一转小写
// （Image 段的 File= 与 file= 同键，重复键后者覆盖）。
func parseCQParams(s string) map[string]string {
	return parseCQParamsMode(s, false)
}

// parseCQParamsMode 与 parseCQParams 同口径，allowColon=true 时段内无 `=`
// 再兼容首个 `:` 切分（§3 stream 冒号语法，两种语法可混用；对齐 Rust
// scan_params 的 allow_colon 与 Go cqparse splitKV 的 action=="stream" 判定）。
// 无 `=` 无 `:` 的裸尾段不并入前键——internal 现状口径不变。
func parseCQParamsMode(s string, allowColon bool) map[string]string {
	m := map[string]string{}
	if s == "" {
		return m
	}
	for _, kv := range splitCQParams(s) {
		eq := strings.IndexByte(kv, '=')
		if eq < 0 && allowColon {
			eq = strings.IndexByte(kv, ':')
		}
		if eq < 0 {
			continue
		}
		key := strings.ToLower(strings.TrimSpace(kv[:eq]))
		if key == "" {
			continue
		}
		m[key] = UnescapeCQ(strings.TrimSpace(kv[eq+1:]))
	}
	return m
}

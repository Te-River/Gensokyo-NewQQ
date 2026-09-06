package cqparse

import "strings"

// CQ 码转义编解码的唯一实现（架构设计 §4.2 规则表）。
//
// 编码（EncodeValue）：先 & → &amp;，再 , → &#44;、] → &#93;、[ → &#91;
// 解码（DecodeValue）：严格逆序 &#93; → &#44; → &#91; → &amp;（最后）。
// 顺序正确性：编码先转义 &，字面 "&#44;" 会变成 "&amp;#44;"，
// 解码前三步不会误伤它，&amp; 最后才还原为 & 本身。

// EncodeValue 将值中的 CQ 特殊字符转义为实体，用于自产 CQ 码参数。
func EncodeValue(s string) string {
	s = strings.ReplaceAll(s, "&", "&amp;")
	s = strings.ReplaceAll(s, ",", "&#44;")
	s = strings.ReplaceAll(s, "]", "&#93;")
	s = strings.ReplaceAll(s, "[", "&#91;")
	return s
}

// DecodeValue 将值中的 CQ 实体还原为原字符，编码的严格逆序。
func DecodeValue(s string) string {
	s = strings.ReplaceAll(s, "&#93;", "]")
	s = strings.ReplaceAll(s, "&#44;", ",")
	s = strings.ReplaceAll(s, "&#91;", "[")
	s = strings.ReplaceAll(s, "&amp;", "&")
	return s
}

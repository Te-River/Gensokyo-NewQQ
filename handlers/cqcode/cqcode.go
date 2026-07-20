// Package cqcode 提供 CQ 码解析与生成功能
package cqcode

// CQ 码类型常量
const (
	TypeAT       = "at"
	TypeImage    = "image"
	TypeReply    = "reply"
	TypeFile     = "file"
	TypeVideo    = "video"
	TypeMusic    = "music"
	TypeVoice    = "voice"
	TypeMarkdown = "markdown"
	TypeEmbed    = "embed"
	TypeActive   = "active"
	TypeKeyboard = "keyboard"
	TypeAvatar   = "avatar"
)

// CQCode 表示一个解析后的 CQ 码
type CQCode struct {
	Type string            `json:"type"`
	Data map[string]string `json:"data"`
}

// Parse 解析 CQ 码字符串
func Parse(input string) []CQCode {
	if input == "" {
		return nil
	}
	return parseRegex(input)
}

// String 将 CQ 码序列化为字符串
func (c CQCode) String() string {
	s := "[CQ:" + c.Type
	for k, v := range c.Data {
		s += "," + k + "=" + v
	}
	s += "]"
	return s
}

// parseRegex 使用正则表达式解析 CQ 码
func parseRegex(input string) []CQCode {
	// 实际解析逻辑在 handlers/message_parser.go 中
	// 此处定义接口，逐步迁移
	return nil
}
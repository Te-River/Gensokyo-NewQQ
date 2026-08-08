// Package message 提供 OneBot 消息的类型化模型与纯函数解析器。
//
// 目标：把散落在 handlers/message_parser.go 的 foundItems map 解析逻辑收敛为
// typed `ParsedMessage`，String 与 Array 两种入站形态输出同一模型。
//
// 本包为纯函数：禁止 HTTP / QQ API / idmap / config / goroutine / 上传等副作用，
// 一切 IO 由调用方在解析后处理。
package message

// MessagePart 是消息中的一个组成部分。
type MessagePart interface {
	messagePart()
}

// TextPart 纯文本。
type TextPart struct {
	Text string
}

// MentionPart @某人（qq 字段，可能是虚拟 ID）。
type MentionPart struct {
	User string
}

// ImagePart 图片。
type ImagePart struct {
	Source MediaSource
	File   string // CQ 原始 file 字段
}

// AudioPart 语音。
type AudioPart struct {
	Source MediaSource
	File   string
}

// VideoPart 视频。
type VideoPart struct {
	Source MediaSource
	File   string
}

// FilePart 文件。
type FilePart struct {
	Source   MediaSource
	Filename string
	File     string
}

// ReplyPart 回复引用。
type ReplyPart struct {
	MessageID string
}

// MarkdownPart Markdown 消息（data 为 base64 编码的 JSON 或明文 JSON）。
type MarkdownPart struct {
	Content string
}

// KeyboardPart 键盘（按钮）消息。
type KeyboardPart struct {
	Content string // keyboard JSON 原文
}

// QQMusicPart QQ 音乐分享。
type QQMusicPart struct {
	ID string
}

// UnknownPart 未识别/暂不支持的消息段，保留原始参数防止信息丢失。
type UnknownPart struct {
	Type string
	Data map[string]string
}

func (TextPart) messagePart()      {}
func (MentionPart) messagePart()   {}
func (ImagePart) messagePart()     {}
func (AudioPart) messagePart()     {}
func (VideoPart) messagePart()     {}
func (FilePart) messagePart()      {}
func (ReplyPart) messagePart()     {}
func (MarkdownPart) messagePart()  {}
func (KeyboardPart) messagePart()  {}
func (QQMusicPart) messagePart()   {}
func (UnknownPart) messagePart()   {}

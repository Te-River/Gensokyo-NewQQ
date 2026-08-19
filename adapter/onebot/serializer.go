// Package onebot 提供 DomainEvent → OneBot 消息格式的序列化与发布。
// 两个 serializer 只负责表示方式，不含业务判断（P8.5）。
package onebot

import (
	"strings"

	"github.com/hoshinonyaruko/gensokyo/internal/domain/message"
)

// SerializeString 把 ParsedMessage 序列化为 OneBot 字符串消息。
func SerializeString(pm message.ParsedMessage) string {
	var b strings.Builder
	for _, seg := range message.Canonicalize(pm) {
		b.WriteString(segmentToString(seg))
	}
	return b.String()
}

// SerializeArray 把 ParsedMessage 序列化为 OneBot 消息段数组。
func SerializeArray(pm message.ParsedMessage) []map[string]interface{} {
	segs := message.Canonicalize(pm)
	out := make([]map[string]interface{}, 0, len(segs))
	for _, seg := range segs {
		out = append(out, segmentToMap(seg))
	}
	return out
}

func segmentToString(seg message.Segment) string {
	if seg.Type == "text" {
		return seg.Data["text"]
	}
	var b strings.Builder
	b.WriteString("[CQ:" + seg.Type)
	for k, v := range seg.Data {
		b.WriteString("," + k + "=" + message.EscapeCQ(v))
	}
	b.WriteString("]")
	return b.String()
}

func segmentToMap(seg message.Segment) map[string]interface{} {
	data := make(map[string]interface{}, len(seg.Data))
	for k, v := range seg.Data {
		data[k] = v
	}
	return map[string]interface{}{"type": seg.Type, "data": data}
}

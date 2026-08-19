package message

import "fmt"

// Segment 是 OneBot 消息段（array 形式）的纯数据表示。
type Segment struct {
	Type string
	Data map[string]string
}

// FromMap 从 map 构造消息段。
func FromMap(m map[string]interface{}) (Segment, bool) {
	typ, ok := m["type"].(string)
	if !ok {
		return Segment{}, false
	}
	seg := Segment{Type: typ, Data: map[string]string{}}
	if raw, ok := m["data"].(map[string]interface{}); ok {
		for k, v := range raw {
			seg.Data[k] = stringify(v)
		}
	}
	return seg, true
}

func stringify(v interface{}) string {
	switch s := v.(type) {
	case string:
		return s
	case nil:
		return ""
	default:
		return fmt.Sprint(v)
	}
}

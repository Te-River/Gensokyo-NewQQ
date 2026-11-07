package handlers

import (
	"testing"

	"github.com/hoshinonyaruko/gensokyo/callapi"
)

// TestParseMessageContentLegacyMissingDataNoPanic 回归：段缺 data 或 data 非对象时，
// 旧实现在链式断言 `segmentMap["data"].(map[string]interface{})["text"]` 处 panic。
// 现在应跳过该段：不 panic、不产生文本、不产生 foundItems。
func TestParseMessageContentLegacyMissingDataNoPanic(t *testing.T) {
	tests := []struct {
		name    string
		message interface{}
	}{
		{
			name: "段数组-各类段缺 data",
			message: []interface{}{
				map[string]interface{}{"type": "text"},
				map[string]interface{}{"type": "image"},
				map[string]interface{}{"type": "voice"},
				map[string]interface{}{"type": "at"},
				map[string]interface{}{"type": "reply"},
				map[string]interface{}{"type": "video"},
				map[string]interface{}{"type": "music"},
			},
		},
		{
			name: "段数组-data 类型错误",
			message: []interface{}{
				map[string]interface{}{"type": "text", "data": "not-a-map"},
				map[string]interface{}{"type": "reply", "data": 42},
			},
		},
		{
			name:    "单段-map 缺 data",
			message: map[string]interface{}{"type": "text"},
		},
		{
			name:    "单段-map data 类型错误",
			message: map[string]interface{}{"type": "image", "data": []interface{}{}},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			params := callapi.ParamsContent{
				GroupID: "g-test-openid",
				UserID:  "u-test-openid",
				Message: tt.message,
			}
			text, items := parseMessageContentLegacy(params, callapi.ActionMessage{Action: "send_group_msg", Params: params}, nil, nil, nil)
			if text != "" {
				t.Fatalf("缺 data 的段不应产生文本, got %q", text)
			}
			if len(items) != 0 {
				t.Fatalf("缺 data 的段不应产生 foundItems, got %v", items)
			}
		})
	}
}

// TestParseMessageContentLegacyMissingDataKeepsNormalSegments 异常段被跳过后，
// 相邻正常段仍按原逻辑解析（成功路径零行为变化）。
func TestParseMessageContentLegacyMissingDataKeepsNormalSegments(t *testing.T) {
	params := callapi.ParamsContent{
		GroupID: "g-test-openid",
		UserID:  "u-test-openid",
		Message: []interface{}{
			seg("text", map[string]interface{}{"text": "hello"}),
			map[string]interface{}{"type": "image"},
			seg("reply", map[string]interface{}{"id": "123"}),
		},
	}
	text, items := parseMessageContentLegacy(params, callapi.ActionMessage{Action: "send_group_msg", Params: params}, nil, nil, nil)
	if text != "hello" {
		t.Fatalf("正常文本段解析结果变了, got %q want %q", text, "hello")
	}
	if got := items["reply_msg_id"]; len(got) != 1 || got[0] != "123" {
		t.Fatalf("正常 reply 段解析结果变了, got %v", got)
	}
}

package handlers

import (
	"encoding/base64"
	"encoding/json"
	"strings"
	"testing"
)

// TestProcessCQCodePipeline 统一管道聚焦测试：覆盖所有 CQ 码类型的解析与文本剔除
func TestProcessCQCodePipeline(t *testing.T) {
	tests := []struct {
		name          string
		input         string
		groupID       interface{}
		wantText      string
		wantKeys      map[string]int // key → 期望 foundItems[key] 长度
		wantNotKeys   []string       // 不应出现的 key
	}{
		{
			name:     "纯文本无CQ码",
			input:    "你好世界",
			wantText: "你好世界",
		},
		{
			name:     "URL图片提取",
			input:    "看这张图[CQ:image,file=https://example.com/pic.jpg]好看吗",
			wantText: "看这张图好看吗",
			wantKeys: map[string]int{"url_images": 1},
		},
		{
			name:     "base64图片提取",
			input:    "[CQ:image,file=base64://aGVsbG8=]",
			wantText: "",
			wantKeys: map[string]int{"base64_image": 1},
		},
		{
			name:     "HTTP语音提取",
			input:    "[CQ:record,file=http://example.com/voice.silk]",
			wantText: "",
			wantKeys: map[string]int{"url_record": 1},
		},
		{
			name:     "HTTPS视频提取",
			input:    "[CQ:video,file=https://example.com/video.mp4]",
			wantText: "",
			wantKeys: map[string]int{"url_videos": 1},
		},
		{
			name:     "base64语音提取",
			input:    "[CQ:record,file=base64://dGVzdA==]",
			wantText: "",
			wantKeys: map[string]int{"base64_record": 1},
		},
		{
			name:     "markdown base64提取",
			input:    "[CQ:markdown,data=base64://eyJjb250ZW50IjoiI35Z5L2g5aW9In0=]",
			wantText: "",
			wantKeys: map[string]int{"markdown": 1},
		},
		{
			name:     "card参数提取",
			input:    "[CQ:card,title=测试,desc=描述,pic=https://x.com/p.png,url=https://x.com]",
			wantText: "",
			wantKeys: map[string]int{"card": 1},
		},
		{
			name:     "input_notify提取",
			input:    "[CQ:input_notify,type=1,second=5]",
			wantText: "",
			wantKeys: map[string]int{"input_notify": 1},
		},
		{
			name:     "stream提取",
			input:    "[CQ:stream,type:start,qq:12345]",
			wantText: "",
			wantKeys: map[string]int{"stream": 1},
		},
		{
			name:     "QQ音乐提取",
			input:    "[CQ:music,type=qq,id=123456]",
			wantText: "",
			wantKeys: map[string]int{"qqmusic": 1},
		},
		{
			name:     "active标记",
			input:    "[CQ:active,type=push,sub_type=1]主动消息",
			wantText: "主动消息",
			wantKeys: map[string]int{"active": 1, "active_type": 1, "active_sub_type": 1},
		},
		{
			name:     "wakeup标记",
			input:    "[CQ:wakeup,userid=123456789]召回消息",
			wantText: "召回消息",
			wantKeys: map[string]int{"wakeup": 1},
		},
		{
			name:     "wakeup带OpenID",
			input:    "唤醒[CQ:wakeup,userid=0123456789abcdef0123456789abcdef]",
			wantText: "唤醒",
			wantKeys: map[string]int{"wakeup": 1},
		},
		{
			name:     "reply提取",
			input:    "[CQ:reply,id=829]这是回复",
			wantText: "这是回复",
			wantKeys: map[string]int{"reply_msg_id": 1},
		},
		{
			name:     "keyboard base64提取",
			input:    "[CQ:keyboard,data=base64://eyJjb250ZW50Ijp7InJvd3MiOltdfX0=]文本",
			wantText: "文本",
			wantKeys: map[string]int{"keyboard": 1},
		},
		{
			name:     "多种CQ码混合",
			input:    "[CQ:reply,id=100][CQ:image,file=https://x.com/a.png]你好[CQ:at,qq=12345]",
			wantText: "你好[CQ:at,qq=12345]",
			wantKeys: map[string]int{"reply_msg_id": 1, "url_images": 1},
		},
		{
			name:     "at码原样保留",
			input:    "[CQ:at,qq=12345]你好",
			wantText: "[CQ:at,qq=12345]你好",
		},
		{
			name:     "空文本",
			input:    "",
			wantText: "",
		},
		{
			name:     "多个同类CQ码",
			input:    "[CQ:image,file=https://x.com/a.png][CQ:image,file=https://x.com/b.png]",
			wantText: "",
			wantKeys: map[string]int{"url_images": 2},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			foundItems := make(map[string][]string)
			got := ProcessCQCodePipeline(tt.input, foundItems, tt.groupID)

			if got != tt.wantText {
				t.Errorf("文本不匹配:\n  got:  %q\n  want: %q", got, tt.wantText)
			}

			for key, wantLen := range tt.wantKeys {
				if items, ok := foundItems[key]; !ok {
					t.Errorf("缺少期望 key: %s", key)
				} else if len(items) != wantLen {
					t.Errorf("key %s 长度: got %d, want %d, items=%v", key, len(items), wantLen, items)
				}
			}

			for _, key := range tt.wantNotKeys {
				if _, ok := foundItems[key]; ok {
					t.Errorf("不应出现 key: %s", key)
				}
			}
		})
	}
}

// TestProcessCQCodePipelineKeyboardJSON 验证 keyboard 的 JSON 形态解析
func TestProcessCQCodePipelineKeyboardJSON(t *testing.T) {
	kbJSON := `{"content":{"rows":[{"buttons":[{"id":"b1","render_data":{"label":"测试","style":1},"action":{"type":2,"data":"/test","permission":{"type":2}}}]}]}}`
	b64 := base64.StdEncoding.EncodeToString([]byte(kbJSON))
	input := "[CQ:keyboard,data=base64://" + b64 + "]你好"

	foundItems := make(map[string][]string)
	got := ProcessCQCodePipeline(input, foundItems, nil)

	if got != "你好" {
		t.Errorf("文本剔除失败: %q", got)
	}
	items, ok := foundItems["keyboard"]
	if !ok || len(items) == 0 {
		t.Fatal("keyboard 未解析")
	}

	// 验证解析出的 JSON 可被 parseKeyboardData 正确解析
	kb, err := parseKeyboardData([]byte(items[0]))
	if err != nil {
		t.Fatalf("parseKeyboardData 失败: %v", err)
	}
	if kb == nil || kb.Content == nil {
		t.Fatal("keyboard 结构为空")
	}
	if len(kb.Content.Rows) != 1 {
		t.Fatalf("rows 数量: got %d, want 1", len(kb.Content.Rows))
	}
}

// TestProcessCQCodePipelineCardJSON 验证 card 参数解析为合法 JSON
func TestProcessCQCodePipelineCardJSON(t *testing.T) {
	input := "[CQ:card,title=标题,desc=描述,pic=https://x.com/p.png,url=https://x.com]"
	foundItems := make(map[string][]string)
	got := ProcessCQCodePipeline(input, foundItems, nil)

	if got != "" {
		t.Errorf("文本未清空: %q", got)
	}
	items, ok := foundItems["card"]
	if !ok || len(items) == 0 {
		t.Fatal("card 未解析")
	}

	var cardData map[string]string
	if err := json.Unmarshal([]byte(items[0]), &cardData); err != nil {
		t.Fatalf("card JSON 解析失败: %v", err)
	}
	if cardData["title"] != "标题" {
		t.Errorf("title: got %q, want %q", cardData["title"], "标题")
	}
	if cardData["url"] != "https://x.com" {
		t.Errorf("url: got %q, want %q", cardData["url"], "https://x.com")
	}
}

// TestProcessCQCodePipelineStreamJSON 验证 stream 参数解析
func TestProcessCQCodePipelineStreamJSON(t *testing.T) {
	input := "[CQ:stream,type:mid,qq:99999]"
	foundItems := make(map[string][]string)
	got := ProcessCQCodePipeline(input, foundItems, nil)

	if got != "" {
		t.Errorf("文本未清空: %q", got)
	}
	items, ok := foundItems["stream"]
	if !ok || len(items) == 0 {
		t.Fatal("stream 未解析")
	}

	var streamData map[string]string
	if err := json.Unmarshal([]byte(items[0]), &streamData); err != nil {
		t.Fatalf("stream JSON 解析失败: %v", err)
	}
	if streamData["type"] != "mid" {
		t.Errorf("type: got %q, want %q", streamData["type"], "mid")
	}
	if streamData["qq"] != "99999" {
		t.Errorf("qq: got %q, want %q", streamData["qq"], "99999")
	}
}

// TestProcessCQCodePipelineSlicePath 模拟消息段数组路径下的管道调用
// 验证 NoneBot 等框架发送的 segment_type_koishi 格式能正确解析
func TestProcessCQCodePipelineSlicePath(t *testing.T) {
	// 模拟 []interface{} 分支中 text 段合并后的 messageText
	segments := []interface{}{
		map[string]interface{}{
			"type": "text",
			"data": map[string]interface{}{
				"text": "[CQ:keyboard,data=base64://eyJjb250ZW50Ijp7InJvd3MiOltdfX0=]你好喵",
			},
		},
	}

	messageText := ""
	for _, seg := range segments {
		segMap := seg.(map[string]interface{})
		if segMap["type"] == "text" {
			messageText += segMap["data"].(map[string]interface{})["text"].(string)
		}
	}

	foundItems := make(map[string][]string)
	got := ProcessCQCodePipeline(messageText, foundItems, nil)

	if got != "你好喵" {
		t.Errorf("消息段路径文本不匹配: %q", got)
	}
	if _, ok := foundItems["keyboard"]; !ok {
		t.Error("消息段路径 keyboard 未解析")
	}
}

// TestProcessCQCodePipelineIdempotent 验证管道幂等性：多次调用结果一致
func TestProcessCQCodePipelineIdempotent(t *testing.T) {
	input := "[CQ:reply,id=1]你好[CQ:image,file=https://x.com/a.png]世界"

	foundItems1 := make(map[string][]string)
	got1 := ProcessCQCodePipeline(input, foundItems1, nil)

	foundItems2 := make(map[string][]string)
	got2 := ProcessCQCodePipeline(input, foundItems2, nil)

	if got1 != got2 {
		t.Errorf("幂等性失败: %q != %q", got1, got2)
	}
	for key := range foundItems1 {
		if len(foundItems1[key]) != len(foundItems2[key]) {
			t.Errorf("key %s 长度不一致", key)
		}
	}
}

// TestProcessCQCodePipelineNoSideEffectOnUnknown 验证未知 CQ 码不被误处理
func TestProcessCQCodePipelineNoSideEffectOnUnknown(t *testing.T) {
	input := "[CQ:unknown,data=test]保留原文"
	foundItems := make(map[string][]string)
	got := ProcessCQCodePipeline(input, foundItems, nil)

	if !strings.Contains(got, "[CQ:unknown,data=test]") {
		t.Errorf("未知 CQ 码被误处理: %q", got)
	}
}

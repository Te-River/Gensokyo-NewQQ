package handlers

import (
	"encoding/base64"
	"encoding/json"
	"strings"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/callapi"
)

// seg 构造 OneBot 消息段（segment_type_koishi 格式）
func seg(segType string, data map[string]interface{}) map[string]interface{} {
	return map[string]interface{}{"type": segType, "data": data}
}

// parseGroupContent 简化 parseMessageContent 调用：群消息路径固定 GroupID/UserID
// 传入 nil 客户端：未开启 transfer_url 时解析路径不会触达 client/api/apiv2
func parseGroupContent(t *testing.T, message interface{}) (string, map[string][]string) {
	t.Helper()
	params := callapi.ParamsContent{
		GroupID: "g-test-openid",
		UserID:  "u-test-openid",
		Message: message,
	}
	return parseMessageContent(params, callapi.ActionMessage{Action: "send_group_msg", Params: params}, nil, nil, nil)
}

// TestParseMessageContentFoundItems 聚焦测试：消息段数组路径的 foundItems 解析与文本拼接
func TestParseMessageContentFoundItems(t *testing.T) {
	tests := []struct {
		name       string
		message    interface{}
		wantText   string
		wantKeys   map[string]int            // key → 期望 foundItems[key] 长度
		wantValues map[string][]string       // key → 期望精确值
		wantNotKey func(string) bool         // 额外负面断言
	}{
		{
			name:     "纯文本字符串路径",
			message:  "你好世界",
			wantText: "你好世界",
		},
		{
			name: "text+image+at+reply 混合段",
			message: []interface{}{
				seg("text", map[string]interface{}{"text": "看这张图"}),
				seg("image", map[string]interface{}{"file": "base64://aGVsbG8="}),
				seg("at", map[string]interface{}{"qq": "12345"}),
				seg("reply", map[string]interface{}{"id": "829"}),
			},
			wantText:   "看这张图[CQ:at,qq=12345]",
			wantKeys:   map[string]int{"base64_image": 1, "reply_msg_id": 1},
			wantValues: map[string][]string{"base64_image": {"aGVsbG8="}, "reply_msg_id": {"829"}},
		},
		{
			name: "语音/视频/文件/音乐段",
			message: []interface{}{
				seg("voice", map[string]interface{}{"file": "https://example.com/voice.silk"}),
				seg("record", map[string]interface{}{"file": "http://example.com/a.silk"}),
				seg("video", map[string]interface{}{"file": "base64://dmlkZW8="}),
				seg("file", map[string]interface{}{"file": "https://example.com/a.zip", "file_name": "a.zip"}),
				seg("music", map[string]interface{}{"type": "qq", "id": "123456"}),
			},
			wantKeys:   map[string]int{"url_records": 1, "url_record": 1, "base64_video": 1, "url_files": 1, "file_name": 1, "qqmusic": 1},
			wantValues: map[string][]string{"url_records": {"example.com/voice.silk"}, "file_name": {"a.zip"}, "qqmusic": {"123456"}},
		},
		{
			name: "card/input_notify/stream/active 控制段",
			message: []interface{}{
				seg("card", map[string]interface{}{"title": "标题", "desc": "描述", "pic": "https://x.com/p.png", "url": "https://x.com"}),
				seg("input_notify", map[string]interface{}{"type": "1", "second": "5"}),
				seg("stream", map[string]interface{}{"type": "mid", "qq": "99999"}),
				seg("active", map[string]interface{}{"type": "push", "sub_type": "1"}),
			},
			wantText: "",
			wantKeys: map[string]int{"card": 1, "input_notify": 1, "stream": 1, "active_type": 1, "active_sub_type": 1},
		},
		{
			name: "markdown 段 data 为 map 时 base64 编码",
			message: []interface{}{
				seg("markdown", map[string]interface{}{"data": map[string]interface{}{"content": "hello"}}),
			},
			wantText: "",
			wantKeys: map[string]int{"markdown": 1},
		},
		{
			name: "未知图片类型落入 unknown_image",
			message: []interface{}{
				seg("image", map[string]interface{}{"file": "random_bytes"}),
			},
			wantText: "",
			wantKeys: map[string]int{"unknown_image": 1},
		},
		{
			name: "本地文件 file:// 恢复用例",
			message: []interface{}{
				seg("file", map[string]interface{}{"file": "file:///tmp/ok.txt"}),
			},
			wantText: "",
			wantKeys: map[string]int{"local_file": 1},
		},
		{
			name: "路径穿越被拒绝",
			message: []interface{}{
				seg("file", map[string]interface{}{"file": "file:///../secret.txt"}),
				seg("image", map[string]interface{}{"file": "file:///../evil.png"}),
			},
			wantText: "",
			wantNotKey: func(key string) bool {
				return key == "local_file" || key == "local_image"
			},
		},
		{
			name: "未知段类型不 panic 且不落 key",
			message: []interface{}{
				seg("sticker", map[string]interface{}{"id": "1"}),
				"非 map 段",
				123,
			},
			wantText: "",
			wantNotKey: func(key string) bool { return true },
		},
		{
			name: "markdown base64 非 JSON 被跳过",
			message: []interface{}{
				seg("markdown", map[string]interface{}{"data": base64.StdEncoding.EncodeToString([]byte("not-json"))}),
			},
			wantText: "",
			wantNotKey: func(key string) bool { return key == "markdown" },
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			gotText, gotItems := parseGroupContent(t, tt.message)

			if gotText != tt.wantText {
				t.Errorf("文本不匹配:\n  got:  %q\n  want: %q", gotText, tt.wantText)
			}

			for key, wantLen := range tt.wantKeys {
				if items, ok := gotItems[key]; !ok {
					t.Errorf("缺少期望 key: %s", key)
				} else if len(items) != wantLen {
					t.Errorf("key %s 长度: got %d, want %d, items=%v", key, len(items), wantLen, items)
				}
			}

			for key, wantValues := range tt.wantValues {
				items, ok := gotItems[key]
				if !ok {
					t.Errorf("缺少期望 key: %s", key)
					continue
				}
				if len(items) != len(wantValues) {
					t.Errorf("key %s 长度: got %d, want %d", key, len(items), len(wantValues))
					continue
				}
				for i, want := range wantValues {
					if items[i] != want {
						t.Errorf("key %s[%d]: got %q, want %q", key, i, items[i], want)
					}
				}
			}

			if tt.wantNotKey != nil {
				for key := range gotItems {
					if tt.wantNotKey(key) {
						t.Errorf("不应出现 key: %s", key)
					}
				}
			}
		})
	}
}

// TestParseMessageContentMarkdownBase64 验证 markdown 段解析结果为可回解的 base64 JSON
func TestParseMessageContentMarkdownBase64(t *testing.T) {
	message := []interface{}{
		seg("markdown", map[string]interface{}{"data": map[string]interface{}{"content": "hello", "template_id": 1}}),
	}

	_, gotItems := parseGroupContent(t, message)

	items, ok := gotItems["markdown"]
	if !ok || len(items) != 1 {
		t.Fatalf("markdown 未解析: %v", gotItems)
	}
	decoded, err := base64.StdEncoding.DecodeString(items[0])
	if err != nil {
		t.Fatalf("markdown 不是合法 base64: %v", err)
	}
	var mdMap map[string]interface{}
	if err := json.Unmarshal(decoded, &mdMap); err != nil {
		t.Fatalf("markdown base64 内容不是 JSON: %v", err)
	}
	if mdMap["content"] != "hello" {
		t.Errorf("markdown content: got %v, want hello", mdMap["content"])
	}
}

// TestParseMessageContentCardJSON 验证 card 段参数为合法 JSON 且可在消息文本中删除
func TestParseMessageContentCardJSON(t *testing.T) {
	message := []interface{}{
		seg("card", map[string]interface{}{"title": "标题", "url": "https://x.com"}),
	}

	gotText, gotItems := parseGroupContent(t, message)

	if gotText != "" {
		t.Errorf("card 段应在文本中留痕: %q", gotText)
	}
	items, ok := gotItems["card"]
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
}

// TestParseMessageContentStringPathCQCode 验证字符串路径下统一 CQ 码管道仍生效
func TestParseMessageContentStringPathCQCode(t *testing.T) {
	gotText, gotItems := parseGroupContent(t, "看这张图[CQ:image,file=https://example.com/pic.jpg]好看吗")

	if gotText != "看这张图好看吗" {
		t.Errorf("文本不匹配: %q", gotText)
	}
	if items, ok := gotItems["url_images"]; !ok || len(items) != 1 || items[0] != "example.com/pic.jpg" {
		t.Errorf("url_images 解析异常: %v", gotItems["url_images"])
	}
}

// TestParseMessageContentNoGroupID 验证 GroupID 为 nil（私聊路径）时不触发 idmap 查询
func TestParseMessageContentNoGroupID(t *testing.T) {
	params := callapi.ParamsContent{
		Message: []interface{}{
			seg("text", map[string]interface{}{"text": "私聊消息"}),
		},
	}

	gotText, gotItems := parseMessageContent(params, callapi.ActionMessage{Action: "send_private_msg", Params: params}, nil, nil, nil)

	if gotText != "私聊消息" {
		t.Errorf("文本不匹配: %q", gotText)
	}
	if len(gotItems) != 0 {
		t.Errorf("不应产生 foundItems: %v", gotItems)
	}
}

// TestParseMessageContentTextAtFallback 验证纯 at 消息的文本回退行为
func TestParseMessageContentTextAtFallback(t *testing.T) {
	message := []interface{}{
		seg("at", map[string]interface{}{"qq": "12345"}),
	}

	gotText, _ := parseGroupContent(t, message)

	// 纯 at 消息：transformMessageTextAt 回退保留原始 at 文本
	if !strings.Contains(gotText, "[CQ:at,qq=12345]") {
		t.Errorf("纯 at 消息未回退保留: %q", gotText)
	}
}

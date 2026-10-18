package handlers

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/tencent-connect/botgo/dto"
)

// findSeg 按段类型查找第一个匹配段
func findSeg(segs []map[string]interface{}, segType string) map[string]interface{} {
	for _, s := range segs {
		if s["type"] == segType {
			return s
		}
	}
	return nil
}

// segData 取出段的 data 字段
func segData(t *testing.T, seg map[string]interface{}) map[string]interface{} {
	t.Helper()
	if seg == nil {
		t.Fatal("段不存在")
	}
	data, ok := seg["data"].(map[string]interface{})
	if !ok {
		t.Fatalf("段 data 类型错误: %#v", seg["data"])
	}
	return data
}

// TestMessageEventFields_ArkPicture 结构化卡片（ark_type=picture）反序列化与 ark 段产出
func TestMessageEventFields_ArkPicture(t *testing.T) {
	raw := `{
		"id": "MSG_ARK",
		"group_openid": "GROUP_OPENID",
		"author": {"id": "USER_OPENID"},
		"content": "",
		"timestamp": "2026-09-16T12:00:00+08:00",
		"message_type": 3,
		"ark_data": {
			"prompt": "点击查看图片",
			"ark_type": "picture",
			"ark_name": "图片",
			"fields": {"title": "标题", "desc": "描述", "jump_url": "https://example.com/a"}
		}
	}`
	var data dto.WSGroupATMessageData
	if err := json.Unmarshal([]byte(raw), &data); err != nil {
		t.Fatalf("事件反序列化失败: %v", err)
	}
	if data.MessageType != 3 {
		t.Fatalf("message_type: got %d, want 3", data.MessageType)
	}
	if data.ArkData == nil || data.ArkData.ARKType != "picture" || data.ArkData.ARKName != "图片" {
		t.Fatalf("ark_data 反序列化错误: %+v", data.ArkData)
	}

	segs := ConvertToSegmentedMessage(&data)
	ark := findSeg(segs, "ark")
	if ark == nil {
		t.Fatalf("应产出 ark 段: %+v", segs)
	}
	arkData := segData(t, ark)
	if arkData["ark_type"] != "picture" {
		t.Errorf("ark_type: got %v, want picture", arkData["ark_type"])
	}
	if arkData["ark_name"] != "图片" {
		t.Errorf("ark_name: got %v, want 图片", arkData["ark_name"])
	}
	if arkData["prompt"] != "点击查看图片" {
		t.Errorf("prompt: got %v, want 点击查看图片", arkData["prompt"])
	}
	fields, ok := arkData["fields"].(map[string]interface{})
	if !ok || fields["title"] != "标题" {
		t.Errorf("fields 丢失: %#v", arkData["fields"])
	}
}

// TestMessageEventFields_VoiceAttachment 语音附件（voice_wav_url + asr_refer_text）产出 record 段
func TestMessageEventFields_VoiceAttachment(t *testing.T) {
	raw := `{
		"id": "MSG_VOICE",
		"group_openid": "GROUP_OPENID",
		"author": {"id": "USER_OPENID"},
		"content": "",
		"attachments": [{
			"url": "https://x.com/a.silk",
			"filename": "a.silk",
			"size": 123,
			"content_type": "voice",
			"voice_wav_url": "https://x.com/a.wav",
			"asr_refer_text": "你好"
		}]
	}`
	var data dto.WSGroupATMessageData
	if err := json.Unmarshal([]byte(raw), &data); err != nil {
		t.Fatalf("事件反序列化失败: %v", err)
	}
	if len(data.Attachments) != 1 || data.Attachments[0].VoiceWavURL != "https://x.com/a.wav" || data.Attachments[0].ASRReferText != "你好" {
		t.Fatalf("语音附件反序列化错误: %+v", data.Attachments)
	}

	segs := ConvertToSegmentedMessage(&data)
	record := findSeg(segs, "record")
	if record == nil {
		t.Fatalf("应产出 record 段: %+v", segs)
	}
	recordData := segData(t, record)
	if recordData["file"] != "https://x.com/a.wav" {
		t.Errorf("file 应优先取 voice_wav_url: got %v", recordData["file"])
	}
	if recordData["url"] != "https://x.com/a.silk" {
		t.Errorf("url: got %v, want https://x.com/a.silk", recordData["url"])
	}
	if recordData["asr_refer_text"] != "你好" {
		t.Errorf("asr_refer_text: got %v, want 你好", recordData["asr_refer_text"])
	}

	// voice_wav_url 缺失时回退附件 url，asr 为空时不产出该键
	fallback := &dto.WSGroupATMessageData{
		Attachments: []*dto.MessageAttachment{{URL: "https://x.com/b.silk", ContentType: "voice"}},
	}
	fallbackSegs := ConvertToSegmentedMessage(fallback)
	fallbackRecord := findSeg(fallbackSegs, "record")
	if fallbackRecord == nil {
		t.Fatalf("回退场景应产出 record 段: %+v", fallbackSegs)
	}
	fallbackData := segData(t, fallbackRecord)
	if fallbackData["file"] != "https://x.com/b.silk" {
		t.Errorf("file 回退: got %v", fallbackData["file"])
	}
	if _, ok := fallbackData["asr_refer_text"]; ok {
		t.Errorf("asr 为空时不应产出 asr_refer_text: %#v", fallbackData)
	}
}

// TestMessageEventFields_QuoteMessage 引用消息（message_type=103 + msg_elements）产出 reply/text 段
func TestMessageEventFields_QuoteMessage(t *testing.T) {
	raw := `{
		"id": "MSG_QUOTE",
		"group_openid": "GROUP_OPENID",
		"author": {"id": "USER_OPENID"},
		"content": "",
		"message_type": 103,
		"msg_elements": [{
			"msg_idx": "REFIDX_x",
			"author": {"id": "USER_OTHER"},
			"message_type": 0,
			"content": "hi"
		}]
	}`
	var data dto.WSGroupATMessageData
	if err := json.Unmarshal([]byte(raw), &data); err != nil {
		t.Fatalf("事件反序列化失败: %v", err)
	}
	if data.MessageType != 103 || len(data.MsgElements) != 1 || data.MsgElements[0].MsgIdx != "REFIDX_x" {
		t.Fatalf("msg_elements 反序列化错误: %+v", data.MsgElements)
	}

	segs := ConvertToSegmentedMessage(&data)
	reply := findSeg(segs, "reply")
	if reply == nil {
		t.Fatalf("应产出 reply 段: %+v", segs)
	}
	replyData := segData(t, reply)
	if replyData["msg_idx"] != "REFIDX_x" {
		t.Errorf("msg_idx: got %v, want REFIDX_x", replyData["msg_idx"])
	}
	if replyData["content"] != "hi" {
		t.Errorf("reply content: got %v, want hi", replyData["content"])
	}
	text := findSeg(segs, "text")
	if text == nil || segData(t, text)["text"] != "hi" {
		t.Errorf("嵌套正文不得丢失: %+v", segs)
	}
}

// TestMessageEventFields_NestedElements 递归降级：101/102 无具体段时保留正文
func TestMessageEventFields_NestedElements(t *testing.T) {
	raw := `{
		"id": "MSG_NESTED",
		"group_openid": "GROUP_OPENID",
		"author": {"id": "USER_OPENID"},
		"content": "",
		"message_type": 103,
		"msg_elements": [{
			"msg_idx": "REFIDX_a",
			"message_type": 102,
			"content": "聊天记录正文",
			"msg_elements": [{
				"msg_idx": "REFIDX_b",
				"message_type": 0,
				"content": "嵌套正文"
			}]
		}]
	}`
	var data dto.WSGroupATMessageData
	if err := json.Unmarshal([]byte(raw), &data); err != nil {
		t.Fatalf("事件反序列化失败: %v", err)
	}

	segs := ConvertToSegmentedMessage(&data)
	var texts []string
	for _, s := range segs {
		if s["type"] == "text" {
			texts = append(texts, segData(t, s)["text"].(string))
		}
	}
	joined := strings.Join(texts, "|")
	if !strings.Contains(joined, "聊天记录正文") || !strings.Contains(joined, "嵌套正文") {
		t.Errorf("嵌套正文不得静默丢弃: %+v", segs)
	}
}

// TestMessageEventFields_CQPrefixConflict 新 CQ 码前缀与出站解析的冲突核查
// 结论：msg_idx/ark_type 非出站正则的 id= 语义，不会污染 reply_msg_id；
// 语音码的 asr= 会被既有贪婪 URL 正则并入 URL（既有 regex 限制，见 FINDINGS）。
// 修复批次后：ark / reply,msg_idx= 属入站专用码，出站无消费点，由管道最后一步整段剥离，
// 故出站正文为空（既不再产出 foundItems，也不再作为字面量发给用户）。
func TestMessageEventFields_CQPrefixConflict(t *testing.T) {
	for _, in := range []string{
		"[CQ:ark,ark_type=picture,ark_name=图片,prompt=点击查看图片]",
		"[CQ:reply,msg_idx=REFIDX_x]",
	} {
		found := map[string][]string{}
		out := ProcessCQCodePipeline(in, found, nil)
		if len(found) != 0 {
			t.Errorf("新入站码不得产出出站 foundItems: in=%q found=%v", in, found)
		}
		if out != "" {
			t.Errorf("新入站码应被出站剥离(不产生字面量文本): in=%q out=%q", in, out)
		}
	}
	// 关键安全性质：msg_idx 绝不落入 reply_msg_id（否则出站会回复到错误消息）
	found := map[string][]string{}
	ProcessCQCodePipeline("[CQ:reply,msg_idx=REFIDX_x]", found, nil)
	if _, ok := found["reply_msg_id"]; ok {
		t.Fatalf("msg_idx 不得污染 reply_msg_id: %v", found)
	}
}

// TestMessageEventFields_ElementRecursive 元素级递归：嵌套卡片与语音附件
func TestMessageEventFields_ElementRecursive(t *testing.T) {
	msg := &dto.WSGroupATMessageData{
		MessageType: 103,
		MsgElements: []*dto.MsgElement{
			{
				MsgIdx:      "REFIDX_a",
				MessageType: 3,
				ARKData:     &dto.ARKData{ARKType: "miniapp", ARKName: "小程序"},
				Attachments: []*dto.MessageAttachment{
					{URL: "https://x.com/v.silk", ContentType: "voice", VoiceWavURL: "https://x.com/v.wav", ASRReferText: "语音"},
				},
			},
		},
	}
	segs := ConvertToSegmentedMessage(msg)
	if findSeg(segs, "ark") == nil {
		t.Errorf("元素级卡片应产出 ark 段: %+v", segs)
	}
	record := findSeg(segs, "record")
	if record == nil {
		t.Fatalf("元素级语音应产出 record 段: %+v", segs)
	}
	if segData(t, record)["asr_refer_text"] != "语音" {
		t.Errorf("元素级 asr 丢失: %+v", segs)
	}
	// 不出现无限递归/重复段
	arkCount := 0
	for _, s := range segs {
		if s["type"] == "ark" {
			arkCount++
		}
	}
	if arkCount != 1 {
		t.Errorf("ark 段数量: got %d, want 1", arkCount)
	}
}

// TestMessageEventFields_ImageRegression 回归保护：图片段行为不变，且无新增字段时不产出新段
func TestMessageEventFields_ImageRegression(t *testing.T) {
	msg := &dto.WSGroupMessageData{
		Content: "看图",
		Attachments: []*dto.MessageAttachment{
			{FileName: "a.png", ContentType: "image/png", URL: "https://x.com/a.png"},
		},
	}
	segs := ConvertToSegmentedMessage(msg)
	if len(segs) != 2 {
		t.Fatalf("应仅产出 text+image 两段: %+v", segs)
	}
	if segs[0]["type"] != "text" || segs[1]["type"] != "image" {
		t.Fatalf("段顺序/类型变化: %+v", segs)
	}
	imageData := segData(t, segs[1])
	if len(imageData) != 3 || imageData["file"] != "a.image" || imageData["subType"] != "0" || imageData["url"] != "https://x.com/a.png" {
		t.Errorf("图片段内容变化: %#v", imageData)
	}
	for _, segType := range []string{"record", "ark", "reply"} {
		if findSeg(segs, segType) != nil {
			t.Errorf("无新增字段时不应产出 %s 段: %+v", segType, segs)
		}
	}
}

// TestMessageEventFields_MediaResponse 富媒体上传响应新增 id / raw_url 字段
func TestMessageEventFields_MediaResponse(t *testing.T) {
	var resp dto.MediaResponse
	if err := json.Unmarshal([]byte(`{"file_uuid":"u","file_info":"f","ttl":300,"id":"m1","raw_url":"https://x"}`), &resp); err != nil {
		t.Fatalf("MediaResponse 反序列化失败: %v", err)
	}
	if resp.FileUUID != "u" || resp.FileInfo != "f" || resp.TTL != 300 {
		t.Errorf("既有字段变化: %+v", resp)
	}
	if resp.ID != "m1" || resp.RawURL != "https://x" {
		t.Errorf("新增字段反序列化错误: %+v", resp)
	}
}

// TestMessageEventFields_StringPath 字符串路径：语音/卡片/引用以 CQ 码补进上报文本
func TestMessageEventFields_StringPath(t *testing.T) {
	msg := &dto.WSGroupATMessageData{
		Content:     "",
		MessageType: 103,
		ArkData: &dto.ARKData{
			Prompt:  "点击查看图片",
			ARKType: "picture",
			ARKName: "图片",
		},
		Attachments: []*dto.MessageAttachment{
			{URL: "https://x.com/a.silk", ContentType: "voice", VoiceWavURL: "https://x.com/a.wav", ASRReferText: "你好"},
		},
		MsgElements: []*dto.MsgElement{
			{MsgIdx: "REFIDX_x", Content: "hi"},
		},
	}
	text := RevertTransformedText(msg, "group", nil, nil, 100, 200, false)
	for _, want := range []string{
		"[CQ:record,file=https://x.com/a.wav,asr=你好]",
		"[CQ:ark,ark_type=picture,ark_name=图片,prompt=点击查看图片]",
		"[CQ:reply,msg_idx=REFIDX_x]",
		"hi",
	} {
		if !strings.Contains(text, want) {
			t.Errorf("上报文本缺少 %q: %q", want, text)
		}
	}
}

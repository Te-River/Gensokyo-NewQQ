package handlers

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"strconv"
	"strings"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/tencent-connect/botgo/dto"
)

// seg 构造 OneBot 消息段（segment_type_koishi 格式）
func seg(segType string, data map[string]interface{}) map[string]interface{} {
	return map[string]interface{}{"type": segType, "data": data}
}

// parseGroupContent 简化 parseMessageContent 调用：群消息路径固定 GroupID/UserID
// 传入 nil 客户端：未开启 transfer_url 时解析路径不会触达 client/api/apiv2
// （cqparse 引入后第三返回值为 PendingAction，legacy 模式下恒为 nil，此处忽略）
func parseGroupContent(t *testing.T, message interface{}) (string, map[string][]string) {
	t.Helper()
	params := callapi.ParamsContent{
		GroupID: "g-test-openid",
		UserID:  "u-test-openid",
		Message: message,
	}
	text, items, _ := parseMessageContent(params, callapi.ActionMessage{Action: "send_group_msg", Params: params}, nil, nil, nil)
	return text, items
}

// TestParseMessageContentFoundItems 聚焦测试：消息段数组路径的 foundItems 解析与文本拼接
func TestParseMessageContentFoundItems(t *testing.T) {
	tests := []struct {
		name       string
		message    interface{}
		wantText   string
		wantKeys   map[string]int      // key → 期望 foundItems[key] 长度
		wantValues map[string][]string // key → 期望精确值
		wantNotKey func(string) bool   // 额外负面断言
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
			name: "wakeup 段提取 userid",
			message: []interface{}{
				seg("wakeup", map[string]interface{}{"userid": "0123456789abcdef0123456789abcdef"}),
				seg("text", map[string]interface{}{"text": "唤醒消息"}),
			},
			wantText:   "唤醒消息",
			wantKeys:   map[string]int{"wakeup": 1},
			wantValues: map[string][]string{"wakeup": {"0123456789abcdef0123456789abcdef"}},
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
			wantText:   "",
			wantNotKey: func(key string) bool { return true },
		},
		{
			name: "markdown base64 非 JSON 被跳过",
			message: []interface{}{
				seg("markdown", map[string]interface{}{"data": base64.StdEncoding.EncodeToString([]byte("not-json"))}),
			},
			wantText:   "",
			wantNotKey: func(key string) bool { return key == "markdown" },
		},
		{
			name: "set_group 动作段还原为 CQ 码字符串（数组路径）",
			message: []interface{}{
				seg("set_group", map[string]interface{}{"action": "ban", "group_id": "123", "user_id": "456", "duration": "60"}),
				seg("text", map[string]interface{}{"text": " 已禁言"}),
			},
			wantText: "[CQ:set_group,action=ban,group_id=123,user_id=456,duration=60] 已禁言",
		},
		{
			name: "set_group 动作段还原为 CQ 码字符串（TRSS map 路径）",
			message: map[string]interface{}{
				"type": "set_group",
				"data": map[string]interface{}{"action": "whole_ban", "group_id": "123", "enable": "true"},
			},
			wantText: "[CQ:set_group,action=whole_ban,group_id=123,enable=true]",
		},
		{
			name: "member 动作段还原为 CQ 码字符串（TRSS map 路径补齐）",
			message: map[string]interface{}{
				"type": "member",
				"data": map[string]interface{}{"type": "add", "group_id": "123", "user_id": "456"},
			},
			wantText: "[CQ:member,type=add,group_id=123,user_id=456]",
		},
		{
			name: "wakeup 段提取 userid（TRSS map 路径）",
			message: map[string]interface{}{
				"type": "wakeup",
				"data": map[string]interface{}{"userid": "0123456789abcdef0123456789abcdef"},
			},
			wantText:   "",
			wantKeys:   map[string]int{"wakeup": 1},
			wantValues: map[string][]string{"wakeup": {"0123456789abcdef0123456789abcdef"}},
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

	gotText, gotItems, _ := parseMessageContent(params, callapi.ActionMessage{Action: "send_private_msg", Params: params}, nil, nil, nil)

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

// ---------- markdown at 段缺 qq → 1400（与 Rust render_markdown 同口径） ----------

// mdContentPayload 把 markdown 正文包成 foundItems["markdown"] 形态的 base64 载荷。
func mdContentPayload(t *testing.T, content string) string {
	t.Helper()
	raw, err := json.Marshal(map[string]any{"markdown": map[string]any{"content": content}})
	if err != nil {
		t.Fatalf("构造 markdown 载荷失败: %v", err)
	}
	return base64.StdEncoding.EncodeToString(raw)
}

// TestParseMarkdownFromMessageERejectsAtWithoutQQ markdown 正文的 at 段必须能被渲染层渲染：
// 缺 qq 的 at 段、idmap 里查不到的数字 qq、渲染层认不出的写法（`[CQ:at,name=x,qq=1]`）都回 1400，
// 与 Rust render_markdown 同口径（审计 §8-2 / G2 / G4）。
// at 与逗号之间的空白（`[CQ:at ,qq=1]`）已由 cqAtNumRe 支持，走正常解析而非 1400。
func TestParseMarkdownFromMessageERejectsAtWithoutQQ(t *testing.T) {
	// 数字 qq 现在要过 idmap 反查（G4）：先绑定一个虚拟号，供合法用例使用
	boundOpenID := "u0123456789abcdef0123456789abc123"
	virtual, err := idmap.StoreIDv2(boundOpenID)
	if err != nil {
		t.Fatalf("StoreIDv2 失败: %v", err)
	}
	boundQQ := strconv.FormatInt(virtual, 10)

	rejected := []struct {
		name, content string
		want          error
	}{
		{"无参数 at", "[CQ:at] 开会", errMarkdownAtNoQQ},
		{"缺 qq 键", "[CQ:at,foo] 开会", errMarkdownAtNoQQ},
		{"qq 空值", "[CQ:at,qq=] 开会", errMarkdownAtNoQQ},
		{"qq 空白值", "[CQ:at, qq =  ] 开会", errMarkdownAtNoQQ},
		{"小写前缀", "[cq:at] 开会", errMarkdownAtNoQQ},
		{"中段出现", "开会 [CQ:at,foo] 了", errMarkdownAtNoQQ},
		// G2：qq 不在 `[CQ:at,` 之后的位置，cqAtNumRe 取不到参数 → 渲染层认不出
		{"参数顺序颠倒", "[CQ:at,name=x,qq=" + boundQQ + "] 开会", errMarkdownAtMalformed},
		{"at 后只有空格", "[CQ:at ] 开会", errMarkdownAtNoQQ},
		{"空白 at 无参数", "[CQ:at  ] 开会", errMarkdownAtNoQQ},
		// G4：数字 qq 反查不到真实 OpenID，放行只会原样发出 CQ 码
		{"未绑定的数字 qq", "[CQ:at,qq=987654321098] 开会", errMarkdownAtUnbound},
		{"未绑定的数字 qq 带附加参数", "[CQ:at,qq=987654321098,name=x] 开会", errMarkdownAtUnbound},
	}
	for _, tc := range rejected {
		t.Run(tc.name, func(t *testing.T) {
			_, _, err := parseMarkdownFromMessageE(mdContentPayload(t, tc.content))
			if !errors.Is(err, tc.want) {
				t.Fatalf("应返回 %v, got %v", tc.want, err)
			}
			if !strings.Contains(strings.ToLower(err.Error()), "[cq:at") {
				t.Fatalf("错误信息应带出问题段, got %v", err)
			}
		})
	}

	accepted := []struct{ name, content string }{
		{"数字 qq", "[CQ:at,qq=" + boundQQ + "] 开会"},
		{"数字 qq 带附加参数", "[CQ:at,qq=" + boundQQ + ",name=x] 开会"},
		{"空白容错", "[CQ:at, qq = " + boundQQ + "] 开会"},
		{"at 后带空白再跟 qq", "[CQ:at ,qq=" + boundQQ + "] 开会"},
		{"at 后带空白且键值带空白", "[CQ:at  , qq = " + boundQQ + " ] 开会"},
		{"键名大写", "[CQ:AT,QQ = " + boundQQ + "] 开会"},
		{"qq=all 若未归一仍算有 qq", "[CQ:at,qq=all] 开会"},
		{"其他 CQ 码不受影响", "[CQ:image,file=https://x.com/p.png] 图"},
		{"形似码 avatar 不是 at 段", "[CQ:avatar] 你好"},
		{"形似码 avatar 带 qq 不是 at 段", "[CQ:avatar,qq=1] 你好"},
		{"形似码 atx 不是 at 段", "[CQ:atx] 你好"},
		{"形似码 atx 带 qq 不是 at 段", "[CQ:atx,qq=987654321098] 你好"},
		{"无 at 段", "纯文本"},
	}
	for _, tc := range accepted {
		t.Run(tc.name, func(t *testing.T) {
			md, _, err := parseMarkdownFromMessageE(mdContentPayload(t, tc.content))
			if err != nil {
				t.Fatalf("合法载荷不应报错: %v", err)
			}
			if md == nil || md.Content != tc.content {
				t.Fatalf("应保留原正文, got %#v", md)
			}
		})
	}
}

// 文本路径不校验：不支持的 at 段原样透传（markdown 才回 1400）。
func TestTextAtWithoutQQPassesThrough(t *testing.T) {
	for _, text := range []string{
		"[CQ:at,foo] 你好",
		"[CQ:at] 你好",
		// at 与参数之间夹空白：管道不解析 at，原样透传给下游
		"[CQ:at ,qq=1] 你好",
		"[CQ:at ] 你好",
	} {
		foundItems := map[string][]string{}
		if out := ProcessCQCodePipeline(text, foundItems, "g-test-openid"); out != text {
			t.Fatalf("ProcessCQCodePipeline(%q) = %q, want 原样透传", text, out)
		}
	}

	// handler 级：文本路径不拦，正常发出一跳
	SetOutboundService(nil)
	t.Cleanup(func() { SetOutboundService(nil) })
	mock := &cardMockAPI{}
	raw, err := HandleSendGroupMsg(&cardRecorderClient{}, nil, mock, callapi.ActionMessage{
		Action: "send_group_msg",
		Params: callapi.ParamsContent{
			GroupID: openID32('g'),
			UserID:  openID32('u'),
			Message: "[CQ:at,foo] 你好",
		},
		Echo: "text-at-no-qq",
	})
	if err != nil {
		t.Fatalf("handler 返回错误: %v", err)
	}
	if got := retcodeOf(t, raw); got == 1400 {
		t.Fatalf("文本路径不应回 1400, got %d (%s)", got, raw)
	}
	if mock.groupCalls != 1 {
		t.Fatalf("文本路径应正常发送一次, got %d", mock.groupCalls)
	}
}

// G1：`[CQ:at,qq=虚拟号,name=x]` 这类带附加参数的 at 段同样要解析。
// Rust 侧按 key 取值、忽略其他参数，at_everyone 的 CQ 正则也一直兼容附加参数。
func TestResolveMarkdownAtMentionsExtraParams(t *testing.T) {
	realOpenID := "u0123456789abcdef0123456789abc125"
	virtual, err := idmap.StoreIDv2(realOpenID)
	if err != nil {
		t.Fatalf("StoreIDv2 失败: %v", err)
	}
	qq := strconv.FormatInt(virtual, 10)
	want := `<qqbot-at-user id="` + realOpenID + `" /> 你好`

	for _, tc := range []struct{ name, in string }{
		{"紧凑附加参数", "[CQ:at,qq=" + qq + ",name=x] 你好"},
		{"空白附加参数", "[CQ:at, qq = " + qq + " , name = 小明 ] 你好"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			if got := ResolveMarkdownAtMentions(tc.in); got != want {
				t.Fatalf("ResolveMarkdownAtMentions(%q) = %q, want %q", tc.in, got, want)
			}
		})
	}

	// 校验侧也必须放行同一形态，否则渲染层根本没机会跑（G1 + G4）
	md, _, err := parseMarkdownFromMessageE(mdContentPayload(t, "[CQ:at,qq="+qq+",name=x] 你好"))
	if err != nil {
		t.Fatalf("带附加参数的已绑定 at 不应被拒: %v", err)
	}
	if md == nil {
		t.Fatalf("载荷应解析成功")
	}
}

// G3：文本路径反查失败时保留原文，绝不能把整段静默删成空串。
func TestResolvePlainTextAtMentionsUnboundKeepsOriginal(t *testing.T) {
	for _, in := range []string{
		"[CQ:at,qq=987654321098] 你好",
		"[CQ:at,qq=987654321098,name=x] 你好",
		// at 后带空白的写法现已能解析，但反查失败时同样保留原文
		"[CQ:at ,qq=987654321098] 你好",
		"[CQ:at ] 你好",
	} {
		if got := resolvePlainTextAtMentions(in); got != in {
			t.Fatalf("resolvePlainTextAtMentions(%q) = %q, want 保留原文", in, got)
		}
	}
}

// `[CQ:at, qq = 虚拟号]` 在 markdown 里同样要解析成 <qqbot-at-user>（空白容错）。
func TestResolveMarkdownAtMentionsWhitespaceParams(t *testing.T) {
	realOpenID := "u0123456789abcdef0123456789abcdef"
	virtual, err := idmap.StoreIDv2(realOpenID)
	if err != nil {
		t.Fatalf("StoreIDv2 失败: %v", err)
	}
	qq := strconv.FormatInt(virtual, 10)

	want := `<qqbot-at-user id="` + realOpenID + `" /> 你好`
	if got := ResolveMarkdownAtMentions("[CQ:at, qq = " + qq + "] 你好"); got != want {
		t.Fatalf("空白容错的 at 段应解析为官方标签, got %q", got)
	}
	// at 与逗号之间的空白同样要解析（G2 修复后与 Rust 的 at 段识别对齐）
	if got := ResolveMarkdownAtMentions("[CQ:at ,qq = " + qq + "] 你好"); got != want {
		t.Fatalf("at 后带空白的 at 段应解析为官方标签, got %q", got)
	}
	if got := ResolveMarkdownAtMentions("[CQ:at  , qq = " + qq + " ] 你好"); got != want {
		t.Fatalf("at 后带空白且键值带空白的 at 段应解析为官方标签, got %q", got)
	}
	// 缺 qq 的 at 段不经此函数处理（由 parseMarkdownFromMessageE 提前拒绝）
	if got := ResolveMarkdownAtMentions("[CQ:at,foo]"); got != "[CQ:at,foo]" {
		t.Fatalf("缺 qq 的 at 段应原样保留, got %q", got)
	}
}

// 文本路径的 at 段：紧凑写法与空白写法都转 <@openid>（契约 5；空白容错与 Rust 同口径）。
func TestResolvePlainTextAtMentionsWhitespaceParams(t *testing.T) {
	realOpenID := "u0123456789abcdef0123456789abcdee"
	virtual, err := idmap.StoreIDv2(realOpenID)
	if err != nil {
		t.Fatalf("StoreIDv2 失败: %v", err)
	}
	qq := strconv.FormatInt(virtual, 10)
	want := "<@" + realOpenID + "> 你好"

	for _, tc := range []struct{ name, in string }{
		{"紧凑写法", "[CQ:at,qq=" + qq + "]你好"},
		{"空白写法", "[CQ:at, qq = " + qq + " ]你好"},
		{"at 后带空白", "[CQ:at ,qq=" + qq + "]你好"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			if got := resolvePlainTextAtMentions(tc.in); got != want {
				t.Fatalf("resolvePlainTextAtMentions(%q) = %q, want %q", tc.in, got, want)
			}
		})
	}
}

// ---------- markdown at 缺 qq：四条出站路径都回 1400 且一次都不发 ----------

// legacy 群聊：markdown 里缺 qq 的 at 段 → 1400。
func TestHandleSendGroupMsgMarkdownAtWithoutQQReturns1400(t *testing.T) {
	SetOutboundService(nil)
	t.Cleanup(func() { SetOutboundService(nil) })

	mock := &cardMockAPI{}
	payload := mdContentPayload(t, "[CQ:at,foo] 开会")
	raw, err := HandleSendGroupMsg(&cardRecorderClient{}, nil, mock, callapi.ActionMessage{
		Action: "send_group_msg",
		Params: callapi.ParamsContent{
			GroupID: openID32('g'),
			UserID:  openID32('u'),
			Message: "[CQ:markdown,data=base64://" + payload + "]",
		},
		Echo: "md-at-no-qq-group",
	})
	if err != nil {
		t.Fatalf("handler 返回错误: %v", err)
	}
	if got := retcodeOf(t, raw); got != 1400 {
		t.Fatalf("legacy 群聊应回 1400, got %d (%s)", got, raw)
	}
	if !strings.Contains(raw, "缺少 qq 参数") {
		t.Fatalf("回执应说明缺 qq 参数, got %s", raw)
	}
	if mock.groupCalls != 0 {
		t.Fatalf("非法载荷不应发出, got %d 次发送", mock.groupCalls)
	}
}

// G4：markdown 里的数字 at 反查不到真实 OpenID → 1400 且一次都不发（Rust 同口径）。
// 放行只会让渲染层把 [CQ:at,qq=未绑定号] 原样发给用户。
func TestHandleSendGroupMsgMarkdownAtUnboundReturns1400(t *testing.T) {
	SetOutboundService(nil)
	t.Cleanup(func() { SetOutboundService(nil) })

	mock := &cardMockAPI{}
	payload := mdContentPayload(t, "[CQ:at,qq=987654321098] 开会")
	raw, err := HandleSendGroupMsg(&cardRecorderClient{}, nil, mock, callapi.ActionMessage{
		Action: "send_group_msg",
		Params: callapi.ParamsContent{
			GroupID: openID32('g'),
			UserID:  openID32('u'),
			Message: "[CQ:markdown,data=base64://" + payload + "]",
		},
		Echo: "md-at-unbound-group",
	})
	if err != nil {
		t.Fatalf("handler 返回错误: %v", err)
	}
	if got := retcodeOf(t, raw); got != 1400 {
		t.Fatalf("未绑定的数字 at 应回 1400, got %d (%s)", got, raw)
	}
	if !strings.Contains(raw, "虚拟号未绑定") {
		t.Fatalf("回执应说明虚拟号未绑定, got %s", raw)
	}
	if mock.groupCalls != 0 {
		t.Fatalf("非法载荷不应发出, got %d 次发送", mock.groupCalls)
	}
}

// G2 修复：markdown 里 `[CQ:at ,qq=数字]`（at 与逗号之间带空白）走正常解析——
// 已绑定 → 渲染成 <qqbot-at-user> 正常发出；未绑定 / 缺 qq → 仍回 1400，不能放行 CQ 码原文。
func TestHandleSendGroupMsgMarkdownAtSpaceForm(t *testing.T) {
	SetOutboundService(nil)
	t.Cleanup(func() { SetOutboundService(nil) })

	boundOpenID := "u0123456789abcdef0123456789abc127"
	virtual, err := idmap.StoreIDv2(boundOpenID)
	if err != nil {
		t.Fatalf("StoreIDv2 失败: %v", err)
	}
	boundQQ := strconv.FormatInt(virtual, 10)

	send := func(content, echo string) (string, *cardMockAPI) {
		mock := &cardMockAPI{}
		raw, err := HandleSendGroupMsg(&cardRecorderClient{}, nil, mock, callapi.ActionMessage{
			Action: "send_group_msg",
			Params: callapi.ParamsContent{
				GroupID: openID32('g'),
				UserID:  openID32('u'),
				Message: "[CQ:markdown,data=base64://" + mdContentPayload(t, content) + "]",
			},
			Echo: echo,
		})
		if err != nil {
			t.Fatalf("handler 返回错误: %v", err)
		}
		return raw, mock
	}

	// 已绑定：正常发出，正文里的 CQ 码渲染成官方 @ 标签
	_, mock := send("[CQ:at , qq = "+boundQQ+"] 开会", "md-at-space-bound")
	if mock.groupCalls != 1 {
		t.Fatalf("已绑定的空白写法应正常发送一次, got %d", mock.groupCalls)
	}
	mtc, ok := mock.lastGroup.(*dto.MessageToCreate)
	if !ok {
		t.Fatalf("应发送 MessageToCreate, got %T", mock.lastGroup)
	}
	if mtc.Markdown == nil || !strings.Contains(mtc.Markdown.Content, `<qqbot-at-user id="`+boundOpenID+`" />`) {
		t.Fatalf("CQ 码应渲染成官方标签, got %#v", mtc.Markdown)
	}

	// 未绑定：仍回 1400 且不发
	raw, mock := send("[CQ:at ,qq=987654321098] 开会", "md-at-space-unbound")
	if got := retcodeOf(t, raw); got != 1400 {
		t.Fatalf("未绑定的空白写法应回 1400, got %d (%s)", got, raw)
	}
	if mock.groupCalls != 0 {
		t.Fatalf("非法载荷不应发出, got %d 次发送", mock.groupCalls)
	}

	// 缺 qq 的空白写法：1400
	raw, mock = send("[CQ:at ] 开会", "md-at-space-noqq")
	if got := retcodeOf(t, raw); got != 1400 {
		t.Fatalf("缺 qq 的空白写法应回 1400, got %d (%s)", got, raw)
	}
	if mock.groupCalls != 0 {
		t.Fatalf("非法载荷不应发出, got %d 次发送", mock.groupCalls)
	}
}

// legacy 私聊：markdown 里缺 qq 的 at 段 → 1400。
func TestHandleSendPrivateMsgMarkdownAtWithoutQQReturns1400(t *testing.T) {
	SetOutboundService(nil)
	t.Cleanup(func() { SetOutboundService(nil) })

	mock := &cardMockAPI{}
	payload := mdContentPayload(t, "[CQ:at] 开会")
	raw, err := HandleSendPrivateMsg(&cardRecorderClient{}, nil, mock, callapi.ActionMessage{
		Action: "send_private_msg",
		Params: callapi.ParamsContent{
			UserID:  openID32('u'),
			Message: "[CQ:markdown,data=base64://" + payload + "]",
		},
		Echo: "md-at-no-qq-private",
	})
	if err != nil {
		t.Fatalf("handler 返回错误: %v", err)
	}
	if got := retcodeOf(t, raw); got != 1400 {
		t.Fatalf("legacy 私聊应回 1400, got %d (%s)", got, raw)
	}
	if mock.c2cCalls != 0 {
		t.Fatalf("非法载荷不应发出, got %d 次发送", mock.c2cCalls)
	}
}

// 新链群聊：同一校验前置到 SendFoundItemsGroup。
func TestSendFoundItemsGroupRejectsMarkdownAtWithoutQQ(t *testing.T) {
	mock := &cardMockAPI{}
	found := map[string][]string{"markdown": {mdContentPayload(t, "[CQ:at,foo] 开会")}}
	if _, err := SendFoundItemsGroup(context.Background(), openID32('g'), "", found, mock); !errors.Is(err, errMarkdownAtNoQQ) {
		t.Fatalf("应返回 errMarkdownAtNoQQ, got %v", err)
	}
	if mock.groupCalls != 0 {
		t.Fatalf("非法载荷不应发出, got %d 次发送", mock.groupCalls)
	}
}

// 新链私聊：同一校验前置到 SendFoundItemsPrivate。
func TestSendFoundItemsPrivateRejectsMarkdownAtWithoutQQ(t *testing.T) {
	mock := &cardMockAPI{}
	found := map[string][]string{"markdown": {mdContentPayload(t, "[CQ:at] 开会")}}
	if _, err := SendFoundItemsPrivate(context.Background(), openID32('u'), "", found, false, mock); !errors.Is(err, errMarkdownAtNoQQ) {
		t.Fatalf("应返回 errMarkdownAtNoQQ, got %v", err)
	}
	if mock.c2cCalls != 0 {
		t.Fatalf("非法载荷不应发出, got %d 次发送", mock.c2cCalls)
	}
}

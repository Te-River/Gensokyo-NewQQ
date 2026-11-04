package handlers

// cq_whitespace_tolerance_test.go：CQ 码统一最大兼容契约（§1~§4）的镜像用例表，
// 与 Rust 侧容错用例表逐条对应（本轮把全部 CQ 段类型一并收口）。
// 口径：段名之前（`[CQ:` 后）与之后、`,` 或 `]` 之前允许任意空白；键与 `=`、
// `=` 与值两侧允许空白；键 trim、值仅 trim 首尾空白、内部空白保留；
// 参数按键取值（任意键序）；qq 允许出现在任意参数位置。
// 大小写口径（统一契约 §1）：段名与参数键大小写不敏感，`[CQ:Image,File=x]` =
// `[CQ:image,file=x]`；`[CQ:` 前缀字面仍区分大小写（小写 [cq: 维持透传）。

import (
	"encoding/base64"
	"strconv"
	"strings"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/handlers/cqparse"
	"github.com/hoshinonyaruko/gensokyo/idmap"
)

// TestCQWhitespaceTolerancePipeline legacy 字符串管道逐类型空白容错表。
func TestCQWhitespaceTolerancePipeline(t *testing.T) {
	compilePatternsOnce.Do(initPlatformPatterns)

	mdJSON := base64.StdEncoding.EncodeToString([]byte(`{"content":"hi"}`))

	cases := []struct {
		name     string
		input    string
		wantText string
		wantKey  string
		wantVal  string
	}{
		// 媒体：image / record / video / file（值内空格保留）
		{"image 空白", "A[CQ:image , file = https://example.com/a.png]B", "AB", "url_images", "example.com/a.png"},
		// G-A：\f 属 ASCII 五元集 {空格,\t,\n,\f,\r}，正则 `\s` 已含——手写词法补齐后三路径同口径均可识别
		{"image 段名后 \\f", "[CQ:image\f , file = https://example.com/a.png]B", "B", "url_images", "example.com/a.png"},
		{"image 逗号后 \\f（键层 TrimSpace 现状）", "[CQ:image,\ffile=https://example.com/a.png]", "", "url_images", "example.com/a.png"},
		// G-A 钉现状：\v 与全角空格不在 Go 正则 `\s` 内 → 不识别、保留字面
		{"image 段名后 \\v 不识别（现状）", "[CQ:image\v,file=x.png]", "[CQ:image\v,file=x.png]", "", ""},
		{"image 段名后全角空格不识别（现状）", "[CQ:image　,file=x.png]", "[CQ:image　,file=x.png]", "", ""},
		{"record 值内空格保留", "[CQ:record,file = http://example.com/my voice.amr]", "", "url_record", "example.com/my voice.amr"},
		{"video 空白", "[CQ:video , file = https://example.com/v.mp4]", "", "url_videos", "example.com/v.mp4"},
		{"file 空白+文件名带空格", "[CQ:file , file_name = my file.txt , file = https://example.com/my file.zip]", "", "url_files", "example.com/my file.zip"},
		{"file 名键序颠倒", "[CQ:file , file = base64://QUJD , file_name = a.bin]", "", "base64_file", "QUJD"},
		{"record asr 尾随参数+空白", "[CQ:record , file = https://x/a.wav , asr=你好]", "", "url_records", "x/a.wav"},
		// 控制：reply / active / wakeup
		{"reply 空白", "[CQ:reply,id = 123] 正文", " 正文", "reply_msg_id", "123"},
		{"reply msg_idx 空白剥离", "[CQ:reply , msg_idx = REFIDX_1]x", "x", "", ""},
		{"active 带参空白", "[CQ:active , type = 1 , sub_type = 2]", "", "active_type", "1"},
		{"active 裸码后空白", "[CQ:active ]x", "x", "active", "true"},
		// 段名后空白再跟逗号：走带参分支（inner 无 k=v），行为同裸码 active=true（对齐 Rust 用例）
		{"active 裸码后空白再跟逗号", "[CQ:active , ]x", "x", "active", "true"},
		{"wakeup 空白", "[CQ:wakeup , userid = ABC123]", "", "wakeup", "ABC123"},
		// 内容型：markdown / keyboard / card / music / input_notify / stream
		{"markdown base64 空白", "[CQ:markdown , data = base64://QUJD]", "", "markdown", "QUJD"},
		{"markdown JSON 空白", `[CQ:markdown , data = {"content":"hi"}]`, "", "markdown", mdJSON},
		{"keyboard base64 空白", "[CQ:keyboard , data = base64://QUJD]", "", "keyboard", "ABC"},
		{"card 空白+非法组合可解析", "[CQ:card , title=A, desc=B]", "", "card", `{"desc":"B","title":"A"}`},
		{"music 空白", "[CQ:music , type = qq, id = 123]", "", "qqmusic", "123"},
		{"input_notify 空白", "[CQ:input_notify , type = 6 , second = 10]", "", "input_notify", `{"second":"10","type":"6"}`},
		{"stream 空白（冒号键语法）", "[CQ:stream , type : start , qq : 123]", "", "stream", `{"qq":"123","type":"start"}`},
		// stream 双语法（统一契约 §3）：等号语法与冒号语法等价，可混用。
		{"stream 等号语法识别", "[CQ:stream,type=start,qq=123]", "", "stream", `{"qq":"123","type":"start"}`},
		{"stream 等号+空白识别", "[CQ:stream , type = start , qq = 123]", "", "stream", `{"qq":"123","type":"start"}`},
		{"stream 混用等号冒号", "[CQ:stream,type=start,qq:123]", "", "stream", `{"qq":"123","type":"start"}`},
		// avatar：反查+URL 构造为离线逻辑，产物含动态 URL，见下方独立前缀断言
		// 透传与现状口径
		{"face 透传（现状：无解析路径）", "[CQ:face , id = 1] hi", "[CQ:face , id = 1] hi", "", ""},
		{"file 裸值无协议（现状：legacy 字符串链保留字面）", "[CQ:file,file=my file.txt]", "[CQ:file,file=my file.txt]", "", ""},
		// 统一契约 §1/§2：段名与键名大小写不敏感（`[CQ:` 前缀字面仍区分大小写）
		{"段名大写同样识别", "[CQ:Image , File = https://x/a.png]", "", "url_images", "x/a.png"},
		{"键名大写同样识别", "[CQ:image , File = https://x/a.png]", "", "url_images", "x/a.png"},
		{"段名键名全大写组合", "[CQ:RECORD , ASR = ab , FILE = https://x/a.wav]", "", "url_records", "x/a.wav"},
		{"段名前空白同样识别", "[CQ: image , file = https://x/a.png]", "", "url_images", "x/a.png"},
		// 畸形段（现状保持）：未闭合不吞
		{"未闭合畸形段保留", "hi[CQ:image,file=x.png", "hi[CQ:image,file=x.png", "", ""},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			found := map[string][]string{}
			out := ProcessCQCodePipeline(tc.input, found, nil)
			if out != tc.wantText {
				t.Fatalf("正文: got %q, want %q", out, tc.wantText)
			}
			if tc.wantKey == "" {
				if len(found) != 0 {
					t.Fatalf("不应产生 foundItems: %v", found)
				}
				return
			}
			got := found[tc.wantKey]
			if len(got) != 1 || got[0] != tc.wantVal {
				t.Fatalf("%s: got %#v, want %q (found=%v)", tc.wantKey, got, tc.wantVal, found)
			}
		})
	}
}

// TestCQAvatarWhitespaceTolerance [CQ:avatar , qq = ...] 空白形态可被识别并替换为 image 码
// （反查与 URL 构造均为离线逻辑；产物含动态 URL，只断言前缀与尾随文本）。
func TestCQAvatarWhitespaceTolerance(t *testing.T) {
	boundOpenID := "u0abcdef0123456789abcdef0123456789"
	virtual, err := idmap.StoreIDv2(boundOpenID)
	if err != nil {
		t.Fatalf("StoreIDv2 失败: %v", err)
	}
	boundQQ := strconv.FormatInt(virtual, 10)
	out := ProcessCQAvatarNoGroupID("[CQ:avatar , qq = " + boundQQ + "]尾")
	if !strings.HasPrefix(out, "[CQ:image,file=") || !strings.HasSuffix(out, "尾") {
		t.Fatalf("avatar 空白形态未被识别: %q", out)
	}
}

// TestCQCardWhitespaceThenValidation card 空白形态解析成功后，四字段校验仍按既有口径拒绝（1400 来源不变）。
func TestCQCardWhitespaceThenValidation(t *testing.T) {
	found := map[string][]string{}
	if out := ProcessCQCodePipeline("[CQ:card , title=A, desc=B]", found, nil); out != "" {
		t.Fatalf("card 段应被移除: %q", out)
	}
	if len(found["card"]) != 1 {
		t.Fatalf("card 应入库: %v", found)
	}
	if _, err := buildCardFromJSON(found["card"][0]); err == nil {
		t.Fatalf("缺 url/pic 的卡片应按既有校验拒绝")
	}
}

// TestCQAtQQAnyPositionRender 统一契约 §7：at 段 qq 非首位 + 空白，渲染层按键取值正常渲染。
func TestCQAtQQAnyPositionRender(t *testing.T) {
	boundOpenID := "u0fedcba9876543210fedcba9876543210"
	virtual, err := idmap.StoreIDv2(boundOpenID)
	if err != nil {
		t.Fatalf("StoreIDv2 失败: %v", err)
	}
	boundQQ := strconv.FormatInt(virtual, 10)

	cases := []struct {
		name, in string
	}{
		{"qq 非首位", "[CQ:at,name=x,qq=" + boundQQ + "]"},
		{"qq 非首位+全空白", "[CQ:AT , name = x , QQ = " + boundQQ + " ]"},
		{"qq 首位+全空白（回归）", "[CQ:at , qq = " + boundQQ + " ]"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got := ResolveMarkdownAtMentions("hi " + tc.in + " 开会")
			want := `hi <qqbot-at-user id="` + boundOpenID + `" /> 开会`
			if got != want {
				t.Fatalf("渲染: got %q, want %q", got, want)
			}
		})
	}

	// 校验层同口径放行
	if _, _, err := parseMarkdownFromMessageE(mdContentPayload(t, "[CQ:at,name=x,qq="+boundQQ+"] 开会")); err != nil {
		t.Fatalf("qq 非首位应通过 markdown 校验: %v", err)
	}
}

// TestCQAtLeadSpaceNowRenderable 统一契约 §4：段名前带空白的 at 段与其余形态同等识别，
// 校验与渲染均放行（旧 errMarkdownAtMalformed 分支已因不可达删除）。
func TestCQAtLeadSpaceNowRenderable(t *testing.T) {
	boundOpenID := "u0555555555555555555555555555aaaa"
	virtual, err := idmap.StoreIDv2(boundOpenID)
	if err != nil {
		t.Fatalf("StoreIDv2 失败: %v", err)
	}
	code := "[CQ: at,qq=" + strconv.FormatInt(virtual, 10) + "]"
	if _, _, err := parseMarkdownFromMessageE(mdContentPayload(t, code+" 开会")); err != nil {
		t.Fatalf("段名前空白形态应通过 markdown 校验, got %v", err)
	}
	want := `hi <qqbot-at-user id="` + boundOpenID + `" /> 开会`
	if got := ResolveMarkdownAtMentions("hi " + code + " 开会"); got != want {
		t.Fatalf("段名前空白形态应正常渲染: got %q, want %q", got, want)
	}
}

// TestNormalizeAtEveryoneKeyAnywhere @全员归一同步收口：qq 任意参数位置 + 空白。
func TestNormalizeAtEveryoneKeyAnywhere(t *testing.T) {
	tag := "<qqbot-at-everyone />"
	cases := []struct {
		in, want string
	}{
		{"[CQ:at,name=x,qq=all]", tag},
		{"[CQ:AT , name = x , QQ = EVERYONE ]", tag},
		{"[CQ:at , qq = all]", tag},
		{"前[CQ:at,qq=everyone]后", "前" + tag + "后"},
		{"[CQ:at,qq=allies] 不误判", "[CQ:at,qq=allies] 不误判"},
		{"[CQ:atx,qq=all] 不误判", "[CQ:atx,qq=all] 不误判"},
		{"[CQ:avatar,qq=all] 不误判", "[CQ:avatar,qq=all] 不误判"},
	}
	for _, tc := range cases {
		if got := NormalizeAtEveryone(tc.in); got != tc.want {
			t.Fatalf("NormalizeAtEveryone(%q) = %q, want %q", tc.in, got, tc.want)
		}
	}
}

// TestCQParseTokenizerWhitespaceTolerance 新链 cqparse 词法层空白容错。
func TestCQParseTokenizerWhitespaceTolerance(t *testing.T) {
	cases := []struct {
		name       string
		in         string
		wantAction string
		wantKind   cqparse.Kind
		wantKey    string
		wantVal    string
	}{
		{"record 裸值内部空格保留", "[CQ:record , file = my voice.amr]", "record", cqparse.KindMedia, "file", "my voice.amr"},
		{"image 空白", "[CQ:image , file = https://example.com/a.png]", "image", cqparse.KindMedia, "file", "https://example.com/a.png"},
		// G-A：段名后空白五元集含 \f（与 legacy 正则 `\s` 同口径）；键侧 \f 走 buildParams TrimSpace 现状。
		{"image 段名后 \\f", "[CQ:image\f , file = https://example.com/a.png]", "image", cqparse.KindMedia, "file", "https://example.com/a.png"},
		{"image 逗号后 \\f（键层现状）", "[CQ:image,\ffile=https://example.com/a.png]", "image", cqparse.KindMedia, "file", "https://example.com/a.png"},
		{"keyboard JSON 前空白仍可配平", `[CQ:keyboard , data = {"k":[1,2]}]`, "keyboard", cqparse.KindContent, "data", `{"k":[1,2]}`},
		{"face 空白（透传类别不变）", "[CQ:face , id = 1]", "face", cqparse.KindPassthrough, "id", "1"},
		{"reply 空白", "[CQ:reply,id = 123]", "reply", cqparse.KindControl, "id", "123"},
		{"裸码段名后空白", "[CQ:active ]", "active", cqparse.KindControl, "", ""},
		// 统一契约 §1/§3/§4：段名大小写不敏感、段名前空白、键名小写归一、任意键序
		{"段名大写归一小写", "[CQ:Image , File = https://x/a.png]", "image", cqparse.KindMedia, "file", "https://x/a.png"},
		{"段名前空白同样识别", "[CQ: image,file=https://x/a.png]", "image", cqparse.KindMedia, "file", "https://x/a.png"},
		{"键序颠倒+大小写+asr 组合", "[CQ: RECORD , ASR = ab , file = https://x/my voice.wav]", "record", cqparse.KindMedia, "file", "https://x/my voice.wav"},
		{"stream 冒号语法大小写", "[CQ:Stream , TYPE : start , QQ : 1]", "stream", cqparse.KindContent, "qq", "1"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			doc := cqparse.Tokenize(tc.in + "尾")
			if len(doc.Tokens) < 2 {
				t.Fatalf("Token 数异常: %#v", doc.Tokens)
			}
			tok := doc.Tokens[0]
			if tok.Action != tc.wantAction || tok.Kind != tc.wantKind {
				t.Fatalf("got %v/%v, want %v/%v", tok.Action, tok.Kind, tc.wantAction, tc.wantKind)
			}
			if tc.wantKey != "" && tok.Params[tc.wantKey] != tc.wantVal {
				t.Fatalf("params[%s] = %q, want %q (all=%v)", tc.wantKey, tok.Params[tc.wantKey], tc.wantVal, tok.Params)
			}
			if last := doc.Tokens[len(doc.Tokens)-1]; last.Kind != cqparse.KindText || last.Raw != "尾" {
				t.Fatalf("尾随文本异常: %#v", last)
			}
		})
	}

	// G-A 钉现状：\v 与全角空格不在段名后空白五元集内（Go 正则 `\s` 亦不含），整段按文本处理
	for _, in := range []string{"[CQ:image\v,file=x.png]", "[CQ:image　,file=x.png]"} {
		doc := cqparse.Tokenize(in)
		for _, tok := range doc.Tokens {
			if tok.Kind != cqparse.KindText {
				t.Fatalf("%q 应保持文本: %#v", in, doc.Tokens)
			}
		}
	}

	// 畸形段现状：未闭合仍按文本
	doc := cqparse.Tokenize("hi[CQ:image,file=x.png")
	for _, tok := range doc.Tokens {
		if tok.Kind != cqparse.KindText {
			t.Fatalf("未闭合码应保持文本: %#v", doc.Tokens)
		}
	}
}

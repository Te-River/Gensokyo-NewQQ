package handlers

// cq_union_contract_test.go：任务 #24「最大兼容并集口径」的镜像契约表驱动用例，
// 覆盖四条口径（§1 段名/键名大小写不敏感、§2 按键取值任意键序、§3 stream 双语法、
// §4 段名前空白）× 三条解析路径（legacy 正则管道 / cqparse 新链 / internal 新链），
// 并钉死边界：真畸形仍透传、record 的 ,asr= 切分保留、值内部空格保留、
// `[CQ:` 前缀与小写 [cq: 仍区分大小写。
// 既有 golden/转义/出站序列化测试零改动，由全量 go test 通过兜底。

import (
	"reflect"
	"strconv"
	"strings"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/handlers/cqparse"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	domainmessage "github.com/hoshinonyaruko/gensokyo/internal/domain/message"
)

// unionLegacyRun 走 legacy 字符串统一管道（无群场景，与空白容忍测试同驱动方式）。
func unionLegacyRun(t *testing.T, in string) (string, map[string][]string) {
	t.Helper()
	compilePatternsOnce.Do(initPlatformPatterns)
	found := map[string][]string{}
	text := ProcessCQCodePipeline(in, found, nil)
	return text, found
}

// unionParseRun 走 cqparse 新链纯解析（deps 置空：表中仅媒体/控制/内容类，不触网）。
func unionParseRun(t *testing.T, in string) (string, map[string][]string, []cqparse.PendingAction) {
	t.Helper()
	text, items, pendings, err := cqparse.Parse(
		cqparse.Input{Kind: cqparse.InputString, String: in}, &cqparse.Deps{})
	if err != nil {
		t.Fatalf("cqparse.Parse: %v", err)
	}
	return text, items, pendings
}

// unionInternalRun 走 internal 新链字符串解析并归一为消息段数组。
func unionInternalRun(t *testing.T, in string) []domainmessage.Segment {
	t.Helper()
	pm, err := domainmessage.ParseOneBotString(in)
	if err != nil {
		t.Fatalf("ParseOneBotString: %v", err)
	}
	return domainmessage.Canonicalize(pm)
}

// unionFirst 取 foundItems[key] 唯一值。
func unionFirst(t *testing.T, items map[string][]string, key string) string {
	t.Helper()
	got := items[key]
	if len(got) != 1 {
		t.Fatalf("foundItems[%s] 期望 1 项, got %v (all=%v)", key, got, items)
	}
	return got[0]
}

// TestCQUnionContractThreePaths 形态×路径主表：同一输入三路对拍。
// internal 列为新链当前落形（file 值含 scheme、UnknownPart 保留原文），
// 与 legacy/cqparse 的剥 scheme 口径是既有分层差异，本表分别钉住。
func TestCQUnionContractThreePaths(t *testing.T) {
	cases := []struct {
		name         string
		input        string
		wantText     string                  // legacy 与 cqparse 的产物正文（两链应一致）
		wantKey      string                  // 断言键（legacy 与 cqparse 同键同值）
		wantVal      string                  // 断言值（"" 表示仅断言 wantNoItems）
		wantNoItems  bool                    // true 时断言两链均无 foundItems
		wantInternal []domainmessage.Segment // internal 新链 canonical（nil 表示跳过该路径）
	}{
		// ---- §1 段名/键名大小写不敏感 ----
		{
			name: "image 段名大写", input: "[CQ:Image,File=https://x/a.png]",
			wantText: "", wantKey: "url_images", wantVal: "x/a.png",
			wantInternal: []domainmessage.Segment{{Type: "image", Data: map[string]string{"file": "https://x/a.png"}}},
		},
		{
			name: "image 键名大写", input: "[CQ:image,FILE=https://x/a.png]",
			wantText: "", wantKey: "url_images", wantVal: "x/a.png",
		},
		{
			name: "video 段名键名全大写", input: "[CQ:VIDEO , FILE = base64://QUJD]",
			wantText: "", wantKey: "base64_video", wantVal: "QUJD",
			wantInternal: []domainmessage.Segment{{Type: "video", Data: map[string]string{"file": "base64://QUJD"}}},
		},
		// ---- §2 按键取值、任意键序（媒体段 file 不再要求首键） ----
		{
			name: "image 键序 name 前 file 后", input: "[CQ:image,name=x,file=https://x/a.png]",
			wantText: "", wantKey: "url_images", wantVal: "x/a.png",
			wantInternal: []domainmessage.Segment{{Type: "image", Data: map[string]string{"file": "https://x/a.png"}}},
		},
		{
			name: "image 键序+大小写+空白组合", input: "[CQ:image , NAME = x , File = https://x/a.png]",
			wantText: "", wantKey: "url_images", wantVal: "x/a.png",
		},
		{
			name:     "record 键序 asr 前 file 后（大小写+空白+asr 组合）",
			input:    "[CQ:RECORD , ASR = ab , File = https://x/my voice.wav]",
			wantText: "", wantKey: "url_records", wantVal: "x/my voice.wav",
			wantInternal: []domainmessage.Segment{{Type: "record", Data: map[string]string{"file": "https://x/my voice.wav"}}},
		},
		{
			name:     "file 键序 file_name 前（大小写+空白）",
			input:    "[CQ:FILE , FILE_NAME = a.bin , file = base64://QUJD]",
			wantText: "", wantKey: "base64_file", wantVal: "QUJD",
		},
		{
			name: "file 键序（断言 file_name 同入）", input: "[CQ:file , name = x , file = https://x/a.zip , file_name = a.zip]",
			wantText: "", wantKey: "url_files", wantVal: "x/a.zip",
		},
		{
			name: "markdown 键序+大小写", input: "[CQ:Markdown , note = x , DATA = base64://QUJD]",
			wantText: "", wantKey: "markdown", wantVal: "QUJD",
			wantInternal: []domainmessage.Segment{{Type: "markdown", Data: map[string]string{"data": "base64://QUJD"}}},
		},
		{
			name: "keyboard 键序+大小写", input: "[CQ:KEYBOARD , x = 1 , data = base64://QUJD]",
			wantText: "", wantKey: "keyboard", wantVal: "ABC",
		},
		{
			name: "music 键序颠倒+大小写", input: "[CQ:Music , id = 123456 , TYPE = qq]",
			wantText: "", wantKey: "qqmusic", wantVal: "123456",
			wantInternal: []domainmessage.Segment{{Type: "qqmusic", Data: map[string]string{"id": "123456"}}},
		},
		{
			name: "input_notify 键序颠倒+大小写", input: "[CQ:INPUT_NOTIFY , second = 5 , Type = 1]",
			wantText: "", wantKey: "input_notify", wantVal: `{"second":"5","type":"1"}`,
		},
		{
			name: "reply 键序+大小写", input: "[CQ:Reply , name = x , ID = 829 ]",
			wantText: "", wantKey: "reply_msg_id", wantVal: "829",
			wantInternal: []domainmessage.Segment{{Type: "reply", Data: map[string]string{"id": "829"}}},
		},
		{
			name: "active 键序+大小写", input: "[CQ:ACTIVE , SUB_TYPE = 1 , TYPE = push ]",
			wantText: "", wantKey: "active_sub_type", wantVal: "1",
		},
		{
			name: "wakeup 键序+大小写", input: "[CQ:Wakeup , note = x , USERID = ABC123]",
			wantText: "", wantKey: "wakeup", wantVal: "ABC123",
		},
		{
			name:     "card 键名大小写归一（JSON 为小写规范形）",
			input:    "[CQ:Card , DESC = 描述 , Title = 标题 , Pic = https://x.com/p.png , URL = https://x.com]",
			wantText: "", wantKey: "card",
			wantVal: `{"desc":"描述","pic":"https://x.com/p.png","title":"标题","url":"https://x.com"}`,
		},
		// ---- §4 段名前空白 ----
		{
			name: "image 段名前空白", input: "[CQ: image , file = https://x/a.png]",
			wantText: "", wantKey: "url_images", wantVal: "x/a.png",
			wantInternal: []domainmessage.Segment{{Type: "image", Data: map[string]string{"file": "https://x/a.png"}}},
		},
		{
			name: "reply 段名前空白", input: "[CQ: reply,id=829]正文",
			wantText: "正文", wantKey: "reply_msg_id", wantVal: "829",
		},
		{
			name: "ark 大写+段名前空白剥离", input: "[CQ: ARK , p = 1]文本",
			wantText: "文本", wantNoItems: true,
		},
		{
			name: "msg_idx 大写键名剥离", input: "[CQ:Reply , MSG_IDX = REFIDX_x]文本",
			wantText: "文本", wantNoItems: true,
		},
		// ---- F1 尾键：值按键取值，尾部键不吞进值 ----
		{
			name: "image 尾键 name 不吞值", input: "[CQ:image,file=https://x/a.png,name=y]",
			wantText: "", wantKey: "url_images", wantVal: "x/a.png",
			wantInternal: []domainmessage.Segment{{Type: "image", Data: map[string]string{"file": "https://x/a.png"}}},
		},
		{
			name: "markdown 尾键不污染载荷", input: "[CQ:markdown,data=base64://QUJD,note=x]",
			wantText: "", wantKey: "markdown", wantVal: "QUJD",
		},
		{
			name: "keyboard 尾键不污染 base64 载荷", input: "[CQ:keyboard,data=base64://QUJD,id=x]",
			wantText: "", wantKey: "keyboard", wantVal: "ABC",
		},
		// ---- F2 段名整词：前缀形段名不被目标段名吞掉 ----
		{
			name: "CARDIGAN 不被 card 前缀吞", input: "[CQ:CARDIGAN,x=1]文本",
			wantText: "[CQ:CARDIGAN,x=1]文本", wantNoItems: true,
			wantInternal: []domainmessage.Segment{
				{Type: "CARDIGAN", Data: map[string]string{"x": "1"}},
				{Type: "text", Data: map[string]string{"text": "文本"}},
			},
		},
		// ---- 边界：真畸形仍透传 ----
		{
			name: "未闭合仍透传", input: "hi[CQ:image,file=https://x/a.png",
			wantText: "hi[CQ:image,file=https://x/a.png", wantNoItems: true,
			wantInternal: []domainmessage.Segment{
				{Type: "text", Data: map[string]string{"text": "hi"}},
				{Type: "text", Data: map[string]string{"text": "[CQ:image,file=https://x/a.png"}},
			},
		},
		{
			name: "花括号配平失败仍透传（legacy/cqparse）", input: `[CQ:markdown,data={"a":1]`,
			wantText: `[CQ:markdown,data={"a":1]`, wantNoItems: true,
		},
		{
			name: "小写 [cq: 前缀仍区分", input: "[cq:image,file=https://x/a.png]",
			wantText: "[cq:image,file=https://x/a.png]", wantNoItems: true,
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			legText, legItems := unionLegacyRun(t, tc.input)
			newText, newItems, _ := unionParseRun(t, tc.input)
			if legText != tc.wantText {
				t.Fatalf("legacy 正文: got %q, want %q", legText, tc.wantText)
			}
			if newText != tc.wantText {
				t.Fatalf("cqparse 正文: got %q, want %q", newText, tc.wantText)
			}
			if tc.wantNoItems {
				if len(legItems) != 0 {
					t.Fatalf("legacy 不应产 foundItems: %v", legItems)
				}
				if len(newItems) != 0 {
					t.Fatalf("cqparse 不应产 foundItems: %v", newItems)
				}
			}
			if tc.wantKey != "" {
				lv := unionFirst(t, legItems, tc.wantKey)
				nv := unionFirst(t, newItems, tc.wantKey)
				if lv != tc.wantVal {
					t.Fatalf("legacy foundItems[%s]: got %q, want %q", tc.wantKey, lv, tc.wantVal)
				}
				if nv != tc.wantVal {
					t.Fatalf("cqparse foundItems[%s]: got %q, want %q", tc.wantKey, nv, tc.wantVal)
				}
			}
			if tc.wantInternal != nil {
				got := unionInternalRun(t, tc.input)
				if len(got) != len(tc.wantInternal) {
					t.Fatalf("internal canonical 段数: got %v, want %v", got, tc.wantInternal)
				}
				for i := range got {
					if got[i].Type != tc.wantInternal[i].Type {
						t.Fatalf("internal 段 %d Type: got %q, want %q", i, got[i].Type, tc.wantInternal[i].Type)
					}
					for k, v := range tc.wantInternal[i].Data {
						if got[i].Data[k] != v {
							t.Fatalf("internal 段 %d Data[%s]: got %q, want %q (all=%v)", i, k, got[i].Data[k], v, got[i].Data)
						}
					}
				}
			}
		})
	}
}

// TestCQUnionContractStreamDualSyntax §3 stream 双语法：冒号、等号、混写三路等价。
func TestCQUnionContractStreamDualSyntax(t *testing.T) {
	want := `{"qq":"12345","type":"start"}`
	for _, in := range []string{
		"[CQ:stream,type:start,qq:12345]",          // 冒号语法（现状）
		"[CQ:stream,type=start,qq=12345]",          // 等号语法（本轮新增）
		"[CQ:stream , TYPE = start , QQ : 12345 ]", // 混写+大小写+空白
		"[CQ:Stream,qq=12345,type=start]",          // 键序颠倒
	} {
		t.Run(in, func(t *testing.T) {
			_, legItems := unionLegacyRun(t, in)
			_, newItems, _ := unionParseRun(t, in)
			if got := unionFirst(t, legItems, "stream"); got != want {
				t.Fatalf("legacy stream: got %q, want %q", got, want)
			}
			if got := unionFirst(t, newItems, "stream"); got != want {
				t.Fatalf("cqparse stream: got %q, want %q", got, want)
			}
		})
	}
}

// TestCQUnionContractReplyIdMsgIdxCoexist 修 F6：[CQ:reply,id=5,msg_idx=x] 同现钉
// 当前实际行为——legacy reply 步与 cqparse 控制步都整码消费：id 落 reply_msg_id、
// msg_idx 随整码一并移除（入站专用剥离步对其不再单独生效），两链正文等价；
// internal 链产 reply 段（仅 id）+ 文本段，msg_idx 不保留为独立字段。
func TestCQUnionContractReplyIdMsgIdxCoexist(t *testing.T) {
	in := "[CQ:reply,id=5,msg_idx=x]文本"
	legText, legItems := unionLegacyRun(t, in)
	newText, newItems, _ := unionParseRun(t, in)
	if legText != "文本" || newText != "文本" {
		t.Fatalf("同现码应整码消费: legacy=%q cqparse=%q", legText, newText)
	}
	if got := unionFirst(t, legItems, "reply_msg_id"); got != "5" {
		t.Fatalf("legacy reply_msg_id: got %q want \"5\"", got)
	}
	if got := unionFirst(t, newItems, "reply_msg_id"); got != "5" {
		t.Fatalf("cqparse reply_msg_id: got %q want \"5\"", got)
	}
	if _, ok := legItems["msg_idx"]; ok {
		t.Fatalf("legacy 不应把 msg_idx 落独立键: %v", legItems)
	}
	want := []domainmessage.Segment{
		{Type: "reply", Data: map[string]string{"id": "5"}},
		{Type: "text", Data: map[string]string{"text": "文本"}},
	}
	got := unionInternalRun(t, in)
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("internal 同现落形变更: got %v want %v", got, want)
	}
}

// TestCQUnionContractBareRecordUnion 契约样例 [CQ: RECORD,file = my voice.amr,asr=ab]：
// 三路都按键识别到 record 段且 file 值一致（含值内部空格）；无 scheme 值的
// 落形按各链现状（legacy 透传字面 / cqparse unknown_record / internal 本地路径），
// 本轮不改动该既有分层差异。
func TestCQUnionContractBareRecordUnion(t *testing.T) {
	in := "[CQ: RECORD,file = my voice.amr,asr=ab]"

	legText, legItems := unionLegacyRun(t, in)
	if legText != in || len(legItems) != 0 {
		t.Fatalf("legacy 无 scheme 值透传现状被改动: text=%q items=%v", legText, legItems)
	}

	_, newItems, _ := unionParseRun(t, in)
	if got := unionFirst(t, newItems, "unknown_record"); got != "my voice.amr" {
		t.Fatalf("cqparse unknown_record: got %q", got)
	}

	got := unionInternalRun(t, in)
	if len(got) != 1 || got[0].Type != "record" || got[0].Data["file"] != "my voice.amr" {
		t.Fatalf("internal record: got %v", got)
	}
	if _, ok := got[0].Data["asr"]; ok {
		t.Fatalf("internal 不应把 asr 并入 file: %v", got[0].Data)
	}
}

// TestCQUnionContractAtLeadSpace §4 at 段名前空白：渲染、@全员归一与新链三路同等识别。
func TestCQUnionContractAtLeadSpace(t *testing.T) {
	const boundOpenID = "u09632587419632587419632587419632"
	virtual, err := idmap.StoreIDv2(boundOpenID)
	if err != nil {
		t.Fatalf("StoreIDv2 失败: %v", err)
	}
	boundQQ := strconv.FormatInt(virtual, 10)

	// 渲染层（markdown/文本共用 cqAtSegRe）
	in := "[CQ: at,qq=" + boundQQ + "]"
	if got := ResolveMarkdownAtMentions("前" + in + "后"); !strings.Contains(got, `<qqbot-at-user id="`+boundOpenID+`" />`) {
		t.Fatalf("段名前空白 at 未渲染: %q", got)
	}

	// @全员归一（大小写 + 段名前空白组合）
	if got := NormalizeAtEveryone("[CQ: AT , QQ = all ]"); got != atEveryoneTag {
		t.Fatalf("段名前空白 @全员未归一: %q", got)
	}

	// cqparse 词法：at 段以 passthrough 保留原文、键名小写归一
	doc := cqparse.Tokenize(in + "尾")
	if len(doc.Tokens) < 2 {
		t.Fatalf("Token 数异常: %#v", doc.Tokens)
	}
	tok := doc.Tokens[0]
	if tok.Action != "at" || tok.Kind != cqparse.KindPassthrough || tok.Params["qq"] != boundQQ {
		t.Fatalf("cqparse at: %#v", tok)
	}
	if tok.Raw != in {
		t.Fatalf("at passthrough 必须保留原文: %q", tok.Raw)
	}

	// internal 新链：段名前空白识别为 at 段
	got := unionInternalRun(t, in)
	if len(got) != 1 || got[0].Type != "at" || got[0].Data["qq"] != boundQQ {
		t.Fatalf("internal at: %v", got)
	}
}

// TestCQUnionContractMemberActionFold 动作码 member：段名/键名大小写与段名前空白
// 在 legacy 执行器与 cqparse pending 两路同等生效。
func TestCQUnionContractMemberActionFold(t *testing.T) {
	const gid = "g0123456789abcdef0123456789abcde"
	in := "[CQ: MEMBER , TYPE = remove , group_id = " + gid + " , user_id = 123 ]"

	// legacy 管道入口对动作码只透传（执行发生在 handler 发送步），三路矩阵口径相同
	legText, legItems := unionLegacyRun(t, in+"尾")
	if legText != in+"尾" || len(legItems) != 0 {
		t.Fatalf("legacy 管道对 member 动作码应原样透传: text=%q items=%v", legText, legItems)
	}

	// 执行器直驱：段名前空白+大小写形态必须命中 member 分支（键名折叠生效）
	eventID := "pre"
	out, realGroupID := ProcessOutboundCQCodes(in, "fallback", &eventID, nil)
	if out != "" {
		t.Fatalf("legacy member 码应移除: %q", out)
	}
	if realGroupID != gid {
		t.Fatalf("legacy member realGroupID: %q want %q", realGroupID, gid)
	}
	if eventID != "" {
		t.Fatalf("legacy member remove 应显式清空 eventID: %q", eventID)
	}

	// cqparse：带群上下文才产 pending（M1 私聊拦截），键名/键序归一后与 legacy 同参
	inWithGroup := cqparse.Input{Kind: cqparse.InputString, String: in, GroupID: gid, HasGroup: true}
	_, _, pps, err := cqparse.Parse(inWithGroup, &cqparse.Deps{})
	if err != nil {
		t.Fatalf("cqparse Parse: %v", err)
	}
	if len(pps) != 1 || pps[0].Action != "member" {
		t.Fatalf("cqparse pending: %+v", pps)
	}
	p := pps[0].Params
	if p["type"] != "remove" || p["group_id"] != gid || p["user_id"] != "123" {
		t.Fatalf("cqparse pending 键名/键序归一失败: %v", p)
	}
}

// TestCQUnionContractSetGroupUnknownActionLiteral 未知 set_group action 两路均保留原文
// （值口径不在放宽范围）。
func TestCQUnionContractSetGroupUnknownActionLiteral(t *testing.T) {
	const gid = "g0123456789abcdef0123456789abcde"
	in := "[CQ:Set_Group , ACTION = no_such_action , group_id = 1 ]"
	leg, legItems := unionLegacyRun(t, in)
	if leg != in {
		t.Fatalf("legacy 未知 action 应保留: %q", leg)
	}
	if len(legItems) != 0 {
		t.Fatalf("legacy 未知 action 不应产 items: %v", legItems)
	}
	// cqparse 须在群会话下断言（M1 私聊拦截会整码移除，语义不同）
	text, _, _, err := cqparse.Parse(cqparse.Input{Kind: cqparse.InputString, String: in, GroupID: gid, HasGroup: true}, &cqparse.Deps{})
	if err != nil {
		t.Fatalf("cqparse Parse: %v", err)
	}
	if text != in {
		t.Fatalf("cqparse 未知 action 应保留: %q", text)
	}
}

// TestCQUnionContractDigitNameSegmentUnion 段名字符集并集口径（统一契约 §1 收口）：
// [A-Za-z0-9_] 且首字符不限数字，四路（legacy / cqparse / internal / Rust）同等识别。
// 数字段名不是已知动作/媒体段，三路均为「识别为对应小写未知段」语义——legacy/cqparse
// 整码字面透传、internal 产对应 Type 的未知段，与 Rust 侧 parse_body 行为一致；
// 含非法字符（`!` 等）段名仍整码按文本（负例），前缀整词口径不回退（[CQ:atx] 不被 at 吞）。
func TestCQUnionContractDigitNameSegmentUnion(t *testing.T) {
	cases := []struct {
		name         string
		input        string
		wantInternal []domainmessage.Segment
	}{
		{"数字开头段名 3d", "[CQ:3d,x=1]", []domainmessage.Segment{
			{Type: "3d", Data: map[string]string{"x": "1"}},
		}},
		{"段名内含数字 h2o", "[CQ:h2o,x=1]", []domainmessage.Segment{
			{Type: "h2o", Data: map[string]string{"x": "1"}},
		}},
		{"数字段名 2048id（type 键不误伤）", "[CQ:2048id,type=1]", []domainmessage.Segment{
			{Type: "2048id", Data: map[string]string{"type": "1"}},
		}},
		// 负例：非法字符段名整码按文本，三路一致（internal 仅产原文本段）
		{"bad! 仍整码文本", "[CQ:bad!,x=1]", []domainmessage.Segment{
			{Type: "text", Data: map[string]string{"text": "[CQ:bad!,x=1]"}},
		}},
		// 负例：atx 是独立未知段名，不回退为 at 前缀吞段（F2 口径保持）
		{"atx 不被 at 吞", "[CQ:atx]", []domainmessage.Segment{
			{Type: "atx", Data: map[string]string{}},
		}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			legText, legItems := unionLegacyRun(t, tc.input)
			newText, newItems, _ := unionParseRun(t, tc.input)
			if legText != tc.input {
				t.Fatalf("legacy 正文应整码透传: got %q, want %q", legText, tc.input)
			}
			if newText != tc.input {
				t.Fatalf("cqparse 正文应整码透传: got %q, want %q", newText, tc.input)
			}
			if len(legItems) != 0 {
				t.Fatalf("legacy 未知段名不应产 foundItems: %v", legItems)
			}
			if len(newItems) != 0 {
				t.Fatalf("cqparse 未知段名不应产 foundItems: %v", newItems)
			}
			// internal 链三路对拍（段数/Type/逐键值，口径同主表）
			got := unionInternalRun(t, tc.input)
			if len(got) != len(tc.wantInternal) {
				t.Fatalf("internal canonical 段数: got %v, want %v", got, tc.wantInternal)
			}
			for i := range got {
				if got[i].Type != tc.wantInternal[i].Type {
					t.Fatalf("internal 段 %d Type: got %q, want %q", i, got[i].Type, tc.wantInternal[i].Type)
				}
				for k, v := range tc.wantInternal[i].Data {
					if got[i].Data[k] != v {
						t.Fatalf("internal 段 %d Data[%s]: got %q, want %q (all=%v)", i, k, got[i].Data[k], v, got[i].Data)
					}
				}
			}
			// legacy 动作分发步：数字段名落入 default 原样保留，不报错、不消费
			if tc.wantInternal[0].Type != "text" {
				out, _ := ProcessOutboundCQCodes(tc.input, "fallback", nil, nil)
				if out != tc.input {
					t.Fatalf("dispatch 应原样保留未知数字段名: got %q, want %q", out, tc.input)
				}
			}
		})
	}
}

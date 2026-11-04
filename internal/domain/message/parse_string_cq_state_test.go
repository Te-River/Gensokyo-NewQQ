package message

// 修 F3/F7 的 internal 侧镜像用例：
//   - 参数区花括号配平感知（对齐 Rust cqcode.rs find_code_end 状态机）：
//     JSON 值内的 `]` 不结束码、配平失败整码按文本透传；
//   - 段名字符集校验（[A-Za-z0-9_] 整词）：非法字符段名整码按文本透传，
//     且失败位置只前进一个字节重扫，不吞后方合法码；
//   - case 表补 stream 双语法：冒号、等号、混写同形（allowColon 仅 stream 段生效）。

import (
	"reflect"
	"strings"
	"testing"
)

func TestParseOneBotStringBraceBalance(t *testing.T) {
	cases := []struct {
		name string
		in   string
		want []Segment
	}{
		{"JSON 值内 ] 不结束码", `[CQ:markdown,data={"template":"hello ] world"}]`,
			[]Segment{{Type: "markdown", Data: map[string]string{"data": `{"template":"hello ] world"}`}}}},
		{"JSON 值内 ] 后仍有正文", `前[CQ:markdown,data={"a":"x]y"}]后`,
			[]Segment{
				{Type: "text", Data: map[string]string{"text": "前"}},
				{Type: "markdown", Data: map[string]string{"data": `{"a":"x]y"}`}},
				{Type: "text", Data: map[string]string{"text": "后"}},
			}},
		// 配平失败（JSON 未闭合即遇串尾）：整码按文本透传，与 legacy/cqparse 同口径
		{"配平失败整码透传", `[CQ:markdown,data={"a":1]`,
			[]Segment{{Type: "text", Data: map[string]string{"text": `[CQ:markdown,data={"a":1]`}}}},		// 键与 `=` 之间空白后接 `{`：JSON 值起点判定不被空白重置（同 F4 口径）
		{"`=` 后空白再接 { 仍配平", `[CQ:markdown , data = {"a":"x]"}]`,
			[]Segment{{Type: "markdown", Data: map[string]string{"data": `{"a":"x]"}`}}}},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			pm, err := ParseOneBotString(c.in)
			if err != nil {
				t.Fatalf("ParseOneBotString: %v", err)
			}
			got := Canonicalize(pm)
			if !reflect.DeepEqual(got, c.want) {
				t.Errorf("\n got %#v\nwant %#v", got, c.want)
			}
		})
	}
}

func TestParseOneBotStringSegmentNameCharset(t *testing.T) {
	cases := []struct {
		name string
		in   string
		want []Segment
	}{
		// 段名以非法字符起：整码按文本透传（不产 UnknownPart 非法段名）
		{"段名前全角空格整码透传", "[CQ:　image,file=x.png]",
			[]Segment{{Type: "text", Data: map[string]string{"text": "[CQ:　image,file=x.png]"}}}},
		// 数字段名字符合法（对齐 Rust find_code_end [a-zA-Z0-9_]+）
		{"段名含数字为未知段", "[CQ:face2 , id = 1]",
			[]Segment{{Type: "face2", Data: map[string]string{"id": "1"}}}},
		// 失败位置只前进一个字节重扫：后方合法码照常识别（不吞内嵌码）
		{"非法段名不吞后方码", "[CQ:image\v,file=x][CQ:image,file=https://x/a.png]",
			[]Segment{
				{Type: "text", Data: map[string]string{"text": "[CQ:image\v,file=x]"}},
				{Type: "image", Data: map[string]string{"file": "https://x/a.png"}},
			}},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			pm, err := ParseOneBotString(c.in)
			if err != nil {
				t.Fatalf("ParseOneBotString: %v", err)
			}
			got := Canonicalize(pm)
			if !reflect.DeepEqual(got, c.want) {
				t.Errorf("\n got %#v\nwant %#v", got, c.want)
			}
		})
	}
}

func TestParseOneBotStringStreamDualSyntax(t *testing.T) {
	want := []Segment{{Type: "stream", Data: map[string]string{"type": "start", "qq": "12345"}}}
	for _, in := range []string{
		"[CQ:stream,type:start,qq:12345]",          // 冒号语法
		"[CQ:stream,type=start,qq=12345]",          // 等号语法
		"[CQ:Stream , TYPE = start , QQ : 12345 ]", // 混写+大小写+空白（Type 保留原文段名）
		"[CQ:stream,qq=12345,type=start]",          // 键序颠倒
	} {
		t.Run(in, func(t *testing.T) {
			pm, err := ParseOneBotString(in)
			if err != nil {
				t.Fatalf("ParseOneBotString: %v", err)
			}
			got := Canonicalize(pm)
			if len(got) != 1 || strings.ToLower(got[0].Type) != want[0].Type {
				t.Fatalf("stream 段落形: %v", got)
			}
			for k, v := range want[0].Data {
				if got[0].Data[k] != v {
					t.Fatalf("stream Data[%s]: got %q want %q (all=%v)", k, got[0].Data[k], v, got[0].Data)
				}
			}
			if in == "[CQ:Stream , TYPE = start , QQ : 12345 ]" && got[0].Type != "Stream" {
				t.Fatalf("未知/保留段名 Type 应保原文: %v", got[0])
			}
		})
	}

	// 冒号兜底只对 stream 生效：其它段内无 `=` 的段维持现状跳过
	pm, err := ParseOneBotString("[CQ:face , id : 1]")
	if err != nil {
		t.Fatalf("ParseOneBotString: %v", err)
	}
	got := Canonicalize(pm)
	if len(got) != 1 || got[0].Type != "face" || len(got[0].Data) != 0 {
		t.Fatalf("非 stream 段不应走冒号语法: %v", got)
	}
}

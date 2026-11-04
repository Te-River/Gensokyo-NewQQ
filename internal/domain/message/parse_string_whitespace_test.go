package message

// 统一空白契约（§1~§3/§7）在新链字符串解析器上的镜像用例：
// 段名后、`,` 或 `]` 之前允许任意空白；键与 `=` 两侧允许空白；键 trim、
// 值仅 trim 首尾、内部空白保留；键值参数按 key 取值（顺序无关）。
// 大小写口径维持现状：段名与键名区分大小写，本文件末组用例把现状钉死。

import (
	"reflect"
	"testing"
)

func TestParseOneBotStringWhitespaceTolerance(t *testing.T) {
	cases := []struct {
		name string
		in   string
		want []Segment
	}{
		{"image 空白", "[CQ:image , file = https://example.com/a.png]",
			[]Segment{{Type: "image", Data: map[string]string{"file": "https://example.com/a.png"}}}},
		{"record 值内空格保留", "[CQ:record,file = my voice.amr]",
			[]Segment{{Type: "record", Data: map[string]string{"file": "my voice.amr"}}}},
		{"at qq 非首参（§7）", "[CQ:at,name=x,qq=123]",
			[]Segment{{Type: "at", Data: map[string]string{"qq": "123"}}}},
		{"at 全空白+乱序", "[CQ:at , name = x , qq = 123 ]",
			[]Segment{{Type: "at", Data: map[string]string{"qq": "123"}}}},
		{"face 空白", "[CQ:face , id = 1]",
			[]Segment{{Type: "face", Data: map[string]string{"id": "1"}}}},
		{"reply 空白", "[CQ:reply,id = 123]",
			[]Segment{{Type: "reply", Data: map[string]string{"id": "123"}}}},
		{"card 空白", `[CQ:card , title=A, desc=B]`,
			[]Segment{{Type: "card", Data: map[string]string{"title": "A", "desc": "B"}}}},
		{"markdown 空白", "[CQ:markdown , data = base64://e30=]",
			[]Segment{{Type: "markdown", Data: map[string]string{"data": "base64://e30="}}}},
		{"music 空白（id 按键取值）", "[CQ:music , type = qq, id = 123]",
			[]Segment{{Type: "qqmusic", Data: map[string]string{"id": "123"}}}},
		{"转义逗号+空白", "[CQ:image , file = http://x.com/a&#44;b.png]",
			[]Segment{{Type: "image", Data: map[string]string{"file": "http://x.com/a,b.png"}}}},
		// G-A：段名后空白为 ASCII 五元集（含 \f），与 legacy 正则 `\s` 同口径；键侧 \f 走 Unicode TrimSpace 现状。
		{"段名后 \\f（五元集）", "[CQ:image\f , file = https://example.com/a.png]",
			[]Segment{{Type: "image", Data: map[string]string{"file": "https://example.com/a.png"}}}},
		{"逗号后 \\f（键层现状）", "[CQ:image,\ffile=https://example.com/a.png]",
			[]Segment{{Type: "image", Data: map[string]string{"file": "https://example.com/a.png"}}}},
		// G-A 钉现状：\v 与全角空格不在五元集内（Go 正则 `\s` 亦不含），段名不识别回 UnknownPart。
		{"段名后 \\v 不识别（现状）", "[CQ:image\v,file=x.png]",
			[]Segment{{Type: "image\v", Data: map[string]string{"file": "x.png"}}}},
		{"段名后全角空格不识别（现状）", "[CQ:image　,file=x.png]",
			[]Segment{{Type: "image　", Data: map[string]string{"file": "x.png"}}}},
		// 现状口径：段名/键名区分大小写（不顺手放宽）。
		// 新链对未识别段名的现状是 UnknownPart（键值仍按键解析），legacy 字符串链才是保留字面文本。
		{"段名大写不识别（现状）", "[CQ:Image , File = https://x/a.png]",
			[]Segment{{Type: "Image", Data: map[string]string{"File": "https://x/a.png"}}}},
		{"键名大写不识别（现状）", "[CQ:image , File = https://x/a.png]",
			[]Segment{{Type: "image", Data: map[string]string{"file": ""}}}},
		// 现状口径：畸形段（无 ] ）保持按文本
		{"未闭合保持文本", "hi[CQ:image,file=x.png",
			[]Segment{
				{Type: "text", Data: map[string]string{"text": "hi"}},
				{Type: "text", Data: map[string]string{"text": "[CQ:image,file=x.png"}},
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

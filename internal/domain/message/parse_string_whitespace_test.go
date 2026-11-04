package message

// 统一最大兼容契约（§1~§4/§7）在新链字符串解析器上的镜像用例：
// 段名前（`[CQ:` 后）与段名之后、`,` 或 `]` 之前允许任意空白；键与 `=` 两侧允许空白；
// 键 trim、值仅 trim 首尾、内部空白保留；键值参数按 key 取值（顺序无关）；
// 段名与键名大小写不敏感（`[CQ:` 前缀字面仍区分大小写）。

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
		// F7 翻转：\v 与全角空格不在五元集内、也不在段名字符集 [A-Za-z0-9_] 内，
		// 段名整词不成立 → 整码按文本透传（不再产 UnknownPart 非法段名）。
		{"段名后 \\v 整码按文本透传", "[CQ:image\v,file=x.png]",
			[]Segment{{Type: "text", Data: map[string]string{"text": "[CQ:image\v,file=x.png]"}}}},
		{"段名后全角空格整码按文本透传", "[CQ:image　,file=x.png]",
			[]Segment{{Type: "text", Data: map[string]string{"text": "[CQ:image　,file=x.png]"}}}},
		// 统一契约 §1/§4：段名与键名大小写不敏感、段名前允许空白（`[CQ:` 前缀字面不变）。
		// 新链对未知段名产出 UnknownPart（Type 保留原文），legacy 字符串链保留字面文本。
		{"段名大写同样识别", "[CQ:Image , File = https://x/a.png]",
			[]Segment{{Type: "image", Data: map[string]string{"file": "https://x/a.png"}}}},
		{"键名大写同样识别", "[CQ:image , File = https://x/a.png]",
			[]Segment{{Type: "image", Data: map[string]string{"file": "https://x/a.png"}}}},
		{"段名前空白同样识别", "[CQ: image , FILE = https://x/a.png]",
			[]Segment{{Type: "image", Data: map[string]string{"file": "https://x/a.png"}}}},
		{"键序颠倒 RECORD+ASR", "[CQ: RECORD , ASR = ab , File = http://x/a.wav]",
			[]Segment{{Type: "record", Data: map[string]string{"file": "http://x/a.wav"}}}},
		{"未知段名大写键名归小写", "[CQ:face , ID = 1]",
			[]Segment{{Type: "face", Data: map[string]string{"id": "1"}}}},
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

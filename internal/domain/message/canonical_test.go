package message

import (
	"reflect"
	"testing"
)

// TestStringAndArrayConsistency 验证 String 与 Array 两种入口输出同一 ParsedMessage 模型。
// 同一语义的输入，canonicalize 后必须一致。
func TestStringAndArrayConsistency(t *testing.T) {
	cases := []struct {
		name   string
		str    string
		segs   []Segment
	}{
		{
			"text+at+image",
			"你好[CQ:at,qq=123][CQ:image,file=https://x.com/a.png]",
			[]Segment{
				{Type: "text", Data: map[string]string{"text": "你好"}},
				{Type: "at", Data: map[string]string{"qq": "123"}},
				{Type: "image", Data: map[string]string{"file": "https://x.com/a.png"}},
			},
		},
		{
			"reply+record",
			"[CQ:reply,id=1][CQ:record,file=base64://e30=]",
			[]Segment{
				{Type: "reply", Data: map[string]string{"id": "1"}},
				{Type: "record", Data: map[string]string{"file": "base64://e30="}},
			},
		},
		{
			"empty",
			"",
			nil,
		},
	}

	for _, c := range cases {
		pmStr, err := ParseOneBotString(c.str)
		if err != nil {
			t.Fatalf("%s: ParseOneBotString: %v", c.name, err)
		}
		pmArr, err := ParseOneBotSegments(c.segs)
		if err != nil {
			t.Fatalf("%s: ParseOneBotSegments: %v", c.name, err)
		}
		gotStr := Canonicalize(pmStr)
		gotArr := Canonicalize(pmArr)
		if !reflect.DeepEqual(gotStr, gotArr) {
			t.Errorf("%s: string(%#v) != array(%#v)", c.name, gotStr, gotArr)
		}
	}
}

func TestToLegacyFoundItems(t *testing.T) {
	pm, err := ParseOneBotString("[CQ:reply,id=9]hi[CQ:image,file=https://x.com/a.png]")
	if err != nil {
		t.Fatal(err)
	}
	text, found := pm.ToLegacyFoundItems()
	if text != "hi" {
		t.Fatalf("text = %q, want hi", text)
	}
	if got := found["reply_msg_id"]; len(got) != 1 || got[0] != "9" {
		t.Fatalf("reply_msg_id = %v", got)
	}
	if got := found["url_image"]; len(got) != 1 || got[0] != "https://x.com/a.png" {
		t.Fatalf("url_image = %v", got)
	}
}

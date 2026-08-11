package message

import (
	"reflect"
	"testing"
)

func TestParseOneBotString(t *testing.T) {
	cases := []struct {
		name string
		in   string
		want []Segment
	}{
		{"plain text", "你好世界", []Segment{{Type: "text", Data: map[string]string{"text": "你好世界"}}}},
		{"image url", "[CQ:image,file=https://x.com/a.png]",
			[]Segment{{Type: "image", Data: map[string]string{"file": "https://x.com/a.png"}}}},
		{"image base64", "[CQ:image,file=base64://QUJD]",
			[]Segment{{Type: "image", Data: map[string]string{"file": "base64://QUJD"}}}},
		{"record", "[CQ:record,file=base64://e30=]",
			[]Segment{{Type: "record", Data: map[string]string{"file": "base64://e30="}}}},
		{"video", "[CQ:video,file=http://x.com/v.mp4]",
			[]Segment{{Type: "video", Data: map[string]string{"file": "http://x.com/v.mp4"}}}},
		{"file", "[CQ:file,file=https://x.com/f.zip,name=doc.zip]",
			[]Segment{{Type: "file", Data: map[string]string{"file": "https://x.com/f.zip", "name": "doc.zip"}}}},
		{"at", "[CQ:at,qq=123456]",
			[]Segment{{Type: "at", Data: map[string]string{"qq": "123456"}}}},
		{"reply", "[CQ:reply,id=42]",
			[]Segment{{Type: "reply", Data: map[string]string{"id": "42"}}}},
		{"markdown", "[CQ:markdown,data=base64://e30=]",
			[]Segment{{Type: "markdown", Data: map[string]string{"data": "base64://e30="}}}},
		{"keyboard", "[CQ:keyboard,data={\"k\":1}]",
			[]Segment{{Type: "keyboard", Data: map[string]string{"data": "{\"k\":1}"}}}},
		{"qqmusic", "[CQ:music,type=qq,id=123]",
			[]Segment{{Type: "qqmusic", Data: map[string]string{"id": "123"}}}},
		{"mixed", "你好[CQ:image,file=http://x.com/a.png]再见",
			[]Segment{
				{Type: "text", Data: map[string]string{"text": "你好"}},
				{Type: "image", Data: map[string]string{"file": "http://x.com/a.png"}},
				{Type: "text", Data: map[string]string{"text": "再见"}},
			}},
		{"escaped comma in file", "[CQ:image,file=http://x.com/a&#44;b.png]",
			[]Segment{{Type: "image", Data: map[string]string{"file": "http://x.com/a,b.png"}}}},
		{"text cq", "[CQ:text,text=hello&#44;world]",
			[]Segment{{Type: "text", Data: map[string]string{"text": "hello,world"}}}},
		{"malformed no close", "hi[CQ:image,file=x.png",
			[]Segment{
				{Type: "text", Data: map[string]string{"text": "hi"}},
				{Type: "text", Data: map[string]string{"text": "[CQ:image,file=x.png"}},
			}},
		{"empty", "", nil},
		{"bare text", "普通文本", []Segment{{Type: "text", Data: map[string]string{"text": "普通文本"}}}},
	}

	for _, c := range cases {
		pm, err := ParseOneBotString(c.in)
		if err != nil {
			t.Fatalf("%s: ParseOneBotString: %v", c.name, err)
		}
		got := Canonicalize(pm)
		if !reflect.DeepEqual(got, c.want) {
			t.Errorf("%s:\n got %#v\nwant %#v", c.name, got, c.want)
		}
	}
}

func TestParseReplyExtracted(t *testing.T) {
	pm, err := ParseOneBotString("[CQ:reply,id=99]内容")
	if err != nil {
		t.Fatal(err)
	}
	if pm.Reply == nil || pm.Reply.MessageID != "99" {
		t.Fatalf("Reply = %+v, want 99", pm.Reply)
	}
}

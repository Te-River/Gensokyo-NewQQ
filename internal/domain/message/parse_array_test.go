package message

import (
	"reflect"
	"testing"
)

func TestParseOneBotSegments(t *testing.T) {
	segs := []Segment{
		{Type: "text", Data: map[string]string{"text": "你好"}},
		{Type: "at", Data: map[string]string{"qq": "123"}},
		{Type: "image", Data: map[string]string{"file": "https://x.com/a.png"}},
	}
	pm, err := ParseOneBotSegments(segs)
	if err != nil {
		t.Fatalf("ParseOneBotSegments: %v", err)
	}
	got := Canonicalize(pm)
	if !reflect.DeepEqual(got, segs) {
		t.Fatalf("got %#v\nwant %#v", got, segs)
	}
}

func TestParseSegmentsReplyExtracted(t *testing.T) {
	pm, err := ParseOneBotSegments([]Segment{{Type: "reply", Data: map[string]string{"id": "7"}}})
	if err != nil {
		t.Fatal(err)
	}
	if pm.Reply == nil || pm.Reply.MessageID != "7" {
		t.Fatalf("Reply = %+v", pm.Reply)
	}
}

func TestSegmentFromMap(t *testing.T) {
	seg, ok := FromMap(map[string]interface{}{
		"type": "image",
		"data": map[string]interface{}{"file": "https://x.com/a.png", "subType": 1},
	})
	if !ok {
		t.Fatal("FromMap failed")
	}
	if seg.Type != "image" || seg.Data["file"] != "https://x.com/a.png" || seg.Data["subType"] != "1" {
		t.Fatalf("segment = %+v", seg)
	}
}

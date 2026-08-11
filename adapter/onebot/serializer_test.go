package onebot

import (
	"testing"

	"github.com/hoshinonyaruko/gensokyo/internal/domain/message"
)

func TestSerializeString(t *testing.T) {
	pm, err := message.ParseOneBotString("你好[CQ:at,qq=123][CQ:image,file=http://x.com/a.png]")
	if err != nil {
		t.Fatal(err)
	}
	got := SerializeString(pm)
	want := "你好[CQ:at,qq=123][CQ:image,file=http://x.com/a.png]"
	if got != want {
		t.Fatalf("got %q\nwant %q", got, want)
	}
}

func TestSerializeStringEscapes(t *testing.T) {
	pm, err := message.ParseOneBotString("[CQ:image,file=http://x.com/a&#44;b.png]")
	if err != nil {
		t.Fatal(err)
	}
	got := SerializeString(pm)
	// 反转义后的 file 含逗号，序列化时应重新转义
	want := "[CQ:image,file=http://x.com/a&#44;b.png]"
	if got != want {
		t.Fatalf("got %q\nwant %q", got, want)
	}
}

func TestSerializeArray(t *testing.T) {
	pm, err := message.ParseOneBotString("hi[CQ:at,qq=9]")
	if err != nil {
		t.Fatal(err)
	}
	arr := SerializeArray(pm)
	if len(arr) != 2 {
		t.Fatalf("len = %d, want 2", len(arr))
	}
	if arr[0]["type"] != "text" || arr[0]["data"].(map[string]interface{})["text"] != "hi" {
		t.Fatalf("seg0 = %#v", arr[0])
	}
	if arr[1]["type"] != "at" {
		t.Fatalf("seg1 type = %v", arr[1]["type"])
	}
}

func TestSerializeStringArrayConsistent(t *testing.T) {
	// string 序列化后重新解析，应与原 ParsedMessage canonical 一致（round-trip）
	orig := "文本[CQ:image,file=https://x.com/a.png]再见"
	pm, err := message.ParseOneBotString(orig)
	if err != nil {
		t.Fatal(err)
	}
	s := SerializeString(pm)
	pm2, err := message.ParseOneBotString(s)
	if err != nil {
		t.Fatalf("re-parse: %v", err)
	}
	got := message.Canonicalize(pm2)
	want := message.Canonicalize(pm)
	if len(got) != len(want) {
		t.Fatalf("round-trip mismatch:\n got %#v\nwant %#v", got, want)
	}
}

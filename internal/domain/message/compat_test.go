package message

import "testing"

func TestMediaSourceFromFile(t *testing.T) {
	cases := []struct {
		file string
		kind MediaKind
	}{
		{"base64://QUJD", MediaBase64},
		{"http://x.com/a.png", MediaRemoteURL},
		{"https://x.com/a.png", MediaRemoteURL},
		{"file:///tmp/a.png", MediaLocalFile},
		{"a.png", MediaLocalFile},
	}
	for _, c := range cases {
		ms := MediaSourceFromFile(c.file)
		if ms.Kind != c.kind {
			t.Errorf("MediaSourceFromFile(%q).Kind = %v, want %v", c.file, ms.Kind, c.kind)
		}
	}
}

func TestToLegacyFoundItemsAllTypes(t *testing.T) {
	segs := []Segment{
		{Type: "text", Data: map[string]string{"text": "hello"}},
		{Type: "at", Data: map[string]string{"qq": "123"}},
		{Type: "image", Data: map[string]string{"file": "http://x.com/a.png"}},
		{Type: "image", Data: map[string]string{"file": "base64://QUJD"}},
		{Type: "image", Data: map[string]string{"file": "/tmp/local.png"}},
		{Type: "record", Data: map[string]string{"file": "https://x.com/r.mp3"}},
		{Type: "video", Data: map[string]string{"file": "http://x.com/v.mp4"}},
		{Type: "file", Data: map[string]string{"file": "https://x.com/f.zip", "name": "d.zip"}},
		{Type: "reply", Data: map[string]string{"id": "1"}},
		{Type: "markdown", Data: map[string]string{"data": "md"}},
		{Type: "keyboard", Data: map[string]string{"data": "kb"}},
		{Type: "qqmusic", Data: map[string]string{"id": "2"}},
		{Type: "face", Data: map[string]string{"id": "3"}},
	}
	pm, err := ParseOneBotSegments(segs)
	if err != nil {
		t.Fatalf("ParseOneBotSegments: %v", err)
	}
	text, found := pm.ToLegacyFoundItems()

	if text != "hello[CQ:at,qq=123]" {
		t.Fatalf("text = %q", text)
	}
	assertFound := func(key, want string) {
		t.Helper()
		got := found[key]
		if len(got) != 1 || got[0] != want {
			t.Errorf("found[%q] = %v, want [%q]", key, got, want)
		}
	}
	assertFound("url_image", "http://x.com/a.png")
	assertFound("base64_image", "QUJD")
	assertFound("local_image", "/tmp/local.png")
	assertFound("url_record", "https://x.com/r.mp3")
	assertFound("url_video", "http://x.com/v.mp4")
	assertFound("url_file", "https://x.com/f.zip")
	assertFound("file_name", "d.zip")
	assertFound("reply_msg_id", "1")
	assertFound("markdown", "md")
	assertFound("keyboard", "kb")
	assertFound("qqmusic", "2")
	if len(found["unknown_face"]) == 0 {
		t.Errorf("unknown_face missing: %v", found)
	}
}

func TestUnknownPartCanonicalizePreservesParams(t *testing.T) {
	pm, err := ParseOneBotString("[CQ:face,id=1]")
	if err != nil {
		t.Fatal(err)
	}
	got := Canonicalize(pm)
	if len(got) != 1 || got[0].Type != "face" || got[0].Data["id"] != "1" {
		t.Fatalf("canonical = %+v", got)
	}
}

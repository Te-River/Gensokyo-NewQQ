package message

import "testing"

func TestEscapeCQ(t *testing.T) {
	if got := EscapeCQ("a,b[c]&"); got != "a&#44;b&#91;c&#93;&amp;" {
		t.Fatalf("EscapeCQ = %q", got)
	}
}

func TestUnescapeCQ(t *testing.T) {
	if got := UnescapeCQ("a&#44;b&#91;c&#93;&amp;"); got != "a,b[c]&" {
		t.Fatalf("UnescapeCQ = %q", got)
	}
}

func TestParseCQParams(t *testing.T) {
	m := parseCQParams("file=base64://abc,id=42")
	if m["file"] != "base64://abc" || m["id"] != "42" {
		t.Fatalf("parseCQParams = %v", m)
	}
}

func TestParseCQParamsEscapedComma(t *testing.T) {
	// value 内的逗号是转义后的 &#44;，不应拆分
	m := parseCQParams("text=hello&#44;world,id=1")
	if m["text"] != "hello,world" {
		t.Fatalf("escaped comma param = %q", m["text"])
	}
	if m["id"] != "1" {
		t.Fatalf("id = %q", m["id"])
	}
}

func TestParseCQParamsEmpty(t *testing.T) {
	if m := parseCQParams(""); len(m) != 0 {
		t.Fatalf("empty params = %v", m)
	}
}

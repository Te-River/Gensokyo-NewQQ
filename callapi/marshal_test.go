package callapi

import (
	"encoding/json"
	"strings"
	"testing"
)

// 回执序列化必须与 Rust 端逐字节一致：带签名的 URL 中 & 原样输出，不转义成 \u0026
func TestMarshalNoEscapeKeepsAmpersand(t *testing.T) {
	raw, err := MarshalNoEscape(map[string]interface{}{
		"retcode": 0,
		"data":    map[string]interface{}{"avatar": "https://example.com/a?x=1&y=2"},
	})
	if err != nil {
		t.Fatalf("MarshalNoEscape 返回错误: %v", err)
	}
	got := string(raw)
	if !strings.Contains(got, "https://example.com/a?x=1&y=2") {
		t.Errorf("URL 未被原样输出: %s", got)
	}
	if strings.Contains(got, `\u0026`) {
		t.Errorf("& 被转义成 \\u0026: %s", got)
	}
	if strings.Contains(got, "\n") {
		t.Errorf("不应残留 Encoder 的尾随换行: %q", got)
	}

	// 关闭转义不影响 JSON 语义
	var back map[string]interface{}
	if err := json.Unmarshal(raw, &back); err != nil {
		t.Fatalf("输出不是合法 JSON: %v (%s)", err, got)
	}
	avatar, _ := back["data"].(map[string]interface{})["avatar"].(string)
	if avatar != "https://example.com/a?x=1&y=2" {
		t.Errorf("往返后 avatar = %q", avatar)
	}
}

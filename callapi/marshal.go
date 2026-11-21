package callapi

import (
	"bytes"
	"encoding/json"
)

// MarshalNoEscape 序列化发往 OneBot 客户端的内容：关闭 HTML 转义（& < > 原样输出）。
// json.Marshal 默认把 &、<、> 转义成 \u0026/\u003c/\u003e，带签名的头像/图床/短链 URL
// 会因此与 Rust 端逐字节不一致；关闭后仍是合法 JSON，解析语义不变。
// 末尾换行已裁掉，与 json.Marshal 的输出逐字节对齐。
func MarshalNoEscape(v interface{}) ([]byte, error) {
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	if err := enc.Encode(v); err != nil {
		return nil, err
	}
	return bytes.TrimRight(buf.Bytes(), "\n"), nil
}

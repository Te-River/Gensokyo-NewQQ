package callapi

import (
	"encoding/json"
	"testing"
)

// file 传对象/数组时不得让整包 Unmarshal 失败（老客户端兼容），归一为字符串。
func TestParamsContentFileAcceptsNonString(t *testing.T) {
	cases := []struct {
		name string
		raw  string
		want string
	}{
		{"字符串", `{"file":"base64://abc"}`, "base64://abc"},
		{"对象", `{"file":{"path":"/tmp/a.png"}}`, `{"path":"/tmp/a.png"}`},
		{"数组", `{"file":[1,2]}`, `[1,2]`},
		{"数字", `{"file":123}`, "123"},
		{"布尔", `{"file":true}`, "true"},
		{"null", `{"file":null}`, ""},
		{"缺失", `{}`, ""},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			var p ParamsContent
			if err := json.Unmarshal([]byte(tc.raw), &p); err != nil {
				t.Fatalf("不应 Unmarshal 失败: %v", err)
			}
			if p.File != tc.want {
				t.Fatalf("file = %q, want %q", p.File, tc.want)
			}
		})
	}

	// 其他字段不受影响：同一包里 group_id 仍按原逻辑归一
	var p ParamsContent
	if err := json.Unmarshal([]byte(`{"group_id":123,"file":{"a":1},"file_name":"a.png"}`), &p); err != nil {
		t.Fatalf("复合请求不应失败: %v", err)
	}
	if p.GroupID != "123" || p.FileName != "a.png" {
		t.Fatalf("其余字段解析异常: %+v", p)
	}
}

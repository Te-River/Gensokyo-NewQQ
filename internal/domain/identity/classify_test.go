package identity

import "testing"

func TestIsOpenID(t *testing.T) {
	openID := "01234567890123456789012345678901" // 32 位
	cases := []struct {
		name string
		in   string
		want bool
	}{
		{"32-char openid", openID, true},
		{"empty", "", false},
		{"short", "abc", false},
		{"numeric virtual id", "123456789", false},
		{"31-char", "0123456789012345678901234567890", false},
		{"33-char", "012345678901234567890123456789012", false},
	}
	for _, c := range cases {
		if got := IsOpenID(c.in); got != c.want {
			t.Errorf("%s: IsOpenID(%q) = %v, want %v", c.name, c.in, got, c.want)
		}
	}
}

func TestIsVirtualID(t *testing.T) {
	cases := []struct {
		name string
		in   string
		want bool
	}{
		{"numeric", "123456789", true},
		{"zero", "0", true},
		{"empty", "", false},
		// 32 位全数字字符串会同时命中 IsOpenID 与 IsVirtualID（legacy 启发式重叠），
		// 此处用非纯数字的 OpenID 验证区分
		{"openid", "a1234567890123456789012345678901", false},
		{"negative", "-1", false},
		{"mixed", "12a3", false},
	}
	for _, c := range cases {
		if got := IsVirtualID(c.in); got != c.want {
			t.Errorf("%s: IsVirtualID(%q) = %v, want %v", c.name, c.in, got, c.want)
		}
	}
}

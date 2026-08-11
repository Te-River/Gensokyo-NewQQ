package identity

import (
	"testing"
)

func TestOpenIDString(t *testing.T) {
	const raw = "01234567890123456789012345678901"
	if got := OpenID(raw).String(); got != raw {
		t.Fatalf("OpenID.String() = %q, want %q", got, raw)
	}
}

func TestVirtualIDInt64(t *testing.T) {
	v := VirtualUserIDFromInt64(123456789)
	if v.String() != "123456789" {
		t.Fatalf("VirtualUserIDFromInt64 = %q", v)
	}
	got, err := v.Int64()
	if err != nil || got != 123456789 {
		t.Fatalf("Int64() = %d, %v", got, err)
	}
}

func TestVirtualIDInt64Invalid(t *testing.T) {
	if _, err := VirtualUserID("abc").Int64(); err == nil {
		t.Fatal("Int64() accepted non-numeric virtual ID")
	}
}

func TestUINAndAppID(t *testing.T) {
	uin, err := UINFromString("10001")
	if err != nil || uin.Uint64() != 10001 {
		t.Fatalf("UINFromString = %d, %v", uin, err)
	}
	appID, err := AppIDFromString("12345")
	if err != nil || appID.Uint64() != 12345 {
		t.Fatalf("AppIDFromString = %d, %v", appID, err)
	}
	if _, err := AppIDFromString("not-a-number"); err == nil {
		t.Fatal("AppIDFromString accepted non-numeric input")
	}
}

func TestVirtualGroupID(t *testing.T) {
	v := VirtualGroupIDFromInt64(42)
	if v.String() != "42" {
		t.Fatalf("VirtualGroupIDFromInt64 = %q", v)
	}
}

func TestDistinctTypesDoNotAlias(t *testing.T) {
	// 编译期类型区分：OpenID 与 VirtualUserID 即使同字符串也不能直接混用
	var open OpenID = "123"
	var virtual VirtualUserID = "123"
	if string(open) != string(virtual) {
		t.Fatal("sanity check failed")
	}
	// 复制与比较必须是同类型
	clone := open
	if clone != open {
		t.Fatal("OpenID copy comparison failed")
	}
}

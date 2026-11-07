package imagehosting

import (
	"strconv"
	"testing"
	"time"
)

// TestNaturePathSuffixAlways8Hex 回归：低位纳秒值曾因 "%x" 后直接切 [:8] 越界 panic。
// 后缀必须恒为 8 位十六进制。
func TestNaturePathSuffixAlways8Hex(t *testing.T) {
	values := []int64{
		0,
		1,
		0xf,
		0x10000000 - 1, // 旧实现下长度 7 的临界点
		0x10000000,
		0xffffffff,
		time.Now().UnixNano(),
	}
	for _, nano := range values {
		got := naturePathSuffix(nano)
		if len(got) != 8 {
			t.Fatalf("naturePathSuffix(%d) = %q，长度 %d，期望 8", nano, got, len(got))
		}
		if _, err := strconv.ParseUint(got, 16, 64); err != nil {
			t.Fatalf("naturePathSuffix(%d) = %q 不是合法十六进制: %v", nano, got, err)
		}
	}
}

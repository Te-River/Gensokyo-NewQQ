package qq

import (
	"context"
	"errors"
	"testing"
)

// timeoutErr 模拟 net.Error 超时（非 context.DeadlineExceeded）。
type timeoutErr struct{}

func (timeoutErr) Error() string   { return "i/o timeout" }
func (timeoutErr) Timeout() bool   { return true }
func (timeoutErr) Temporary() bool { return true }

// TestClassifierRetryability 钉死 classifier.go 的可重试判定：
// 仅超时/网络类与 legacy 富媒体上传超时串可重试，业务错误码不可重试。
func TestClassifierRetryability(t *testing.T) {
	c := NewClassifier()
	cases := []struct {
		name string
		err  error
		want bool
	}{
		{"nil error", nil, false},
		{"context deadline exceeded", context.DeadlineExceeded, true},
		{"net timeout", timeoutErr{}, true},
		{"rich media upload timeout string", errors.New("富媒体文件上传超时: upload"), true},
		{"deadline phrase without wrapped sentinel", errors.New("PostGroupMessage: context deadline exceeded"), true},
		{"business error code", errors.New("code:22009"), false},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := c.Classify(tc.err).Retryable; got != tc.want {
				t.Fatalf("Classify(%v).Retryable = %v, want %v", tc.err, got, tc.want)
			}
		})
	}
}

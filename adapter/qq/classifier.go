package qq

import (
	"context"
	"errors"
	"net"
	"strings"

	"github.com/hoshinonyaruko/gensokyo/internal/application/outbound"
)

// Classifier 是 outbound.ErrorClassifier 的 QQ 实现：
// 仅把超时/网络类错误标记为可重试，其余（业务错误码等）不可重试。
type Classifier struct{}

// NewClassifier 创建错误分类器。
func NewClassifier() *Classifier { return &Classifier{} }

// Classify 实现 outbound.ErrorClassifier。
func (c *Classifier) Classify(err error) outbound.ErrorClass {
	if err == nil {
		return outbound.ErrorClass{}
	}
	if errors.Is(err, context.DeadlineExceeded) {
		return outbound.ErrorClass{Retryable: true}
	}
	var ne net.Error
	if errors.As(err, &ne) && ne.Timeout() {
		return outbound.ErrorClass{Retryable: true}
	}
	// legacy retry_policy.go:36-38：富媒体上传超时以字符串标记（非 net.Error）
	if strings.Contains(err.Error(), "富媒体文件上传超时") {
		return outbound.ErrorClass{Retryable: true}
	}
	// legacy retry_policy.go:26 同款字符串判定：botgo/底层 HTTP 未包装 sentinel 时，
	// errors.Is 漏判，错误文本里已含该短语（如 "PostGroupMessage: context deadline exceeded"）。
	if strings.Contains(err.Error(), "context deadline exceeded") {
		return outbound.ErrorClass{Retryable: true}
	}
	return outbound.ErrorClass{}
}

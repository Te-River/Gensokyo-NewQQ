package outbound

import "time"

// ErrorClass 发送错误分类（由 ErrorClassifier 提供，解耦具体 QQ 错误码）。
type ErrorClass struct {
	// Retryable 是否可重试（如超时）。
	Retryable bool
	// RateLimited 是否被限流。
	RateLimited bool
	// Expired 事件/消息是否已过期。
	Expired bool
}

// ErrorClassifier 把发送错误分类为可重试/限流/过期等。
// 具体实现（对接 QQ 错误码）由 adapter 提供，Application 不感知。
type ErrorClassifier interface {
	Classify(err error) ErrorClass
}

// RetryPolicy 发送重试策略。
type RetryPolicy struct {
	MaxAttempts int
	Backoff     func(attempt int) time.Duration
	Classify    ErrorClassifier
}

// DefaultRetryPolicy 返回默认策略（超时重试 3 次，线性退避）。
func DefaultRetryPolicy(classify ErrorClassifier) RetryPolicy {
	return RetryPolicy{
		MaxAttempts: 3,
		Backoff: func(attempt int) time.Duration {
			if attempt <= 0 {
				return 0
			}
			return time.Duration(attempt) * time.Second
		},
		Classify: classify,
	}
}

// ShouldRetry 判断第 attempt 次（从 0 开始）失败后是否重试。
func (p RetryPolicy) ShouldRetry(err error, attempt int) bool {
	if err == nil || p.Classify == nil {
		return false
	}
	if attempt+1 >= p.MaxAttempts {
		return false
	}
	return p.Classify.Classify(err).Retryable
}


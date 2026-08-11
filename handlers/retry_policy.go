package handlers

import (
	"context"
	"errors"
	"strings"
	"time"
)

// QQErrorClass 是发送路径共享的错误分类结果。
type QQErrorClass struct {
	Code         int
	Timeout      bool
	RateLimited  bool
	EventExpired bool
}

func ClassifyQQError(err error) QQErrorClass {
	if err == nil {
		return QQErrorClass{}
	}
	errText := err.Error()
	code := ExtractQQErrorCode(errText)
	return QQErrorClass{
		Code:         code,
		Timeout:      errors.Is(err, context.DeadlineExceeded) || strings.Contains(errText, "context deadline exceeded") || strings.Contains(errText, "富媒体文件上传超时"),
		RateLimited:  code == 22009,
		EventExpired: code == 40034025 || code == 40034026,
	}
}

func IsQQError(err error, code int) bool {
	return err != nil && ClassifyQQError(err).Code == code
}

func IsDeliveryTimeout(err error) bool {
	return ClassifyQQError(err).Timeout
}

func IsQQRateLimited(err error) bool {
	return ClassifyQQError(err).RateLimited
}

func IsQQEventExpired(err error) bool {
	return ClassifyQQError(err).EventExpired
}

// RetryPolicy 描述发送类重试的次数和退避策略。
type RetryPolicy struct {
	MaxAttempts int
	Backoff     func(attempt int) time.Duration
}

var defaultRetryPolicy = RetryPolicy{
	MaxAttempts: 3,
	Backoff: func(attempt int) time.Duration {
		if attempt <= 0 {
			return 0
		}
		return time.Duration(attempt) * time.Second
	},
}

func (p RetryPolicy) ShouldRetry(err error, attempt int) bool {
	return attempt+1 < p.MaxAttempts && IsDeliveryTimeout(err)
}

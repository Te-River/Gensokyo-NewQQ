package messagequeue

import (
	"context"
	"sync"
	"time"

	"github.com/hoshinonyaruko/gensokyo/mylog"
	"golang.org/x/time/rate"
)

var (
	globalLimiter *RateLimiter
	limiterOnce   sync.Once
)

// RateLimiter 基于令牌桶的 API 调用频率限制
type RateLimiter struct {
	limiter  *rate.Limiter
	burstCnt int
}

// GetRateLimiter 获取全局频率限制器
func GetRateLimiter() *RateLimiter {
	limiterOnce.Do(func() {
		// 默认每秒 5 个请求，突发 10 个
		// QQ 官方 API 限制约为 5qps
		globalLimiter = &RateLimiter{
			limiter:  rate.NewLimiter(rate.Limit(5), 10),
			burstCnt: 10,
		}
	})
	return globalLimiter
}

// Wait 等待直到可以发送下一个请求
func (rl *RateLimiter) Wait(ctx context.Context) error {
	return rl.limiter.Wait(ctx)
}

// Allow 检查是否允许立即发送
func (rl *RateLimiter) Allow() bool {
	return rl.limiter.Allow()
}

// SetRate 动态调整速率
func (rl *RateLimiter) SetRate(qps float64) {
	rl.limiter.SetLimit(rate.Limit(qps))
	mylog.Printf("[限流] 调整 QPS 为: %.1f", qps)
}

// SetBurst 动态调整突发量
func (rl *RateLimiter) SetBurst(burst int) {
	rl.limiter.SetBurst(burst)
	rl.burstCnt = burst
}

// Stats 返回当前限流器状态
func (rl *RateLimiter) Stats() (float64, int) {
	return float64(rl.limiter.Limit()), rl.burstCnt
}

// WaitWithTimeout 带超时的等待
func (rl *RateLimiter) WaitWithTimeout(timeout time.Duration) bool {
	ctx, cancel := context.WithTimeout(context.Background(), timeout)
	defer cancel()
	err := rl.limiter.Wait(ctx)
	return err == nil
}

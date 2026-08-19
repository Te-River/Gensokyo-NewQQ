// Package manager 实现 session manager 所需要的公共方法。
package manager

import (
	"math"
	"time"

	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/errs"
)

// CanNotResumeErrSet 不能进行 resume 操作的错误码
var CanNotResumeErrSet = map[int]bool{
	errs.CodeConnCloseCantResume: true,
}

// CanNotIdentifyErrSet 不能进行 identify 操作的错误码
var CanNotIdentifyErrSet = map[int]bool{
	errs.CodeConnCloseCantIdentify: true,
}

// concurrencyTimeWindowSec 并发时间窗口，单位秒
const concurrencyTimeWindowSec = 2

// MaxAuthRetryCount 连续鉴权失败达到上限后，自动从头整轮重试的最大轮数。
// 每轮仍会触发 token 强刷与重新连接/鉴权；达到该上限后仍未成功才真正停止重连，
// 用于覆盖服务器停电重启等瞬时抖动，同时避免密钥真正错误时无限重连。
const MaxAuthRetryCount = 5

// CalcInterval 根据并发要求，计算连接启动间隔
func CalcInterval(maxConcurrency uint32) time.Duration {
	if maxConcurrency == 0 {
		maxConcurrency = 1
	}
	f := math.Round(concurrencyTimeWindowSec / float64(maxConcurrency))
	if f == 0 {
		f = 1
	}
	return time.Duration(f) * time.Second
}

// CanNotResume 是否是不能够 resume 的错误
func CanNotResume(err error) bool {
	e := errs.Error(err)
	if flag, ok := CanNotResumeErrSet[e.Code()]; ok {
		return flag
	}
	return false
}

// CanNotIdentify 是否是不能够 identify 的错误
func CanNotIdentify(err error) bool {
	e := errs.Error(err)
	if flag, ok := CanNotIdentifyErrSet[e.Code()]; ok {
		return flag
	}
	return false
}

// CheckSessionLimit 检查链接数是否达到限制，如果达到限制需要等待重置
func CheckSessionLimit(apInfo *dto.WebsocketAP) error {
	if apInfo.Shards > apInfo.SessionStartLimit.Remaining {
		return errs.ErrSessionLimit
	}
	return nil
}

// CheckSessionLimit 检查链接数是否达到限制，如果达到限制需要等待重置
func CheckSessionLimitSingle(apInfo *dto.WebsocketAPSingle) error {
	if apInfo.ShardCount > apInfo.SessionStartLimit.Remaining {
		return errs.ErrSessionLimit
	}
	return nil
}

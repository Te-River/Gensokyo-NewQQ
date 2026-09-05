package Processor

import (
	"math"

	"github.com/hoshinonyaruko/gensokyo/mylog"
)

// safeMessageID 将 int64 虚拟消息 ID 安全转换为 int；
// 超出 int 位宽（仅 32 位构建可能）时记录日志并返回 -1。
// -1 的选择依据：虚拟 ID 池从 1 起顺序分配、0 是解绑残留哨兵、负值永不分配，
// 因此 -1 不会与真实 ID 撞车，避免截断产生错误 ID（与 M5-B 的 -1 惯例一致）。
// 64 位构建下比较恒为假，零开销直通；仅 32 位构建会实际拦截。
func safeMessageID(v int64) int {
	if v > int64(math.MaxInt) || v < int64(math.MinInt) {
		mylog.Errorf("消息 ID 超出 int 位宽: %d", v)
		return -1
	}
	return int(v)
}

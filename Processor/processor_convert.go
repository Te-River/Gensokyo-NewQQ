package Processor

import (
	"math"

	"github.com/hoshinonyaruko/gensokyo/mylog"
)

// safeMessageID 将 int64 虚拟消息 ID 安全转换为 int；
// 虚拟 msg_id 为顺序分配计数器，int32 上界（约 21.4 亿）远超实际规模，
// 统一按 int32 边界收紧以兼容 32 位 int 构建并满足静态检查；
// 越界时记录日志并返回 -1（虚拟池从 1 起分配、0 为解绑残留哨兵、负值永不分配，不会与真实 ID 撞车）。
func safeMessageID(v int64) int {
	if v < math.MinInt32 || v > math.MaxInt32 {
		mylog.Errorf("消息 ID 超出安全位宽: %d", v)
		return -1
	}
	return int(v)
}

package handlers

import (
	"context"

	"github.com/hoshinonyaruko/gensokyo/echo"
	appstate "github.com/hoshinonyaruko/gensokyo/internal/application/state"
)

// 本文件是 D 包（state 接缝）的桥：把 echo 仓储实现注入 handlers。
// new 路径的 msgseq 递增经 nextMappingSeq 走注入的 SequenceRepository；
// legacy 仍直调 echo（两者底层同为 echo.NextMappingSeq，结果一致）。

var (
	seqRepo appstate.SequenceRepository
	ctxRepo appstate.MessageContextRepository
)

// SetStateRepositories 注入 state 仓储（main.go bootstrap 调用）。
func SetStateRepositories(seq appstate.SequenceRepository, ctx appstate.MessageContextRepository) {
	seqRepo = seq
	ctxRepo = ctx
}

// SequenceRepository 返回当前注入的序列仓储（诊断/测试用）。
func SequenceRepository() appstate.SequenceRepository { return seqRepo }

// MessageContextRepository 返回当前注入的消息上下文仓储（诊断/测试用）。
func MessageContextRepository() appstate.MessageContextRepository { return ctxRepo }

// nextMappingSeq 原子递增并返回 msgseq。
// arch_mode=new（非 legacy）且仓储已注入时经 SequenceRepository.Next；
// 否则回退 echo.IncrementMappingSeq。两者底层同为 echo.NextMappingSeq，结果逐字节一致。
func nextMappingSeq(key string) int {
	if ArchMode() != "legacy" {
		if repo := SequenceRepository(); repo != nil {
			if v, err := repo.Next(context.Background(), key); err == nil {
				return int(v)
			}
		}
	}
	return echo.IncrementMappingSeq(key)
}

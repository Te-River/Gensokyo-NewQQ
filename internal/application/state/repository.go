// Package state 提供有明确 owner 与生命周期的仓储接口（idmap/echo 的目标形态）。
//
// 目标：拆分职责（Identity / MessageContext / Sequence），msgseq 只允许原子 Next，
// 统一 TTL，清理走显式 Start/Close（禁止 package init 启动 goroutine）。
// 与现有 idmap / echo 全局状态双轨并存，接入属 P13。
package state

import (
	"context"
	"errors"
	"time"

	"github.com/hoshinonyaruko/gensokyo/internal/domain/identity"
)

// 仓储错误。
var (
	// ErrNotFound 条目不存在或已过期。
	ErrNotFound = errors.New("state: not found")
)

// IdentityRepository 身份映射仓储。
// 复用 identity.IdentityResolver（P3）：实现方（adapter）是 string↔typed 的转换边界。
type IdentityRepository = identity.IdentityResolver

// SequenceRepository 原子序列仓储（msgseq 等）。
// 只提供原子 Next，禁止重新暴露 Get+Set 给业务层组合。
type SequenceRepository interface {
	Next(ctx context.Context, key string) (uint32, error)
}

// Entry 消息上下文条目（带 owner 与生命周期）。
type Entry struct {
	Owner     string
	Value     string
	CreatedAt time.Time
	ExpiresAt time.Time
}

// MessageContextRepository 消息上下文仓储（message id / event id / reply 临时映射）。
// 查询必须校验 owner 与过期（P10.5/P10.6）。
type MessageContextRepository interface {
	// Get 按 owner+key 查询；owner 不匹配或已过期返回 ErrNotFound。
	Get(ctx context.Context, owner, key string) (string, error)
	// Set 写入带 owner 与 TTL 的条目。
	Set(ctx context.Context, owner, key, value string, ttl time.Duration) error
	// Delete 删除条目（仅限 owner 名下）。
	Delete(ctx context.Context, owner, key string) error
}

// Cleaner 生命周期管理（禁止 init() 启动 ticker）。
type Cleaner interface {
	// Start 启动后台清理（通常以 goroutine 运行）。
	Start(ctx context.Context)
	// Close 停止后台清理。
	Close()
}

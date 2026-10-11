// Package staterepo 基于现有 echo 全局状态实现 state 仓储接口。
//
// 这是 echo 全局 map 与 internal/application/state 契约之间的适配层：
// 业务层依赖 SequenceRepository / MessageContextRepository，不再直接触碰 echo 全局函数。
// owner 语义映射到 echo 的 appid 维度（echo 键格式为 appid + "_" + key）。
package staterepo

import (
	"context"
	"sync"
	"time"

	"github.com/hoshinonyaruko/gensokyo/echo"
	appstate "github.com/hoshinonyaruko/gensokyo/internal/application/state"
)

// EchoSequenceRepository 基于 echo 全局 seq map 的原子序列仓储。
type EchoSequenceRepository struct{}

// NewEchoSequenceRepository 创建 echo 序列仓储。
func NewEchoSequenceRepository() *EchoSequenceRepository { return &EchoSequenceRepository{} }

// Next 原子递增并返回下一个 seq（包 echo.NextMappingSeq）。
func (r *EchoSequenceRepository) Next(_ context.Context, key string) (uint32, error) {
	return uint32(echo.NextMappingSeq(key)), nil
}

// contextMeta 记录条目的过期时间与删除墓碑。
// echo 的 msgID map 无 TTL、无 Delete，故由本适配层补充。
type contextMeta struct {
	expiresAt time.Time
	deleted   bool
}

// EchoContextRepository 基于 echo msgID map 的消息上下文仓储。
// 值存 echo（键 = owner + "_" + key，owner 即 appid 维度），
// TTL 与删除由本适配层的 meta 表补充。
type EchoContextRepository struct {
	meta sync.Map // ownerKey -> contextMeta
}

// NewEchoContextRepository 创建 echo 上下文仓储。
func NewEchoContextRepository() *EchoContextRepository { return &EchoContextRepository{} }

// ownerKey 生成适配层内部键；用 NUL 分隔，避免与 echo 的 "_" 键格式混淆。
func ownerKey(owner, key string) string { return owner + "\x00" + key }

// Get 按 owner+key 查询；owner 不匹配、已删除或已过期返回 ErrNotFound。
func (r *EchoContextRepository) Get(_ context.Context, owner, key string) (string, error) {
	if m, ok := r.meta.Load(ownerKey(owner, key)); ok {
		meta := m.(contextMeta)
		if meta.deleted {
			return "", appstate.ErrNotFound
		}
		if !meta.expiresAt.IsZero() && time.Now().After(meta.expiresAt) {
			return "", appstate.ErrNotFound
		}
	}
	v := echo.GetMsgIDv3(owner, key)
	if v == "" {
		return "", appstate.ErrNotFound
	}
	return v, nil
}

// Set 写入带 owner 与 TTL 的条目；ttl<=0 表示不过期。
func (r *EchoContextRepository) Set(_ context.Context, owner, key, value string, ttl time.Duration) error {
	echo.AddMsgIDv3(owner, key, value)
	var exp time.Time
	if ttl > 0 {
		exp = time.Now().Add(ttl)
	}
	r.meta.Store(ownerKey(owner, key), contextMeta{expiresAt: exp})
	return nil
}

// Delete 删除条目（仅限 owner 名下）。
func (r *EchoContextRepository) Delete(_ context.Context, owner, key string) error {
	r.meta.Store(ownerKey(owner, key), contextMeta{deleted: true})
	return nil
}

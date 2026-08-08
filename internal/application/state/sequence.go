package state

import (
	"context"
	"sync"
	"time"
)

// TTL 统一配置（P10.6）：业务引用这些常量，禁止各 map 硬编码。
const (
	// TTLShort 短生命周期条目。
	TTLShort = 5 * time.Minute
	// TTLMedium 中生命周期条目。
	TTLMedium = 30 * time.Minute
	// TTLDefault 默认生命周期。
	TTLDefault = time.Hour
)

// MemSequenceRepository 内存原子序列实现（单进程）。
type MemSequenceRepository struct {
	mu   sync.Mutex
	seqs map[string]uint32
}

// NewMemSequenceRepository 创建内存序列仓储。
func NewMemSequenceRepository() *MemSequenceRepository {
	return &MemSequenceRepository{seqs: map[string]uint32{}}
}

// Next 原子递增并返回下一个序列值（从 1 开始）。
func (m *MemSequenceRepository) Next(_ context.Context, key string) (uint32, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.seqs[key]++
	return m.seqs[key], nil
}

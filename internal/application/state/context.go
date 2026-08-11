package state

import (
	"context"
	"sync"
	"time"
)

// MemContextRepository 内存消息上下文仓储（单进程）。
// 条目按 owner+key 隔离，查询校验 owner 与过期。
type MemContextRepository struct {
	mu      sync.Mutex
	entries map[string]Entry
	now     func() time.Time

	done chan struct{}
	once sync.Once
}

// NewMemContextRepository 创建内存上下文仓储。
func NewMemContextRepository() *MemContextRepository {
	return &MemContextRepository{
		entries: map[string]Entry{},
		now:     time.Now,
		done:    make(chan struct{}),
	}
}

func ownerKey(owner, key string) string { return owner + "\x00" + key }

// Get 按 owner+key 查询；owner 不匹配或已过期返回 ErrNotFound。
func (m *MemContextRepository) Get(_ context.Context, owner, key string) (string, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	e, ok := m.entries[ownerKey(owner, key)]
	if !ok || m.now().After(e.ExpiresAt) {
		if ok {
			delete(m.entries, ownerKey(owner, key))
		}
		return "", ErrNotFound
	}
	return e.Value, nil
}

// Set 写入带 owner 与 TTL 的条目。
func (m *MemContextRepository) Set(_ context.Context, owner, key, value string, ttl time.Duration) error {
	now := m.now()
	m.mu.Lock()
	defer m.mu.Unlock()
	m.entries[ownerKey(owner, key)] = Entry{
		Owner:     owner,
		Value:     value,
		CreatedAt: now,
		ExpiresAt: now.Add(ttl),
	}
	return nil
}

// Delete 删除条目（仅限 owner 名下）。
func (m *MemContextRepository) Delete(_ context.Context, owner, key string) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	delete(m.entries, ownerKey(owner, key))
	return nil
}

// Start 启动定期清理过期条目（P10.7：显式 Start，禁止 init 启动）。
func (m *MemContextRepository) Start(ctx context.Context) {
	go func() {
		ticker := time.NewTicker(TTLMedium)
		defer ticker.Stop()
		for {
			select {
			case <-ctx.Done():
				m.Close()
				return
			case <-ticker.C:
				m.cleanupExpired()
			}
		}
	}()
}

// Close 停止清理。
func (m *MemContextRepository) Close() {
	m.once.Do(func() { close(m.done) })
}

func (m *MemContextRepository) cleanupExpired() {
	now := m.now()
	m.mu.Lock()
	defer m.mu.Unlock()
	for k, e := range m.entries {
		if now.After(e.ExpiresAt) {
			delete(m.entries, k)
		}
	}
}

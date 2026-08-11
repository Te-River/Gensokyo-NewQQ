package config

import (
	"context"
	"sync"
	"time"
)

// Snapshot 是配置的不可变快照。
// 通过 Config() 获取运行时配置（slice 字段已防御性拷贝），禁止直接修改。
type Snapshot struct {
	config   RuntimeConfig
	loadedAt time.Time
	version  uint64
}

// NewSnapshot 从已校验的 DTO 构建不可变快照。
func NewSnapshot(dto ConfigDTO) *Snapshot {
	return &Snapshot{
		config:   buildRuntime(dto),
		loadedAt: time.Now(),
		version:  uint64(dto.Version),
	}
}

// Config 返回运行时配置的深拷贝（slice 字段独立底层数组）。
func (s Snapshot) Config() RuntimeConfig { return s.config.clone() }

// LoadedAt 快照加载时间。
func (s Snapshot) LoadedAt() time.Time { return s.loadedAt }

// Version 配置 schema 版本。
func (s Snapshot) Version() uint64 { return s.version }

// BuildSnapshot 完整管线：迁移 → 解码 DTO → 校验 → 构建快照。
func BuildSnapshot(data []byte) (*Snapshot, error) {
	root, err := ParseNode(data)
	if err != nil {
		return nil, err
	}
	if err := Migrate(root, CurrentSchemaVersion); err != nil {
		return nil, err
	}
	var dto ConfigDTO
	if err := root.Decode(&dto); err != nil {
		return nil, newParseError(err)
	}
	if err := Validate(dto); err != nil {
		return nil, err
	}
	return NewSnapshot(dto), nil
}

// LoadSnapshot 从文件读取并执行完整管线。
func LoadSnapshot(path string) (*Snapshot, error) {
	data, err := readFile(path)
	if err != nil {
		return nil, newIOError(err)
	}
	return BuildSnapshot(data)
}

// Manager 持有当前快照，提供"失败保留旧快照"的重载与监听。
type Manager struct {
	path string
	mu   sync.RWMutex
	cur  *Snapshot
}

// NewManager 创建配置管理器。
func NewManager(path string) *Manager { return &Manager{path: path} }

// Load 初始加载并构建快照。
func (m *Manager) Load() error {
	snap, err := LoadSnapshot(m.path)
	if err != nil {
		return err
	}
	m.mu.Lock()
	m.cur = snap
	m.mu.Unlock()
	return nil
}

// Reload 重新加载；失败时保留旧快照（不置零、不崩溃、不覆盖有效配置）。
func (m *Manager) Reload() error {
	snap, err := LoadSnapshot(m.path)
	if err != nil {
		return err
	}
	m.mu.Lock()
	m.cur = snap
	m.mu.Unlock()
	return nil
}

// Snapshot 返回当前快照的拷贝。
func (m *Manager) Snapshot() Snapshot {
	m.mu.RLock()
	defer m.mu.RUnlock()
	if m.cur == nil {
		return Snapshot{}
	}
	return *m.cur
}

// Watch 启动带 debounce 的配置监听并自动重载。
// debounce 建议 200~500ms；ctx 取消时停止。
func (m *Manager) Watch(ctx context.Context, debounce time.Duration) error {
	w, err := NewWatcher(m.path, debounce, m.Reload)
	if err != nil {
		return err
	}
	go w.Start(ctx)
	return nil
}

package config

import (
	"context"
	"time"

	"github.com/hoshinonyaruko/gensokyo/internal/application/media"
	"github.com/hoshinonyaruko/gensokyo/internal/application/outbound"
)

// defaultWatchDebounce 配置监听去抖窗口（设计建议 200~500ms）。
const defaultWatchDebounce = 300 * time.Millisecond

// Bootstrap 加载配置并启动热重载监听，返回持有当前快照的 Manager。
// 失败时返回错误（调用方决定是否降级）；成功后 Manager 始终持有有效快照。
func Bootstrap(path string) (*Manager, error) {
	m := NewManager(path)
	if err := m.Load(); err != nil {
		return nil, err
	}
	// 监听生命周期与进程一致：签名无 ctx，使用 Background。
	if err := m.Watch(context.Background(), defaultWatchDebounce); err != nil {
		return nil, err
	}
	return m, nil
}

// OutboundConfigFrom 从快照提取出站服务子配置。
// FallbackToPassive 映射 lazy_message_id：开启后主动消息失败可回退被动补发（SSM）。
func OutboundConfigFrom(s Snapshot) outbound.OutboundConfig {
	return outbound.OutboundConfig{
		FallbackToPassive: s.Config().OneBot.LazyMessageId,
	}
}

// MediaPolicyFrom 从快照提取媒体策略：以保守默认值为基线，
// 若配置了 image_sizelimit（KB，>0）则作为媒体字节硬上限。
func MediaPolicyFrom(s Snapshot) media.MediaPolicy {
	p := media.DefaultPolicy()
	if kb := s.Config().Media.ImageLimit; kb > 0 {
		p.MaxBytes = int64(kb) << 10
	}
	return p
}

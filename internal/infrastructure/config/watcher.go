package config

import (
	"context"
	"log"
	"time"

	"github.com/fsnotify/fsnotify"
)

// Watcher 监听配置文件变化，以 debounce 合并事件风暴后触发一次重载。
type Watcher struct {
	path     string
	debounce time.Duration
	reload   func() error
	w        *fsnotify.Watcher
}

// NewWatcher 创建监听器并立即开始监听（同步 Add，保证 Start 前事件不丢失）。
func NewWatcher(path string, debounce time.Duration, reload func() error) (*Watcher, error) {
	w, err := fsnotify.NewWatcher()
	if err != nil {
		return nil, err
	}
	if err := w.Add(path); err != nil {
		w.Close()
		return nil, err
	}
	return &Watcher{path: path, debounce: debounce, reload: reload, w: w}, nil
}

// Start 运行监听循环（阻塞），通常以 goroutine 运行；ctx 取消时停止。
func (wt *Watcher) Start(ctx context.Context) {
	var timer *time.Timer
	var tick <-chan time.Time

	fire := func() {
		tick = nil
		if wt.reload == nil {
			return
		}
		if err := wt.reload(); err != nil {
			log.Printf("config watcher: reload failed, keeping old snapshot: %v", err)
		}
	}

	for {
		select {
		case <-ctx.Done():
			wt.Close()
			return
		case _, ok := <-wt.w.Events:
			if !ok {
				return
			}
			// debounce：每次事件重置计时器
			if timer == nil {
				timer = time.NewTimer(wt.debounce)
			} else {
				timer.Reset(wt.debounce)
			}
			tick = timer.C
		case <-tick:
			fire()
		case _, ok := <-wt.w.Errors:
			if !ok {
				return
			}
		}
	}
}

// Close 关闭底层监听。
func (wt *Watcher) Close() { _ = wt.w.Close() }

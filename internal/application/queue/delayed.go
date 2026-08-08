package queue

import (
	"sync"
	"time"
)

// scheduler 延迟调度器：任务到期后执行，避免 worker 被 Sleep 占用（P7.5）。
type scheduler struct {
	mu     sync.Mutex
	timers map[uint64]*time.Timer
	seq    uint64
	closed bool
}

func newScheduler() *scheduler {
	return &scheduler{timers: map[uint64]*time.Timer{}}
}

// After 在 d 后执行 fn（若调度器已关闭则忽略）。
func (s *scheduler) After(d time.Duration, fn func()) {
	s.mu.Lock()
	if s.closed {
		s.mu.Unlock()
		return
	}
	s.seq++
	id := s.seq
	t := time.AfterFunc(d, func() {
		fn()
		s.mu.Lock()
		delete(s.timers, id)
		s.mu.Unlock()
	})
	s.timers[id] = t
	s.mu.Unlock()
}

// Close 停止所有待执行定时器。
func (s *scheduler) Close() {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.closed = true
	for _, t := range s.timers {
		t.Stop()
	}
	s.timers = map[uint64]*time.Timer{}
}

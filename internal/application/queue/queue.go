// Package queue 提供有界并发队列基础设施。
//
// 目标：无限 goroutine → 有界并发；同 session（群/用户）消息保持顺序；
// 重试走 delay scheduler 而非占用 worker Sleep；容量/拒绝/深度均有明确指标。
//
// 与现有 messagequeue 双轨并存，接入生产属 P13。
package queue

import (
	"context"
	"errors"
	"hash/fnv"
	"sync"
	"sync/atomic"
	"time"
)

// 队列错误。
var (
	// ErrQueueFull 队列已满（BackpressureReject 时返回）。
	ErrQueueFull = errors.New("queue: full")
	// ErrQueueClosed 队列已关闭。
	ErrQueueClosed = errors.New("queue: closed")
)

// Backpressure 背压策略（必须显式选择，禁止静默丢）。
type Backpressure uint8

const (
	// BackpressureBlock 阻塞直到有空位（或 ctx 取消）。
	BackpressureBlock Backpressure = iota
	// BackpressureDrop 满时丢弃（metrics.Rejected 计数）。
	BackpressureDrop
	// BackpressureReject 满时返回 ErrQueueFull。
	BackpressureReject
)

// Task 队列任务。
type Task struct {
	ID      string
	Session string // session key：同 session 顺序执行（hash → partition → worker）
	Payload interface{}
}

// Metrics 队列指标快照。
type Metrics struct {
	Capacity  int
	Depth     int
	Rejected  int64
	Processed int64
	Active    int64
}

// ProcessFunc 处理任务；返回 (是否需要重试, 退避时长)。
// 重试通过 delay scheduler 调度，不阻塞 worker。
type ProcessFunc func(ctx context.Context, task Task) (retry bool, backoff time.Duration)

// Queue 有界队列接口。
type Queue interface {
	Enqueue(ctx context.Context, task Task) error
	Close()
	Wait()
	Depth() int
	Metrics() Metrics
}

// Config 队列配置。
type Config struct {
	Capacity     int // 总容量（均分到分区）
	Workers      int // 分区/worker 数
	Backpressure Backpressure
}

// New 创建有界队列并启动 worker。
func New(cfg Config, process ProcessFunc) Queue {
	if cfg.Capacity <= 0 {
		cfg.Capacity = 1024
	}
	if cfg.Workers <= 0 {
		cfg.Workers = 8
	}
	if process == nil {
		process = func(context.Context, Task) (bool, time.Duration) { return false, 0 }
	}

	per := cfg.Capacity / cfg.Workers
	if per < 1 {
		per = 1
	}
	q := &boundedQueue{
		cfg:        cfg,
		partitions: make([]chan Task, cfg.Workers),
		process:    process,
		sched:      newScheduler(),
	}
	for i := range q.partitions {
		q.partitions[i] = make(chan Task, per)
	}
	q.start()
	return q
}

// ---- 实现 ----

type boundedQueue struct {
	cfg        Config
	partitions []chan Task
	process    ProcessFunc
	sched      *scheduler

	wg     sync.WaitGroup
	closed atomic.Bool

	rejected  atomic.Int64
	processed atomic.Int64
	active    atomic.Int64
}

func (q *boundedQueue) start() {
	for i := 0; i < q.cfg.Workers; i++ {
		p := q.partitions[i]
		q.wg.Add(1)
		go func() {
			defer q.wg.Done()
			for task := range p {
				q.active.Add(1)
				retry, backoff := q.process(context.Background(), task)
				q.active.Add(-1)
				q.processed.Add(1)
				if retry {
					// 重试不占 worker：交给 delay scheduler 到期后重新入队
					t := task
					q.sched.After(backoff, func() { _ = q.Enqueue(context.Background(), t) })
				}
			}
		}()
	}
}

// partition 按 session hash 选择分区，保证同 session 顺序。
func (q *boundedQueue) partition(session string) chan Task {
	h := fnv.New32a()
	h.Write([]byte(session))
	return q.partitions[int(h.Sum32())%q.cfg.Workers]
}

func (q *boundedQueue) Enqueue(ctx context.Context, task Task) error {
	if q.closed.Load() {
		return ErrQueueClosed
	}
	if task.Session == "" {
		task.Session = task.ID
	}
	p := q.partition(task.Session)

	switch q.cfg.Backpressure {
	case BackpressureDrop:
		select {
		case p <- task:
		default:
			q.rejected.Add(1)
		}
		return nil
	case BackpressureReject:
		select {
		case p <- task:
			return nil
		default:
			q.rejected.Add(1)
			return ErrQueueFull
		}
	default: // Block
		select {
		case p <- task:
			return nil
		case <-ctx.Done():
			return ctx.Err()
		}
	}
}

func (q *boundedQueue) Close() {
	if q.closed.Swap(true) {
		return
	}
	q.sched.Close()
	for _, p := range q.partitions {
		close(p)
	}
}

func (q *boundedQueue) Wait() { q.wg.Wait() }

func (q *boundedQueue) Depth() int {
	n := 0
	for _, p := range q.partitions {
		n += len(p)
	}
	return n
}

func (q *boundedQueue) Metrics() Metrics {
	return Metrics{
		Capacity:  q.cfg.Capacity,
		Depth:     q.Depth(),
		Rejected:  q.rejected.Load(),
		Processed: q.processed.Load(),
		Active:    q.active.Load(),
	}
}

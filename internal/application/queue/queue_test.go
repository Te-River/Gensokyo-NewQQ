package queue

import (
	"context"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

func processOK(_ context.Context, _ Task) (bool, time.Duration) { return false, 0 }

func TestEnqueueRejectWhenFull(t *testing.T) {
	// 单 worker、容量 1，且 worker 阻塞不放行 → 第二次 Enqueue 应 Reject
	q := New(Config{Capacity: 1, Workers: 1, Backpressure: BackpressureReject},
		func(context.Context, Task) (bool, time.Duration) { time.Sleep(100 * time.Millisecond); return false, 0 })

	if err := q.Enqueue(context.Background(), Task{ID: "a", Session: "s1"}); err != nil {
		t.Fatalf("first enqueue: %v", err)
	}
	err := q.Enqueue(context.Background(), Task{ID: "b", Session: "s1"})
	if err != ErrQueueFull {
		t.Fatalf("second enqueue err = %v, want ErrQueueFull", err)
	}
	if m := q.Metrics(); m.Rejected != 1 {
		t.Fatalf("Rejected = %d, want 1", m.Rejected)
	}
	q.Close()
	q.Wait()
}

func TestEnqueueDropWhenFull(t *testing.T) {
	q := New(Config{Capacity: 1, Workers: 1, Backpressure: BackpressureDrop},
		func(context.Context, Task) (bool, time.Duration) { time.Sleep(100 * time.Millisecond); return false, 0 })

	_ = q.Enqueue(context.Background(), Task{ID: "a", Session: "s1"})
	if err := q.Enqueue(context.Background(), Task{ID: "b", Session: "s1"}); err != nil {
		t.Fatalf("drop enqueue should not error: %v", err)
	}
	if m := q.Metrics(); m.Rejected != 1 {
		t.Fatalf("Rejected = %d, want 1", m.Rejected)
	}
	q.Close()
	q.Wait()
}

func TestEnqueueBlock(t *testing.T) {
	q := New(Config{Capacity: 1, Workers: 1, Backpressure: BackpressureBlock},
		func(context.Context, Task) (bool, time.Duration) { time.Sleep(50 * time.Millisecond); return false, 0 })

	_ = q.Enqueue(context.Background(), Task{ID: "a", Session: "s1"})
	// Block：第二个入队应阻塞直到有空间（worker 消费后）
	done := make(chan error, 1)
	go func() {
		done <- q.Enqueue(context.Background(), Task{ID: "b", Session: "s1"})
	}()
	select {
	case err := <-done:
		if err != nil {
			t.Fatalf("block enqueue: %v", err)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("block enqueue did not unblock")
	}
	q.Close()
	q.Wait()
}

func TestSessionOrdering(t *testing.T) {
	var mu sync.Mutex
	var order []string
	q := New(Config{Capacity: 100, Workers: 4, Backpressure: BackpressureBlock},
		func(_ context.Context, task Task) (bool, time.Duration) {
			mu.Lock()
			order = append(order, task.ID)
			mu.Unlock()
			time.Sleep(5 * time.Millisecond)
			return false, 0
		})

	// 同一 session 顺序入队
	for i := 0; i < 20; i++ {
		_ = q.Enqueue(context.Background(), Task{ID: string(rune('a' + i)), Session: "same-session"})
	}
	q.Close()
	q.Wait()

	mu.Lock()
	defer mu.Unlock()
	for i := 1; i < len(order); i++ {
		if order[i] <= order[i-1] {
			t.Fatalf("session order violated: %v", order)
		}
	}
}

func TestEnqueueAfterClose(t *testing.T) {
	q := New(Config{Capacity: 10, Workers: 1, Backpressure: BackpressureReject}, processOK)
	q.Close()
	q.Wait()
	if err := q.Enqueue(context.Background(), Task{ID: "a", Session: "s"}); err != ErrQueueClosed {
		t.Fatalf("enqueue after close err = %v, want ErrQueueClosed", err)
	}
}

func TestRetryScheduledNotBlockingWorker(t *testing.T) {
	var retried atomic.Int64
	q := New(Config{Capacity: 10, Workers: 1, Backpressure: BackpressureBlock},
		func(_ context.Context, task Task) (bool, time.Duration) {
			if retried.Load() == 0 && task.ID == "a" {
				retried.Add(1)
				return true, 50 * time.Millisecond // 重试，走 scheduler
			}
			return false, 0
		})

	_ = q.Enqueue(context.Background(), Task{ID: "a", Session: "s1"})
	// worker 不应被 retry 阻塞：第二个任务应很快被处理
	done := make(chan struct{})
	go func() {
		_ = q.Enqueue(context.Background(), Task{ID: "b", Session: "s1"})
		close(done)
	}()

	deadline := time.After(2 * time.Second)
	select {
	case <-done:
	case <-deadline:
		t.Fatal("second task blocked by retry sleep")
	}
	// 等待 scheduler 重试完成
	time.Sleep(150 * time.Millisecond)
	q.Close()
	q.Wait()
	if retried.Load() != 1 {
		t.Fatalf("retried = %d, want 1", retried.Load())
	}
}

func TestMetricsReported(t *testing.T) {
	q := New(Config{Capacity: 10, Workers: 2, Backpressure: BackpressureReject},
		func(context.Context, Task) (bool, time.Duration) { return false, 0 })
	_ = q.Enqueue(context.Background(), Task{ID: "a", Session: "s1"})
	q.Close()
	q.Wait()
	m := q.Metrics()
	if m.Capacity != 10 {
		t.Fatalf("Capacity = %d", m.Capacity)
	}
	if m.Processed != 1 {
		t.Fatalf("Processed = %d, want 1", m.Processed)
	}
}

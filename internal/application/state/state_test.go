package state

import (
	"context"
	"sync"
	"testing"
	"time"
)

func TestMemSequenceAtomicNext(t *testing.T) {
	seq := NewMemSequenceRepository()
	ctx := context.Background()

	var mu sync.Mutex
	max := uint32(0)
	var wg sync.WaitGroup
	for i := 0; i < 16; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for j := 0; j < 50; j++ {
				v, err := seq.Next(ctx, "msgseq:group1")
				if err != nil {
					t.Error(err)
					return
				}
				mu.Lock()
				if v > max {
					max = v
				}
				mu.Unlock()
			}
		}()
	}
	wg.Wait()
	if max != 800 {
		t.Fatalf("max seq = %d, want 800 (no duplicates, no gaps below max)", max)
	}
}

func TestMemSequencePerKey(t *testing.T) {
	seq := NewMemSequenceRepository()
	ctx := context.Background()
	a, _ := seq.Next(ctx, "k1")
	b, _ := seq.Next(ctx, "k2")
	if a != 1 || b != 1 {
		t.Fatalf("a=%d b=%d, want both 1", a, b)
	}
}

func TestMemContextOwnerIsolation(t *testing.T) {
	repo := NewMemContextRepository()
	ctx := context.Background()

	if err := repo.Set(ctx, "botA", "mid", "abc", TTLDefault); err != nil {
		t.Fatal(err)
	}
	// 相同 key，不同 owner 不可见
	if _, err := repo.Get(ctx, "botB", "mid"); err != ErrNotFound {
		t.Fatalf("cross-owner read err = %v, want ErrNotFound", err)
	}
	got, err := repo.Get(ctx, "botA", "mid")
	if err != nil || got != "abc" {
		t.Fatalf("owner read = %q, %v", got, err)
	}
}

func TestMemContextExpired(t *testing.T) {
	repo := NewMemContextRepository()
	ctx := context.Background()
	_ = repo.Set(ctx, "botA", "k", "v", TTLShort)

	// 把 now 前移使其过期
	repo.mu.Lock()
	e := repo.entries[ownerKey("botA", "k")]
	e.ExpiresAt = time.Now().Add(-time.Second)
	repo.entries[ownerKey("botA", "k")] = e
	repo.mu.Unlock()

	if _, err := repo.Get(ctx, "botA", "k"); err != ErrNotFound {
		t.Fatalf("expired read err = %v, want ErrNotFound", err)
	}
}

func TestMemContextDelete(t *testing.T) {
	repo := NewMemContextRepository()
	ctx := context.Background()
	_ = repo.Set(ctx, "botA", "k", "v", TTLDefault)
	_ = repo.Delete(ctx, "botA", "k")
	if _, err := repo.Get(ctx, "botA", "k"); err != ErrNotFound {
		t.Fatalf("after delete err = %v", err)
	}
}

func TestMemContextCleanup(t *testing.T) {
	repo := NewMemContextRepository()
	ctx := context.Background()
	_ = repo.Set(ctx, "botA", "expired", "v", TTLShort)

	repo.mu.Lock()
	for k, e := range repo.entries {
		e.ExpiresAt = time.Now().Add(-time.Hour)
		repo.entries[k] = e
	}
	repo.mu.Unlock()

	repo.cleanupExpired()

	repo.mu.Lock()
	n := len(repo.entries)
	repo.mu.Unlock()
	if n != 0 {
		t.Fatalf("entries after cleanup = %d, want 0", n)
	}
}

func TestMemContextStartStop(t *testing.T) {
	repo := NewMemContextRepository()
	ctx, cancel := context.WithCancel(context.Background())
	repo.Start(ctx)
	time.Sleep(20 * time.Millisecond)
	cancel()
	// Close 由 ctx 取消触发；再次调用幂等
	repo.Close()
}

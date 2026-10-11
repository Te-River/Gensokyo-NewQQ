package staterepo

import (
	"context"
	"sync"
	"testing"
	"time"

	appstate "github.com/hoshinonyaruko/gensokyo/internal/application/state"
)

// TestEchoSequenceAtomicNext 并发调用 Next 不得重复、不得跳号。
func TestEchoSequenceAtomicNext(t *testing.T) {
	repo := NewEchoSequenceRepository()
	ctx := context.Background()
	const key = "staterepo-test:seq:atomic"

	var mu sync.Mutex
	max := uint32(0)
	var wg sync.WaitGroup
	for i := 0; i < 16; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for j := 0; j < 50; j++ {
				v, err := repo.Next(ctx, key)
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

// TestEchoSequencePerKey 不同 key 的序列相互独立。
func TestEchoSequencePerKey(t *testing.T) {
	repo := NewEchoSequenceRepository()
	ctx := context.Background()
	a, _ := repo.Next(ctx, "staterepo-test:seq:k1")
	b, _ := repo.Next(ctx, "staterepo-test:seq:k2")
	if a != 1 || b != 1 {
		t.Fatalf("a=%d b=%d, want both 1", a, b)
	}
}

// TestEchoContextOwnerMapping owner 映射到 echo appid 维度：同 key 不同 owner 隔离。
func TestEchoContextOwnerMapping(t *testing.T) {
	repo := NewEchoContextRepository()
	ctx := context.Background()

	if err := repo.Set(ctx, "botA", "mid", "abc", appstate.TTLDefault); err != nil {
		t.Fatal(err)
	}
	if _, err := repo.Get(ctx, "botB", "mid"); err != appstate.ErrNotFound {
		t.Fatalf("cross-owner read err = %v, want ErrNotFound", err)
	}
	got, err := repo.Get(ctx, "botA", "mid")
	if err != nil || got != "abc" {
		t.Fatalf("owner read = %q, %v", got, err)
	}
}

// TestEchoContextTTL 过期后 Get 返回 ErrNotFound。
func TestEchoContextTTL(t *testing.T) {
	repo := NewEchoContextRepository()
	ctx := context.Background()

	if err := repo.Set(ctx, "botA", "ttl", "v", 30*time.Millisecond); err != nil {
		t.Fatal(err)
	}
	if got, err := repo.Get(ctx, "botA", "ttl"); err != nil || got != "v" {
		t.Fatalf("fresh read = %q, %v", got, err)
	}
	time.Sleep(60 * time.Millisecond)
	if _, err := repo.Get(ctx, "botA", "ttl"); err != appstate.ErrNotFound {
		t.Fatalf("expired read err = %v, want ErrNotFound", err)
	}
}

// TestEchoContextDelete 删除后不可见；重新 Set 清除墓碑。
func TestEchoContextDelete(t *testing.T) {
	repo := NewEchoContextRepository()
	ctx := context.Background()

	_ = repo.Set(ctx, "botA", "del", "v", appstate.TTLDefault)
	_ = repo.Delete(ctx, "botA", "del")
	if _, err := repo.Get(ctx, "botA", "del"); err != appstate.ErrNotFound {
		t.Fatalf("after delete err = %v, want ErrNotFound", err)
	}
	_ = repo.Set(ctx, "botA", "del", "v2", appstate.TTLDefault)
	if got, err := repo.Get(ctx, "botA", "del"); err != nil || got != "v2" {
		t.Fatalf("re-set read = %q, %v", got, err)
	}
}

// TestEchoContextConcurrent 并发 Set/Get/Delete 无竞态（配合 -race）。
func TestEchoContextConcurrent(t *testing.T) {
	repo := NewEchoContextRepository()
	ctx := context.Background()

	var wg sync.WaitGroup
	for i := 0; i < 16; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for j := 0; j < 50; j++ {
				_ = repo.Set(ctx, "botA", "concurrent", "v", appstate.TTLDefault)
				_, _ = repo.Get(ctx, "botA", "concurrent")
				if j%5 == 0 {
					_ = repo.Delete(ctx, "botA", "concurrent")
				}
			}
		}()
	}
	wg.Wait()
}

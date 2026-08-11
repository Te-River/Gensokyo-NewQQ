package config

import (
	"context"
	"os"
	"path/filepath"
	"sync"
	"testing"
	"time"
)

func TestWatcherDebounce(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "config.yml")
	base := []byte("version: 1\nsettings:\n  app_id: 12345\n")
	if err := os.WriteFile(path, base, 0600); err != nil {
		t.Fatal(err)
	}

	var mu sync.Mutex
	count := 0
	w, err := NewWatcher(path, 300*time.Millisecond, func() error {
		mu.Lock()
		count++
		mu.Unlock()
		return nil
	})
	if err != nil {
		t.Fatalf("NewWatcher: %v", err)
	}
	defer w.Close()

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	go w.Start(ctx)

	// 事件风暴：短时间内连续写入，debounce 后应只触发 1 次
	for i := 0; i < 4; i++ {
		if err := os.WriteFile(path, base, 0600); err != nil {
			t.Fatal(err)
		}
		time.Sleep(30 * time.Millisecond)
	}

	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		mu.Lock()
		c := count
		mu.Unlock()
		if c == 1 {
			break
		}
		time.Sleep(50 * time.Millisecond)
	}

	mu.Lock()
	c := count
	mu.Unlock()
	if c != 1 {
		t.Fatalf("debounce failed: reload count = %d, want 1 (storm of 4 writes)", c)
	}
}

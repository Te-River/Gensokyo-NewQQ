package mylog

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
)

func readPanicLog(t *testing.T) string {
	t.Helper()
	data, err := os.ReadFile(filepath.Join("logs", "panic.log"))
	if err != nil {
		t.Fatalf("读取 panic.log 失败: %v", err)
	}
	return string(data)
}

func TestRecoverPanicWritesPanicLog(t *testing.T) {
	t.Chdir(t.TempDir())

	done := make(chan struct{})
	go func() {
		defer close(done)
		defer RecoverPanic("test")
		panic("boom")
	}()
	<-done // 若 panic 向外传播，进程会直接崩溃，走不到这里

	content := readPanicLog(t)
	for _, want := range []string{"scope=test", "panic: boom", "goroutine"} {
		if !strings.Contains(content, want) {
			t.Fatalf("panic.log 缺少 %q，实际内容:\n%s", want, content)
		}
	}
}

func TestRecoverPanicConcurrent(t *testing.T) {
	t.Chdir(t.TempDir())

	const workers = 8
	var wg sync.WaitGroup
	for i := 0; i < workers; i++ {
		wg.Add(1)
		go func(i int) {
			defer wg.Done()
			defer RecoverPanic(fmt.Sprintf("worker-%d", i))
			panic(fmt.Sprintf("boom-%d", i))
		}(i)
	}
	wg.Wait()

	content := readPanicLog(t)
	if got := strings.Count(content, "=== panic @"); got != workers {
		t.Fatalf("期望 %d 条崩溃记录，实际 %d 条：\n%s", workers, got, content)
	}
	for i := 0; i < workers; i++ {
		for _, want := range []string{fmt.Sprintf("scope=worker-%d", i), fmt.Sprintf("panic: boom-%d", i)} {
			if !strings.Contains(content, want) {
				t.Fatalf("panic.log 缺少 %q，实际内容:\n%s", want, content)
			}
		}
	}
}

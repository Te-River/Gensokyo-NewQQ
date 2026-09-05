package config

import (
	"bytes"
	"io"
	"os"
	"strings"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/structs"
)

// TestGetCQParseModeInvalidWarnsOnce 钉死 Minor 修复（2026-09-05 终审轮）：
// 首次命中非法 cq_parse_mode（含大小写错写）时输出一次警告（sync.Once 防刷屏），
// 值仍回退 legacy；合法值与空值不触发警告。
func TestGetCQParseModeInvalidWarnsOnce(t *testing.T) {
	// 保存并恢复单例，避免污染同包其他测试
	oldInstance := instance
	defer func() {
		mu.Lock()
		instance = oldInstance
		mu.Unlock()
	}()

	capture := func(fn func()) string {
		old := os.Stdout
		r, w, err := os.Pipe()
		if err != nil {
			t.Fatalf("os.Pipe 失败: %v", err)
		}
		os.Stdout = w
		done := make(chan string, 1)
		go func() {
			var buf bytes.Buffer
			_, _ = io.Copy(&buf, r)
			done <- buf.String()
		}()
		fn()
		_ = w.Close()
		os.Stdout = old
		return <-done
	}

	withMode := func(mode string) {
		mu.Lock()
		instance = &Config{Settings: structs.Settings{CQParseMode: mode}}
		mu.Unlock()
	}

	got := ""
	// 非法值（大小写错写）：回退 legacy + 恰好一次警告
	withMode("New")
	log1 := capture(func() {
		got = GetCQParseMode()
	})
	if got != "legacy" {
		t.Errorf("非法值应回退 legacy: got %q", got)
	}
	if !strings.Contains(log1, "cq_parse_mode") || !strings.Contains(log1, "回退 legacy") {
		t.Errorf("首次非法值应输出警告: got %q", log1)
	}

	// 第二次非法调用：不再警告（sync.Once 防刷屏）
	log2 := capture(func() {
		got = GetCQParseMode()
	})
	if got != "legacy" || strings.Contains(log2, "cq_parse_mode") {
		t.Errorf("第二次非法值不应重复警告: mode=%q log=%q", got, log2)
	}

	// 合法值：返回原值无警告
	withMode("shadow")
	log3 := capture(func() {
		got = GetCQParseMode()
	})
	if got != "shadow" || strings.Contains(log3, "cq_parse_mode") {
		t.Errorf("合法值不应警告: mode=%q log=%q", got, log3)
	}

	// 空值：静默回退 legacy 无警告
	withMode("")
	log4 := capture(func() {
		got = GetCQParseMode()
	})
	if got != "legacy" || strings.Contains(log4, "cq_parse_mode") {
		t.Errorf("空值应静默回退 legacy: mode=%q log=%q", got, log4)
	}
}

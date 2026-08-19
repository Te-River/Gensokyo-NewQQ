package config

import (
	"os"
	"path/filepath"
	"sync"
	"testing"
)

func TestBuildSnapshotMigratesLegacy(t *testing.T) {
	snap, err := BuildSnapshot(readFixture(t, "legacy-basic.yml"))
	if err != nil {
		t.Fatalf("BuildSnapshot: %v", err)
	}
	if snap.Version() != CurrentSchemaVersion {
		t.Fatalf("snapshot version = %d, want %d", snap.Version(), CurrentSchemaVersion)
	}
	if snap.LoadedAt().IsZero() {
		t.Fatal("loadedAt is zero")
	}
	if got := snap.Config().QQ.AppID; got != 12345 {
		t.Fatalf("app_id = %d, want 12345", got)
	}
}

func TestBuildSnapshotRejectsInvalid(t *testing.T) {
	if _, err := BuildSnapshot(readFixture(t, "invalid-port.yml")); err == nil {
		t.Fatal("BuildSnapshot accepted invalid config")
	}
}

func TestSnapshotDefensiveCopy(t *testing.T) {
	snap, err := BuildSnapshot(readFixture(t, "legacy-full.yml"))
	if err != nil {
		t.Fatalf("BuildSnapshot: %v", err)
	}

	// 篡改返回值的 slice，不应影响快照内部状态
	rc := snap.Config()
	rc.Transport.WsAddress[0] = "ws://hacked"
	rc.QQ.TextIntent = append(rc.QQ.TextIntent, "HackedHandler")

	again := snap.Config()
	if again.Transport.WsAddress[0] == "ws://hacked" {
		t.Fatal("snapshot WsAddress slice was mutated via returned copy")
	}
	if len(again.QQ.TextIntent) != 2 {
		t.Fatalf("snapshot TextIntent mutated: %v", again.QQ.TextIntent)
	}
}

func TestManagerReloadKeepsOldOnError(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "config.yml")
	if err := os.WriteFile(path, readFixture(t, "v1-basic.yml"), 0600); err != nil {
		t.Fatal(err)
	}

	m := NewManager(path)
	if err := m.Load(); err != nil {
		t.Fatalf("Load: %v", err)
	}
	if got := m.Snapshot().Config().QQ.AppID; got != 12345 {
		t.Fatalf("app_id = %d, want 12345", got)
	}

	// 写入坏配置后 reload 必须失败，且保留旧快照
	if err := os.WriteFile(path, []byte("version: 1\nsettings: [broken"), 0600); err != nil {
		t.Fatal(err)
	}
	if err := m.Reload(); err == nil {
		t.Fatal("Reload should fail on bad config")
	}
	if got := m.Snapshot().Config().QQ.AppID; got != 12345 {
		t.Fatalf("old snapshot lost after failed reload: app_id = %d", got)
	}

	// 修复配置后 reload 恢复
	if err := os.WriteFile(path, readFixture(t, "v1-basic.yml"), 0600); err != nil {
		t.Fatal(err)
	}
	if err := m.Reload(); err != nil {
		t.Fatalf("Reload after fix: %v", err)
	}
	if got := m.Snapshot().Config().QQ.AppID; got != 12345 {
		t.Fatalf("app_id after reload = %d, want 12345", got)
	}
}

func TestConcurrentSnapshotReadAndReload(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "config.yml")
	if err := os.WriteFile(path, readFixture(t, "legacy-full.yml"), 0600); err != nil {
		t.Fatal(err)
	}
	m := NewManager(path)
	if err := m.Load(); err != nil {
		t.Fatalf("Load: %v", err)
	}

	var wg sync.WaitGroup
	stop := make(chan struct{})
	for i := 0; i < 8; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for {
				select {
				case <-stop:
					return
				default:
					_ = m.Snapshot().Config()
				}
			}
		}()
	}
	for i := 0; i < 20; i++ {
		if err := os.WriteFile(path, readFixture(t, "v1-basic.yml"), 0600); err != nil {
			t.Fatal(err)
		}
		_ = m.Reload()
	}
	close(stop)
	wg.Wait()
}

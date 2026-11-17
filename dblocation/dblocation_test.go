package dblocation

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"go.etcd.io/bbolt"
)

// writeBboltFile 生成一个最小可用的 bbolt 数据库文件
func writeBboltFile(t *testing.T, path string) {
	t.Helper()
	db, err := bbolt.Open(path, 0600, &bbolt.Options{Timeout: time.Second})
	if err != nil {
		t.Fatalf("创建测试 bbolt 文件 %s 失败: %v", path, err)
	}
	err = db.Update(func(tx *bbolt.Tx) error {
		_, err := tx.CreateBucketIfNotExists([]byte("ids"))
		return err
	})
	db.Close()
	if err != nil {
		t.Fatalf("写入测试 bucket 失败: %v", err)
	}
}

func TestNormalizeDBLocation(t *testing.T) {
	cases := []struct {
		name    string
		in      string
		wantErr string // 非空表示期望报错且错误信息包含该片段
	}{
		{name: "绝对路径", in: filepath.Join(t.TempDir(), "data")},
		{name: "相对路径", in: "sub/data"},
		{name: "UNC路径", in: `\\server\share\gsk`},
		{name: "前后空格", in: "  sub/data  "},
		{name: "空路径", in: "   ", wantErr: "为空"},
		{name: "smb协议URL", in: "smb://server/share", wantErr: "协议 URL"},
		{name: "http协议URL", in: "http://example.com/db", wantErr: "协议 URL"},
		{name: "https协议URL", in: "https://example.com/db", wantErr: "协议 URL"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got, err := normalizeDBLocation(tc.in)
			if tc.wantErr != "" {
				if err == nil {
					t.Fatalf("期望报错含 %q,实际返回 %q", tc.wantErr, got)
				}
				if !strings.Contains(err.Error(), tc.wantErr) {
					t.Fatalf("错误信息 %q 未包含 %q", err.Error(), tc.wantErr)
				}
				return
			}
			if err != nil {
				t.Fatalf("意外报错: %v", err)
			}
			if !filepath.IsAbs(got) {
				t.Fatalf("期望解析为绝对路径,实际 %q", got)
			}
		})
	}
}

func TestProbeDir(t *testing.T) {
	t.Run("存在的可写目录", func(t *testing.T) {
		dir := t.TempDir()
		if err := probeDir(dir); err != nil {
			t.Fatalf("探测可写目录失败: %v", err)
		}
		if _, err := os.Stat(filepath.Join(dir, "gensokyo_db_probe.tmp")); !os.IsNotExist(err) {
			t.Fatalf("探针文件应被删除,实际: %v", err)
		}
	})
	t.Run("不存在的子目录可创建", func(t *testing.T) {
		if err := probeDir(filepath.Join(t.TempDir(), "a", "b")); err != nil {
			t.Fatalf("探测可创建目录失败: %v", err)
		}
	})
	t.Run("目标是文件时不可达", func(t *testing.T) {
		blocker := filepath.Join(t.TempDir(), "blocker")
		if err := os.WriteFile(blocker, []byte("x"), 0600); err != nil {
			t.Fatalf("创建占位文件失败: %v", err)
		}
		if err := probeDir(blocker); err == nil {
			t.Fatal("期望探测失败,实际成功")
		}
	})
	t.Run("探针文件被锁时探测失败", func(t *testing.T) {
		dir := t.TempDir()
		holder, err := bbolt.Open(filepath.Join(dir, "gensokyo_db_probe.tmp"), 0600, &bbolt.Options{Timeout: time.Second})
		if err != nil {
			t.Fatalf("创建持锁探针文件失败: %v", err)
		}
		defer holder.Close()
		// 短超时模拟网络挂载上锁互不可见/锁等待超时的场景
		if err := bboltProbe(dir, 200*time.Millisecond); err == nil {
			t.Fatal("期望探针失败(锁被占用),实际成功")
		}
	})
}

func TestMigrateDecisions(t *testing.T) {
	const name = "idmap-identity.db"
	cases := []struct {
		name        string
		localExists bool // 工作目录存在本地文件
		remoteFile  bool // 目标目录存在同名文件
		wantErr     string
		wantMigrate bool // 期望发生本地→目标迁移
	}{
		{name: "两边都没有_全新直接用目标"},
		{name: "本地有_迁移", localExists: true, wantMigrate: true},
		{name: "目标有_直接用目标", remoteFile: true},
		{name: "两边都有_冲突拒绝启动", localExists: true, remoteFile: true, wantErr: "迁移冲突"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			dir := t.TempDir()
			// 切到临时工作目录,避免污染真实工作目录
			t.Chdir(t.TempDir())

			localPath := name
			remotePath := filepath.Join(dir, name)
			if tc.localExists {
				writeBboltFile(t, localPath)
			}
			if tc.remoteFile {
				writeBboltFile(t, remotePath)
			}

			err := migrateOne(dir, name)
			if tc.wantErr != "" {
				if err == nil || !strings.Contains(err.Error(), tc.wantErr) {
					t.Fatalf("期望报错含 %q,实际: %v", tc.wantErr, err)
				}
				return
			}
			if err != nil {
				t.Fatalf("意外报错: %v", err)
			}
			if tc.wantMigrate {
				if _, err := os.Stat(remotePath); err != nil {
					t.Fatalf("迁移后目标文件不存在: %v", err)
				}
				if err := verifyBbolt(remotePath); err != nil {
					t.Fatalf("迁移后目标文件校验失败: %v", err)
				}
				matches, err := filepath.Glob(localPath + ".bak.*")
				if err != nil {
					t.Fatalf("查找备份文件失败: %v", err)
				}
				if len(matches) != 1 {
					t.Fatalf("期望本地原件保留为 .bak.*,实际 %v", matches)
				}
				if _, err := os.Stat(localPath); !os.IsNotExist(err) {
					t.Fatal("迁移后本地原件应已改名")
				}
			}
			if tc.remoteFile {
				if _, err := os.Stat(remotePath); err != nil {
					t.Fatalf("目标文件应保持不动: %v", err)
				}
			}
		})
	}
}

func TestPrepare(t *testing.T) {
	t.Run("协议URL拒绝启动", func(t *testing.T) {
		if _, err := prepare("smb://server/share"); err == nil || !strings.Contains(err.Error(), "协议 URL") {
			t.Fatalf("期望协议URL被拒绝,实际: %v", err)
		}
	})
	t.Run("空路径拒绝启动", func(t *testing.T) {
		if _, err := prepare("  "); err == nil || !strings.Contains(err.Error(), "为空") {
			t.Fatalf("期望空路径被拒绝,实际: %v", err)
		}
	})
	t.Run("不可达目录拒绝启动", func(t *testing.T) {
		blocker := filepath.Join(t.TempDir(), "blocker")
		if err := os.WriteFile(blocker, []byte("x"), 0600); err != nil {
			t.Fatalf("创建占位文件失败: %v", err)
		}
		if _, err := prepare(blocker); err == nil || !strings.Contains(err.Error(), "不可达或不可写") {
			t.Fatalf("期望不可达被拒绝,实际: %v", err)
		}
	})
	t.Run("与工作目录相同视为未启用", func(t *testing.T) {
		wd, err := os.Getwd()
		if err != nil {
			t.Fatalf("获取工作目录失败: %v", err)
		}
		got, err := prepare(wd)
		if err != nil {
			t.Fatalf("意外报错: %v", err)
		}
		if got != "" {
			t.Fatalf("期望返回空目录(视为未启用),实际 %q", got)
		}
	})
	t.Run("正常目录_含迁移", func(t *testing.T) {
		t.Chdir(t.TempDir())
		dir := t.TempDir()
		writeBboltFile(t, "idmap.db")
		got, err := prepare(dir)
		if err != nil {
			t.Fatalf("意外报错: %v", err)
		}
		if !sameDir(got, dir) {
			t.Fatalf("期望返回 %q,实际 %q", dir, got)
		}
		if _, err := os.Stat(filepath.Join(dir, "idmap.db")); err != nil {
			t.Fatalf("迁移后目标文件不存在: %v", err)
		}
	})
}

func TestResolve(t *testing.T) {
	if got := Resolve("idmap.db"); got != "idmap.db" {
		t.Fatalf("默认(未启用)应原样返回,实际 %q", got)
	}

	dir := t.TempDir()
	mu.Lock()
	enabled, baseDir = true, dir
	mu.Unlock()
	t.Cleanup(func() {
		mu.Lock()
		enabled, baseDir = false, ""
		mu.Unlock()
	})

	want := filepath.Join(dir, "idmap.db")
	if got := Resolve("idmap.db"); got != want {
		t.Fatalf("启用后期望 %q,实际 %q", want, got)
	}
}

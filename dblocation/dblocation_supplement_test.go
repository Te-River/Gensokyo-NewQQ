package dblocation

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// writeGarbageFile 生成一个非 bbolt 格式的非空文件,用于触发迁移校验失败
func writeGarbageFile(t *testing.T, path string) {
	t.Helper()
	data := make([]byte, 8192)
	for i := range data {
		data[i] = 'x'
	}
	if err := os.WriteFile(path, data, 0600); err != nil {
		t.Fatalf("创建垃圾文件 %s 失败: %v", path, err)
	}
}

// TestMigrateVerifyFailureRollback 非 bbolt 本地文件迁移时校验失败:
// 报错拒绝启动、目标副本删除、本地原件不动、不产生 .bak
func TestMigrateVerifyFailureRollback(t *testing.T) {
	t.Chdir(t.TempDir())
	dir := t.TempDir()
	const name = "idmap.db"
	writeGarbageFile(t, name)

	err := migrateOne(dir, name)
	if err == nil || !strings.Contains(err.Error(), "校验失败") {
		t.Fatalf("期望校验失败报错,实际: %v", err)
	}
	if _, err := os.Stat(filepath.Join(dir, name)); !os.IsNotExist(err) {
		t.Fatalf("校验失败后目标副本应已删除,实际: %v", err)
	}
	if _, err := os.Stat(name); err != nil {
		t.Fatalf("本地原件应原样保留: %v", err)
	}
	matches, err := filepath.Glob(name + ".bak.*")
	if err != nil {
		t.Fatalf("查找备份文件失败: %v", err)
	}
	if len(matches) != 0 {
		t.Fatalf("校验失败时不应产生备份文件,实际 %v", matches)
	}
}

// TestMigrateZeroByteFile 0 字节本地文件(边界):跳过 bbolt 只读校验,迁移成功并保留 .bak
func TestMigrateZeroByteFile(t *testing.T) {
	t.Chdir(t.TempDir())
	dir := t.TempDir()
	const name = "idmap-msg.db"
	if err := os.WriteFile(name, nil, 0600); err != nil {
		t.Fatalf("创建空文件失败: %v", err)
	}
	if err := migrateOne(dir, name); err != nil {
		t.Fatalf("0 字节文件迁移应成功: %v", err)
	}
	dstInfo, err := os.Stat(filepath.Join(dir, name))
	if err != nil || dstInfo.Size() != 0 {
		t.Fatalf("目标应为 0 字节副本: %v (size %d)", err, dstInfo.Size())
	}
	matches, err := filepath.Glob(name + ".bak.*")
	if err != nil || len(matches) != 1 {
		t.Fatalf("期望本地原件保留一个 .bak.*,实际 %v (err %v)", matches, err)
	}
}

// TestMigrateFamilyAllRegistered 注册表内全部 5 个文件名应一起参与迁移
func TestMigrateFamilyAllRegistered(t *testing.T) {
	t.Chdir(t.TempDir())
	dir := t.TempDir()
	// idmap.db 用真实 bbolt 内容,其余用 0 字节(历史遗留空文件场景)
	writeBboltFile(t, "idmap.db")
	for _, name := range []string{"idmap-identity.db", "idmap-msg.db", "openid-map.db", "msgid-map.db"} {
		if err := os.WriteFile(name, nil, 0600); err != nil {
			t.Fatalf("创建 %s 失败: %v", name, err)
		}
	}
	if err := migrateFamily(dir); err != nil {
		t.Fatalf("全族迁移失败: %v", err)
	}
	for _, name := range knownDBFiles {
		if _, err := os.Stat(filepath.Join(dir, name)); err != nil {
			t.Fatalf("注册表文件 %s 迁移后不在目标: %v", name, err)
		}
		matches, err := filepath.Glob(name + ".bak.*")
		if err != nil || len(matches) != 1 {
			t.Fatalf("本地 %s 应保留一个 .bak.*,实际 %v (err %v)", name, matches, err)
		}
	}
}

// TestMigrateConflictNoSideEffects 冲突分支:报错且本地原件保持原名、不产生备份
func TestMigrateConflictNoSideEffects(t *testing.T) {
	t.Chdir(t.TempDir())
	dir := t.TempDir()
	const name = "openid-map.db"
	writeBboltFile(t, name)
	writeBboltFile(t, filepath.Join(dir, name))

	err := migrateOne(dir, name)
	if err == nil || !strings.Contains(err.Error(), "迁移冲突") {
		t.Fatalf("期望迁移冲突报错,实际: %v", err)
	}
	if _, err := os.Stat(name); err != nil {
		t.Fatalf("冲突时本地原件应保持原名: %v", err)
	}
	matches, err := filepath.Glob(name + ".bak.*")
	if err != nil || len(matches) != 0 {
		t.Fatalf("冲突时不应产生备份,实际 %v (err %v)", matches, err)
	}
}

// TestProbeDirBboltProbePathOccupied bbolt 探针文件名被目录占用:
// 可写性检查通过而 bbolt.Open 探针失败,错误信息应指明目录不可用且不残留探测文件
func TestProbeDirBboltProbePathOccupied(t *testing.T) {
	dir := t.TempDir()
	if err := os.Mkdir(filepath.Join(dir, "gensokyo_db_probe.tmp"), 0755); err != nil {
		t.Fatalf("创建占位目录失败: %v", err)
	}
	err := probeDir(dir)
	if err == nil {
		t.Fatal("期望探针失败(探针文件名被目录占用),实际成功")
	}
	if !strings.Contains(err.Error(), "不可达或不可写") {
		t.Fatalf("错误信息应说明目录不可用: %v", err)
	}
	if _, err := os.Stat(filepath.Join(dir, ".gensokyo-dbprobe")); !os.IsNotExist(err) {
		t.Fatalf("可写性探测文件 .gensokyo-dbprobe 应被清理,实际: %v", err)
	}
}

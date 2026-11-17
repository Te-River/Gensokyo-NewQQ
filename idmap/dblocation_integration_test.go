package idmap

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"go.etcd.io/bbolt"

	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/dblocation"
	"github.com/hoshinonyaruko/gensokyo/template"
)

// TestInitializeDBCustomLocation 自定义数据库位置启用后,idmap 族全部数据库应在目标目录打开:
// 串起 dblocation.Setup(解析/探测/迁移裁决) -> Resolve -> service.go/new_service.go 的正式打开点,
// 验证工作目录不再产生 idmap 族文件(五处 30s 超时打开点在运行时可到达)。
func TestInitializeDBCustomLocation(t *testing.T) {
	target := t.TempDir()
	t.Chdir(t.TempDir())

	// 目标目录预置一份合法旧库,验证"目标有/本地无"分支与旧库打开点
	seed, err := bbolt.Open(filepath.Join(target, DBName), 0600, nil)
	if err != nil {
		t.Fatalf("创建目标旧库失败: %v", err)
	}
	if err := seed.Close(); err != nil {
		t.Fatalf("关闭目标旧库失败: %v", err)
	}

	// 以完整模板配置为基础(缺项会触发 config.ensureConfigComplete 重启进程),只改两项
	cfg := strings.ReplaceAll(template.ConfigTemplate, "<YOUR_SERVER_DIR>", "127.0.0.1")
	cfg = strings.ReplaceAll(cfg, "custom_db_enabled: false", "custom_db_enabled: true")
	cfg = strings.ReplaceAll(cfg, `custom_db_path: ""`, "custom_db_path: '"+filepath.ToSlash(target)+"'")
	if err := os.WriteFile("config.yml", []byte(cfg), 0644); err != nil {
		t.Fatalf("写入 config.yml 失败: %v", err)
	}
	if _, err := config.LoadConfig("config.yml", false); err != nil {
		t.Fatalf("加载配置失败: %v", err)
	}

	// 保存并恢复进程级全局态,避免影响同包其他用例
	origDB, origIdentity, origMsg := db, identityDB, msgDB
	t.Cleanup(func() {
		for _, h := range []*bbolt.DB{db, identityDB, msgDB} {
			if h != nil {
				h.Close()
			}
		}
		db, identityDB, msgDB = origDB, origIdentity, origMsg
	})

	dblocation.Setup()
	InitializeDB()

	if db == nil {
		t.Fatal("旧库应从目标目录打开成功,实际为 nil(未迁移或打开失败)")
	}
	for _, name := range []string{DBName, IdentityDBName, MsgDBName} {
		if _, err := os.Stat(filepath.Join(target, name)); err != nil {
			t.Fatalf("目标目录应有 %s: %v", name, err)
		}
		if _, err := os.Stat(name); !os.IsNotExist(err) {
			t.Fatalf("工作目录不应出现 %s(应全部落在目标目录),实际: %v", name, err)
		}
	}
}

// Package dblocation 提供数据库文件的自定义位置支持:
// 已知数据库文件名注册表 + 路径解析 + 可达性探测 + 迁移裁决,全部收敛在本包。
// 第一阶段注册表仅覆盖 idmap 族;botstats.db、cookie.db、gensokyo.db 等后续在
// knownDBFiles 追加一行即可接入,接入前其行为完全不变(开关打开也不迁移、不搬迁)。
package dblocation

import (
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"time"

	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"go.etcd.io/bbolt"
)

// 已知数据库文件名注册表:迁移裁决只针对注册表内的文件名,运行时打开路径也由本包统一解析。
// 其中 openid-map.db / msgid-map.db 来自 map_idmap 构建(默认不参与编译),
// 磁盘上可能存在历史文件,因此只进迁移清单,不参与默认构建的运行时打开。
var knownDBFiles = []string{
	"idmap.db",
	"idmap-identity.db",
	"idmap-msg.db",
	"openid-map.db",
	"msgid-map.db",
}

var (
	mu      sync.RWMutex
	enabled bool
	baseDir string // 解析后的数据库目录,仅在 enabled 时有效
)

// Setup 在配置加载后、任何数据库打开前调用(main.go 启动序列)。
// 开关关闭时零行为变化:所有数据库仍按相对工作目录打开。
// 启用失败(路径非法/不可达/迁移冲突/校验失败)时拒绝启动,绝不静默回退本地,避免两侧数据分叉。
func Setup() {
	if !config.GetCustomDbEnabled() {
		return
	}
	dir, err := prepare(config.GetCustomDbPath())
	if err != nil {
		mylog.Fatalf("[dblocation] 自定义数据库位置启用失败,拒绝启动(绝不回退本地,以免本地与目标两侧数据分叉): %v", err)
	}
	if dir == "" {
		mylog.Printf("[dblocation] custom_db_path 解析结果与工作目录相同,视为未启用,数据库位置不变")
		return
	}
	mu.Lock()
	enabled = true
	baseDir = dir
	mu.Unlock()
	mylog.Printf("[dblocation] idmap 族数据库位置: %s", dir)
	mylog.Warnf("[dblocation] 网络文件系统(NFS/SMB/挂载盘)上的文件锁语义无官方保证:本功能按单实例运行+用户自担风险设计,多进程并发访问同一网络位置上的库文件会损坏数据(etcd-io/bbolt issue #257 实测),请勿多开实例共用同一目录")
}

// prepare 执行路径解析、可达性探测与迁移裁决,返回解析后的数据库目录;
// 返回空字符串表示与工作目录相同(视为未启用),返回错误表示应拒绝启动。
func prepare(rawPath string) (string, error) {
	dir, err := normalizeDBLocation(rawPath)
	if err != nil {
		return "", err
	}
	workDir, err := os.Getwd()
	if err != nil {
		return "", fmt.Errorf("获取工作目录失败: %w", err)
	}
	if sameDir(dir, workDir) {
		return "", nil
	}
	if err := probeDir(dir); err != nil {
		return "", err
	}
	if err := migrateFamily(dir); err != nil {
		return "", err
	}
	return dir, nil
}

// normalizeDBLocation 是数据库位置的唯一路径解析入口,也是未来协议客户端的唯一扩展缝:
// 新的远程形态(smb:// 等)应在此接入协议客户端,而不是散落到各调用点。
// 当前仅支持文件系统路径/挂载点:
//   - 相对路径:相对工作目录解析为绝对路径
//   - 绝对路径与 UNC(\\server\share):直接使用
//   - 形如 smb:// http:// 的协议 URL:当前版本拒绝启动,由未来版本支持
func normalizeDBLocation(rawPath string) (string, error) {
	p := strings.TrimSpace(rawPath)
	if p == "" {
		return "", fmt.Errorf("custom_db_enabled 已开启但 custom_db_path 为空,请在 config.yml 中配置数据库目录")
	}
	if strings.Contains(p, "://") {
		return "", fmt.Errorf("custom_db_path %q 形似协议 URL:当前版本仅支持文件系统路径/网络挂载点(如 UNC \\\\server\\share 或已挂载盘符),协议 URL 由未来版本支持", p)
	}
	if !filepath.IsAbs(p) {
		abs, err := filepath.Abs(p)
		if err != nil {
			return "", fmt.Errorf("相对路径 %q 解析失败: %w", p, err)
		}
		p = abs
	}
	return filepath.Clean(p), nil
}

// sameDir 判断两个目录是否指向同一路径(Windows 路径大小写不敏感)
func sameDir(a, b string) bool {
	a, b = filepath.Clean(a), filepath.Clean(b)
	if runtime.GOOS == "windows" {
		return strings.EqualFold(a, b)
	}
	return a == b
}

// probeDir 探测目录可达性:存在(或可创建)、可写,且文件锁语义对 bbolt 数据库可用。
// 网络挂载可能慢,失败时间隔退避重试,共 3 次。
func probeDir(dir string) error {
	var lastErr error
	for attempt := 0; attempt < 3; attempt++ {
		if attempt > 0 {
			time.Sleep(time.Duration(attempt) * 500 * time.Millisecond)
		}
		if lastErr = probeDirOnce(dir); lastErr == nil {
			return nil
		}
	}
	return fmt.Errorf("目录 %s 不可达或不可写: %w (若为网络挂载,请检查挂载状态后重试;若为文件锁探针失败,说明该位置的文件锁语义对 bbolt 数据库不可用,不能作为数据库位置)", dir, lastErr)
}

// probeDirOnce 单次探测:可写性检查(快速失败) + bbolt 探针库(真实文件锁路径)
func probeDirOnce(dir string) error {
	if err := os.MkdirAll(dir, 0755); err != nil {
		return err
	}
	probe := filepath.Join(dir, ".gensokyo-dbprobe")
	f, err := os.OpenFile(probe, os.O_WRONLY|os.O_CREATE|os.O_TRUNC, 0600)
	if err != nil {
		return err
	}
	if _, err := f.WriteString("probe"); err != nil {
		f.Close()
		return err
	}
	if err := f.Close(); err != nil {
		return err
	}
	if err := os.Remove(probe); err != nil {
		return err
	}
	return bboltProbe(dir, 2*time.Second)
}

// bboltProbe 用一次性 bbolt 探针库验证目标位置的文件锁语义:
// bbolt.Open 会创建探针文件并获取进程文件锁,走的是与正式库完全相同的锁路径——
// 网络挂载上文件锁可能静默失效(NFS/SMB 的 flock/fcntl 语义均无保证,
// etcd-io/bbolt issue #257 实测 NFS 上多进程锁互不可见),仅靠"建临时文件再删"
// 无法发现。探针超时或锁错误 = 该位置对文件数据库不可用。
// 成功后立即关闭并删除探针文件,绝不残留。
func bboltProbe(dir string, lockTimeout time.Duration) error {
	probe := filepath.Join(dir, "gensokyo_db_probe.tmp")
	db, err := bbolt.Open(probe, 0600, &bbolt.Options{Timeout: lockTimeout})
	if err != nil {
		// 锁超时说明探针文件由持锁的另一个进程创建,删除它会破坏双实例同时启动时的意外互斥;
		// 非普通文件(如被同名目录占用)不是本进程的残留,删掉它会让重试轮次把占用当成已排除,故只清理普通文件
		if !errors.Is(err, bbolt.ErrTimeout) {
			if info, statErr := os.Stat(probe); statErr == nil && info.Mode().IsRegular() {
				os.Remove(probe)
			}
		}
		return err
	}
	if err := db.Close(); err != nil {
		return err
	}
	return os.Remove(probe)
}

// migrateFamily 按注册表清单逐文件做迁移裁决
func migrateFamily(dir string) error {
	for _, name := range knownDBFiles {
		if err := migrateOne(dir, name); err != nil {
			return err
		}
	}
	return nil
}

// migrateOne 对单个数据库文件做迁移裁决:
//   - 目标与本地同时存在 → 冲突,拒绝启动(无法自动裁决保留哪一份)
//   - 目标有、本地无     → 视为已迁移,直接用目标
//   - 本地有、目标无     → 文件级复制迁移(不解读格式),校验通过后本地原件改名保留
//   - 两边都没有         → 全新,直接用目标
func migrateOne(dir, name string) error {
	localPath := name // 相对工作目录
	remotePath := filepath.Join(dir, name)
	localInfo, localErr := os.Stat(localPath)
	remoteInfo, remoteErr := os.Stat(remotePath)
	if localErr != nil && !os.IsNotExist(localErr) {
		return fmt.Errorf("检查本地文件 %s 失败: %w", localPath, localErr)
	}
	if remoteErr != nil && !os.IsNotExist(remoteErr) {
		return fmt.Errorf("检查目标文件 %s 失败: %w", remotePath, remoteErr)
	}
	switch {
	case localErr == nil && remoteErr == nil:
		return fmt.Errorf("迁移冲突: 本地与目标目录同时存在 %s (本地 %s: %d 字节 / 目标 %s: %d 字节),请手动确认保留哪一份并删除另一份后重启", name, localPath, localInfo.Size(), remotePath, remoteInfo.Size())
	case remoteErr == nil:
		return nil
	case localErr == nil:
		return migrateFile(localPath, remotePath)
	default:
		return nil
	}
}

// migrateFile 执行单文件迁移:复制 → 校验 → 本地原件改名保留 → 摘要日志。
// 校验失败时删除目标副本(本地原件不动)并返回错误拒绝启动。
func migrateFile(localPath, remotePath string) error {
	srcInfo, err := os.Stat(localPath)
	if err != nil {
		return fmt.Errorf("读取本地文件 %s 失败: %w", localPath, err)
	}
	if err := copyFile(localPath, remotePath, srcInfo.Mode()); err != nil {
		// 网络盘断连/磁盘满会留下半截副本,不清掉则下次启动落入迁移冲突分支,把瞬时故障变成需人工删文件的永久拒绝启动
		os.Remove(remotePath)
		return fmt.Errorf("复制 %s 到 %s 失败: %w,已删除目标残留副本", localPath, remotePath, err)
	}
	dstInfo, err := os.Stat(remotePath)
	if err != nil {
		os.Remove(remotePath)
		return fmt.Errorf("校验目标文件 %s 失败: %w,已删除目标残留副本", remotePath, err)
	}
	if srcInfo.Size() != dstInfo.Size() {
		os.Remove(remotePath)
		return fmt.Errorf("迁移 %s 校验失败: 大小不一致(本地 %d 字节 / 目标 %d 字节),已删除目标副本,本地原件未动", localPath, srcInfo.Size(), dstInfo.Size())
	}
	// 0 字节文件无数据可校验,跳过 bbolt 只读验证(正式打开时会自动初始化)
	if srcInfo.Size() > 0 {
		if err := verifyBbolt(remotePath); err != nil {
			os.Remove(remotePath)
			return fmt.Errorf("迁移 %s 校验失败(只读打开 %s 失败): %v,已删除目标副本,本地原件未动", localPath, remotePath, err)
		}
	}
	bakPath := localPath + ".bak." + time.Now().Format("20060102-150405")
	if err := os.Rename(localPath, bakPath); err != nil {
		os.Remove(remotePath)
		return fmt.Errorf("备份本地文件 %s 到 %s 失败: %w,已删除目标副本", localPath, bakPath, err)
	}
	mylog.Printf("[dblocation] 已迁移 %s -> %s (%d 字节),本地原件保留为 %s", localPath, remotePath, srcInfo.Size(), bakPath)
	return nil
}

// copyFile 纯文件级拷贝,不解读文件格式
func copyFile(src, dst string, perm os.FileMode) error {
	srcFile, err := os.Open(src)
	if err != nil {
		return err
	}
	defer srcFile.Close()
	dstFile, err := os.OpenFile(dst, os.O_WRONLY|os.O_CREATE|os.O_TRUNC, perm)
	if err != nil {
		return err
	}
	defer dstFile.Close()
	if _, err := io.Copy(dstFile, srcFile); err != nil {
		return err
	}
	return dstFile.Sync()
}

// verifyBbolt 以只读方式打开文件验证可读且为 bbolt 格式,用完即关,绝不写入
func verifyBbolt(path string) error {
	db, err := bbolt.Open(path, 0400, &bbolt.Options{ReadOnly: true, Timeout: 1 * time.Second})
	if err != nil {
		return err
	}
	return db.Close()
}

// Resolve 返回数据库文件的实际打开路径,idmap 族所有运行时打开点必须经由此函数取路径:
// 开关关闭或未 Setup 时原样返回文件名(相对工作目录,与历史行为完全一致);
// 开启后返回解析目录下的绝对路径。
func Resolve(name string) string {
	mu.RLock()
	defer mu.RUnlock()
	if !enabled {
		return name
	}
	return filepath.Join(baseDir, name)
}

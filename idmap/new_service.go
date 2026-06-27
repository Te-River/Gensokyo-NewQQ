package idmap

import (
	"bytes"
	"encoding/binary"
	"fmt"
	"strconv"
	"sync"
	"sync/atomic"
	"time"

	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"go.etcd.io/bbolt"
)

// 新 idmap 系统：三库分离
//   idmap-identity.db — GroupOpenID ↔ 虚拟群ID + UserOpenID ↔ 虚拟用户ID（永久数据）
//   idmap-msg.db      — 真实 message_id ↔ 虚拟 message_id（临时缓存）
//   旧 idmap.db       — 仅读取（惰性迁移），不再写入

const (
	IdentityDBName = "idmap-identity.db"
	MsgDBName      = "idmap-msg.db"

	IdentityBucketName = "ids"
	MsgBucketName      = "cache"

	IdentityCounterKey = "currentRow"
	MsgCounterKey      = "currentRow"

	latestMsgBucket    = "latest_by_group_user"
	latestMsgExpBucket = "latest_expires_at"

	// internal sentinel key used by StoreLatestBotMsgID / GetLatestBotMsgID
	latestBotMessageUserKey = "\x00qq_bot_self"
)

var (
	identityDB *bbolt.DB // 身份映射 DB（group + user 共用）
	msgDB      *bbolt.DB // 消息 ID 缓存 DB
	newDBOnce  sync.Once
)

// initNewDBs 初始化新 DB（惰性，首次调用时打开）
func initNewDBs() {
	newDBOnce.Do(func() {
		var err error

		identityDB, err = bbolt.Open(IdentityDBName, 0600, nil)
		if err != nil {
			mylog.Fatalf("Error opening %s: %v", IdentityDBName, err)
		}

		msgDB, err = bbolt.Open(MsgDBName, 0600, nil)
		if err != nil {
			mylog.Fatalf("Error opening %s: %v", MsgDBName, err)
		}

		// 创建 buckets
		for _, d := range []struct {
			db     *bbolt.DB
			name   string
			bucket string
		}{
			{identityDB, IdentityDBName, IdentityBucketName},
			{identityDB, IdentityDBName, ConfigBucket},
			{identityDB, IdentityDBName, UserInfoBucket},
			{msgDB, MsgDBName, MsgBucketName},
			{msgDB, MsgDBName, latestMsgBucket},
			{msgDB, MsgDBName, latestMsgExpBucket},
		} {
			err = d.db.Update(func(tx *bbolt.Tx) error {
				_, err := tx.CreateBucketIfNotExists([]byte(d.bucket))
				return err
			})
			if err != nil {
				mylog.Fatalf("Error creating bucket in %s: %v", d.name, err)
			}
		}

		mylog.Printf("新 idmap 数据库已就绪: %s, %s", IdentityDBName, MsgDBName)

		// 启动 msg_id 缓存自动清理（每分钟扫描，过期 ≥6 分钟删除）
		startMsgCleanup()

		// 不在此处启动后台迁移（由 StartMigration 统一管理）
	})
}

// StartMigration 显式启动后台数据库迁移
// 先同步计数器（阻塞），再启动后台数据迁移（非阻塞）
// 调用返回后计数器已就绪，可安全连接 QQ 后端
func StartMigration() {
	initNewDBs()
	if hasOldDB() {
		mylog.Printf("========== idmap 数据库迁移 ==========")
		mylog.Printf("检测到旧 idmap.db，将自动迁移数据到新库:")
		mylog.Printf("  ├─ %s  ── 永久身份映射（群/用户 OpenID ↔ 虚拟 ID）", IdentityDBName)
		mylog.Printf("  └─ %s  ── 临时消息 ID 缓存", MsgDBName)
		mylog.Printf("=======================================")
		syncMigrationCounters()
		go backgroundMigration()
	}
}

// syncMigrationCounters 同步复制旧库计数器到新库（阻塞）
// 必须在连接 QQ 后端之前完成，防止 storeIdentity 分配的虚拟 ID 与迁移条目冲突
func syncMigrationCounters() {
	for _, spec := range []struct {
		oldBucket  string
		newBucket  string
		newDB      *bbolt.DB
		counterKey string
		label      string
	}{
		{BucketName, IdentityBucketName, identityDB, IdentityCounterKey, "identity"},
		{CacheBucketName, MsgBucketName, msgDB, MsgCounterKey, "msg"},
	} {
		var counterVal []byte
		_ = db.View(func(tx *bbolt.Tx) error {
			b := tx.Bucket([]byte(spec.oldBucket))
			if b != nil {
				v := b.Get([]byte(spec.counterKey))
				if v != nil {
					counterVal = make([]byte, len(v))
					copy(counterVal, v)
				}
			}
			return nil
		})
		if counterVal == nil {
			mylog.Printf("[idmap] %s 旧库无计数器，跳过", spec.label)
			continue
		}
		_ = spec.newDB.Update(func(tx *bbolt.Tx) error {
			b := tx.Bucket([]byte(spec.newBucket))
			if b.Get([]byte(spec.counterKey)) == nil {
				b.Put([]byte(spec.counterKey), counterVal)
				mylog.Printf("[idmap] %s 计数器同步完成: %d", spec.label, binary.BigEndian.Uint64(counterVal))
			}
			return nil
		})
	}
}

// hasOldDB 检查旧 idmap.db 是否存在
func hasOldDB() bool {
	// 如果旧 db 已经打开（由原有初始化逻辑负责），则返回 true
	return db != nil
}

// ---------------------------------------------------------------------------
// 身份映射（Group + User）
// ---------------------------------------------------------------------------

// storeIdentity 写入身份映射（内部核心函数）
// openID: 真实 OpenID（32位 hex 字符串）
// 返回: 虚拟数字 ID
func storeIdentity(openID string) (int64, error) {
	initNewDBs()

	var newRow int64
	key := uinKey(openID)
	revPrefix := uinRowKey("")

	err := identityDB.Update(func(tx *bbolt.Tx) error {
		b := tx.Bucket([]byte(IdentityBucketName))

		// 已存在直接返回（虚拟 ID=0 的除外，那是解绑残留，需重新分配新 ID）
		existing := b.Get([]byte(key))
		if existing != nil {
			newRow = int64(binary.BigEndian.Uint64(existing))
			if newRow != 0 {
				return nil
			}
			// 虚拟 ID=0：先清理旧逆向映射 row-0，再分配新 ID
			b.Delete([]byte(revPrefix + "0"))
		}

		// 分配虚拟 ID
		if !config.GetHashIDValue() {
			currentRowBytes := b.Get([]byte(IdentityCounterKey))
			if currentRowBytes == nil {
				newRow = 1
			} else {
				newRow = int64(binary.BigEndian.Uint64(currentRowBytes)) + 1
			}
		} else {
			var err error
			maxDigits := 18
			for digits := 9; digits <= maxDigits; digits++ {
				newRow, err = GenerateRowID(openID, digits)
				if err != nil {
					return err
				}
				rowKey := uinRowKey(strconv.FormatInt(newRow, 10))
				if b.Get([]byte(rowKey)) == nil {
					break
				}
				if digits == maxDigits {
					return fmt.Errorf("unable to find unique row ID after %d attempts", maxDigits-8)
				}
			}
		}

		rowBytes := make([]byte, 8)
		binary.BigEndian.PutUint64(rowBytes, uint64(newRow))

		if !config.GetHashIDValue() {
			b.Put([]byte(IdentityCounterKey), rowBytes)
		}
		b.Put([]byte(key), rowBytes)
		b.Put([]byte(revPrefix+strconv.FormatInt(newRow, 10)), []byte(key))

		if config.GetIdmapIsolation() && config.GetIdmapLegacyCompat() {
			b.Put([]byte(openID), rowBytes)
		}
		return nil
	})

	// 写旧库保持双写兼容（惰性迁移期）
	if err == nil {
		dualWriteToOldDB(key, openID, newRow)
	}

	return newRow, err
}

// retrieveIdentity 根据虚拟 ID 查找真实 OpenID（惰性：新库找不到时查旧库）
func retrieveIdentity(virtualID string) (string, error) {
	initNewDBs()

	var id string
	revKey := uinRowKey(virtualID)

	err := identityDB.View(func(tx *bbolt.Tx) error {
		b := tx.Bucket([]byte(IdentityBucketName))
		idBytes := b.Get([]byte(revKey))
		if idBytes == nil {
			return ErrKeyNotFound
		}
		id = stripUinPrefix(string(idBytes))
		return nil
	})
	if err == nil && id != "" {
		return id, nil
	}

	// 惰性迁移：新库查不到，查旧库
	id, err = lazyMigrateIdentity(virtualID)
	if err == nil {
		return id, nil
	}

	return "", ErrKeyNotFound
}

// lazyMigrateIdentity 从旧 idmap.db 读取并写入新库
func lazyMigrateIdentity(virtualID string) (string, error) {
	if !hasOldDB() {
		return "", ErrKeyNotFound
	}

	id, err := RetrieveRowByID(virtualID)
	if err != nil {
		return "", err
	}

	// 写入新库，下次就不用查旧库了
	rawKey := stripUinPrefix(id)
	if len(rawKey) == 32 {
		// 32位 OpenID，写入新库
		writeBackIdentity(virtualID, id)
	}

	return id, nil
}

// writeBackIdentity 将旧库数据回写到新库
func writeBackIdentity(virtualID string, openID string) {
	key := uinKey(openID)
	revPrefix := uinRowKey("")

	_ = identityDB.Update(func(tx *bbolt.Tx) error {
		b := tx.Bucket([]byte(IdentityBucketName))

		rowBytes := make([]byte, 8)
		vID, _ := strconv.ParseInt(virtualID, 10, 64)
		binary.BigEndian.PutUint64(rowBytes, uint64(vID))

		b.Put([]byte(key), rowBytes)
		b.Put([]byte(revPrefix+virtualID), []byte(key))
		return nil
	})
}

// dualWriteToOldDB 双写到旧库（兼容期，迁移完成后跳过）
func dualWriteToOldDB(key, openID string, rowID int64) {
	if !hasOldDB() || isMigrationComplete() {
		return
	}

	revKey := uinRowKey(strconv.FormatInt(rowID, 10))
	rowBytes := make([]byte, 8)
	binary.BigEndian.PutUint64(rowBytes, uint64(rowID))

	_ = db.Update(func(tx *bbolt.Tx) error {
		b := tx.Bucket([]byte(BucketName))
		// 先清理旧的 row-0 逆向映射（解绑残留）
		b.Delete([]byte(uinRowKey("0")))
		b.Delete([]byte("row-0"))
		// 写入新映射
		b.Put([]byte(key), rowBytes)
		b.Put([]byte(revKey), []byte(key))
		if config.GetIdmapIsolation() && config.GetIdmapLegacyCompat() {
			b.Put([]byte(openID), rowBytes)
		}
		return nil
	})
}

// ---------------------------------------------------------------------------
// 后台静默迁移
// ---------------------------------------------------------------------------

var migrationStarted int32 // atomic: 0=未启动, 1=已启动

// backgroundMigration 后台扫描旧库，将数据分批搬入新库
func backgroundMigration() {
	if !atomic.CompareAndSwapInt32(&migrationStarted, 0, 1) {
		return
	}

	go func() {
		// 检查是否已全部迁移完成
		if isMigrationComplete() {
			mylog.Printf("[idmap] 数据库迁移已完成，跳过")
			return
		}

		mylog.Printf("[idmap] ──── 第一阶段：迁移身份映射（群/用户） ────")
		// 先迁移 identity（ids 桶）
		migrateBucket(BucketName, IdentityBucketName, identityDB, "identity")

		mylog.Printf("[idmap] ──── 第二阶段：迁移配置与应用数据 ────")
		// msg_id 缓存不迁移（旧 cache 桶跳过），新 msg DB 从零开始
		migrateBucket(ConfigBucket, ConfigBucket, identityDB, "config")
		migrateBucket(UserInfoBucket, UserInfoBucket, identityDB, "UserInfo")

		mylog.Printf("[idmap] ──── 第三阶段：数据完整性校验 ────")
		// 校验

		mylog.Printf("[idmap] ──── 第四阶段：数据完整性校验 ────")
		// 校验
		if verifyMigration() {
			mylog.Printf("[idmap] ✅ 数据校验通过，旧 idmap.db 数据已全部安全迁移")
			markMigrationComplete()
			finalizeOldDB()
		} else {
			mylog.Printf("[idmap] ❌ 数据校验失败，自动修复中...")
			repairMigration()
		}
	}()
}

// entry 表示旧库中一条待迁移的键值对
type entry struct {
	key   string
	value string
}

// migrateBucket 将旧库一个桶中的数据逐条搬入新库
func migrateBucket(oldBucket, newBucket string, newDB *bbolt.DB, label string) {
	if !hasOldDB() {
		return
	}

	mylog.Printf("[idmap] 开始迁移 %s ...", label)

	// === 第零步：先同步计数器，防止 storeIdentity 与迁移条目 ID 碰撞 ===
	// 迁移期间新写入可能并发发生，如果新库 counter 为 0 而迁移条目 ID 已有 1~N，
	// storeIdentity 会分配冲突的虚拟 ID，导致反向映射被覆盖丢失。
	var oldCounterVal []byte
	_ = db.View(func(tx *bbolt.Tx) error {
		b := tx.Bucket([]byte(oldBucket))
		if b != nil {
			counterKey := IdentityCounterKey
			if oldBucket == CacheBucketName {
				counterKey = MsgCounterKey
			}
			v := b.Get([]byte(counterKey))
			if v != nil {
				oldCounterVal = make([]byte, len(v))
				copy(oldCounterVal, v)
			}
		}
		return nil
	})
	if oldCounterVal != nil {
		_ = newDB.Update(func(tx *bbolt.Tx) error {
			b := tx.Bucket([]byte(newBucket))
			counterKey := IdentityCounterKey
			if newBucket == MsgBucketName {
				counterKey = MsgCounterKey
			}
			// 只在没有计数器时才写入（幂等）
			if b.Get([]byte(counterKey)) == nil {
				b.Put([]byte(counterKey), oldCounterVal)
				mylog.Printf("[idmap] %s 计数器已同步: %d", label, binary.BigEndian.Uint64(oldCounterVal))
			}
			return nil
		})
	}

	batchSize := 100
	total := 0
	lastLog := time.Now()
	var cursorKey []byte // 局部游标，每个桶独立

	// 先统计旧库总条数（粗略估算）
	var estimatedTotal int
	_ = db.View(func(tx *bbolt.Tx) error {
		b := tx.Bucket([]byte(oldBucket))
		if b != nil {
			stats := b.Stats()
			estimatedTotal = stats.KeyN
		}
		return nil
	})
	if estimatedTotal > 0 {
		mylog.Printf("[idmap] %s 旧库约 %d 条数据，开始分批迁移（每批 %d 条）", label, estimatedTotal, batchSize)
	} else {
		mylog.Printf("[idmap] %s 旧库数据统计中...", label)
	}

	for {
		// 从旧库读一批（传入局部游标指针）
		batch, done, err := readOldDBBatch(oldBucket, &cursorKey, batchSize)
		if err != nil || len(batch) == 0 {
			if done {
				mylog.Printf("[idmap] ✅ %s 迁移完成，共 %d 条", label, total)
			}
			return
		}

		// 写入新库（跳过已存在的）
		written := writeBatchToNewDB(newDB, newBucket, batch)
		total += written

		if done {
			mylog.Printf("[idmap] ✅ %s 迁移完成，共 %d 条", label, total)
			return
		}

		// 每 2 秒或每 500 条打印一次进度
		if time.Since(lastLog) > 2*time.Second || total%(batchSize*5) == 0 {
			pct := ""
			if estimatedTotal > 0 {
				p := total * 100 / estimatedTotal
				if p > 100 {
					p = 100
				}
				pct = fmt.Sprintf(" (%d%%)", p)
			}
			mylog.Printf("[idmap] ⏳ %s 迁移进度: %d 条%s", label, total, pct)
			lastLog = time.Now()
		}

		// 每批之间稍作暂停，避免 CPU 争抢
		time.Sleep(10 * time.Millisecond)
	}
}

// readOldDBBatch 从旧库中读取一批尚未迁移的条目
func readOldDBBatch(bucketName string, cursorKey *[]byte, limit int) ([]entry, bool, error) {
	var entries []entry
	done := false

	err := db.View(func(tx *bbolt.Tx) error {
		b := tx.Bucket([]byte(bucketName))
		if b == nil {
			done = true
			return nil
		}

		c := b.Cursor()
		k, v := c.Seek(*cursorKey)
		if k == nil {
			done = true
			return nil
		}

		// 跳过计数器 key（不消耗批次额度）
		count := 0
		for count < limit && k != nil {
			keyStr := string(k)
			if keyStr == IdentityCounterKey || keyStr == MsgCounterKey {
				k, v = c.Next()
				continue // 不消耗 limit，继续下一条
			}
			entries = append(entries, entry{key: keyStr, value: string(v)})
			count++
			k, v = c.Next()
		}

		if k == nil {
			done = true
		} else {
			*cursorKey = make([]byte, len(k))
			copy(*cursorKey, k)
		}
		return nil
	})

	if err != nil {
		return nil, true, err
	}
	if len(entries) == 0 && done {
		return entries, true, nil
	}
	return entries, done, nil
}

// writeBatchToNewDB 将一批条目写入新库（跳过已存在的）
func writeBatchToNewDB(newDB *bbolt.DB, bucketName string, entries []entry) int {
	written := 0
	_ = newDB.Update(func(tx *bbolt.Tx) error {
		b := tx.Bucket([]byte(bucketName))
		for _, e := range entries {
			if b.Get([]byte(e.key)) == nil {
				b.Put([]byte(e.key), []byte(e.value))
				written++
			}
		}
		return nil
	})
	return written
}

// ---------------------------------------------------------------------------
// 数据校验与收尾
// ---------------------------------------------------------------------------

const migrationMarkerKey = "_migration_complete_v1"

// isMigrationComplete 检查是否已全部迁移完成
func isMigrationComplete() bool {
	initNewDBs()
	var done bool
	_ = identityDB.View(func(tx *bbolt.Tx) error {
		b := tx.Bucket([]byte(IdentityBucketName))
		done = b.Get([]byte(migrationMarkerKey)) != nil
		return nil
	})
	return done
}

// markMigrationComplete 标记迁移完成
func markMigrationComplete() {
	initNewDBs()
	_ = identityDB.Update(func(tx *bbolt.Tx) error {
		b := tx.Bucket([]byte(IdentityBucketName))
		return b.Put([]byte(migrationMarkerKey), []byte("1"))
	})
}

// verifyMigration 校验旧库数据是否全部正确迁移到新库
func verifyMigration() bool {
	if !hasOldDB() {
		return true
	}

	mylog.Printf("[idmap] 正在逐条校验数据完整性...")
	ok := true

	ok = verifyBucket(BucketName, IdentityBucketName, identityDB, "identity") && ok
	ok = verifyBucket(ConfigBucket, ConfigBucket, identityDB, "config") && ok
	ok = verifyBucket(UserInfoBucket, UserInfoBucket, identityDB, "UserInfo") && ok

	return ok
}

// verifyBucket 校验单个桶的迁移完整性（游标流式对比，避免大库 OOM）
func verifyBucket(oldBucket, newBucket string, newDB *bbolt.DB, label string) bool {
	var oldCount, mismatch int

	_ = newDB.View(func(newTx *bbolt.Tx) error {
		newB := newTx.Bucket([]byte(newBucket))
		if newB == nil {
			mismatch = 1
			mylog.Printf("[idmap] 校验 %s: 新库桶 %s 不存在", label, newBucket)
			return nil
		}

		return db.View(func(oldTx *bbolt.Tx) error {
			oldB := oldTx.Bucket([]byte(oldBucket))
			if oldB == nil {
				// 旧桶不存在 = 没有数据需要迁移，校验通过
				return nil
			}

			c := oldB.Cursor()
			for k, v := c.First(); k != nil; k, v = c.Next() {
				key := string(k)
				if key == IdentityCounterKey || key == MsgCounterKey {
					continue
				}
				oldCount++

				nv := newB.Get(k)
				if nv == nil {
					mismatch++
					if mismatch <= 5 {
						mylog.Printf("[idmap] 校验 %s: 丢失 key=%s", label, key)
					}
				} else if string(nv) != string(v) {
					mismatch++
					if mismatch <= 5 {
						mylog.Printf("[idmap] 校验 %s: 值不匹配 key=%s (old=%s new=%s)", label, key, string(v), string(nv))
					}
				}
			}
			return nil
		})
	})

	if mismatch > 0 {
		mylog.Printf("[idmap] 校验 %s: %d 条丢失/不匹配 (共 %d 条)", label, mismatch, oldCount)
		return false
	}

	mylog.Printf("[idmap] 校验 %s: %d 条全部一致", label, oldCount)
	return true
}

// ---------------------------------------------------------------------------
// 数据校验与收尾
// ---------------------------------------------------------------------------

// repairMigration 用旧库覆盖修复新库中不一致的条目
func repairMigration() {
	if !hasOldDB() {
		mylog.Printf("[idmap] 无旧库可修复")
		return
	}

	r1 := repairBucket(BucketName, IdentityBucketName, identityDB, "identity")
	r2 := repairBucket(ConfigBucket, ConfigBucket, identityDB, "config")
	r3 := repairBucket(UserInfoBucket, UserInfoBucket, identityDB, "UserInfo")

	if r1+r2+r3 > 0 {
		mylog.Printf("[idmap] 修复完成，共修复 %d 条", r1+r2+r3)
	} else {
		mylog.Printf("[idmap] 未发现需修复的条目")
	}

	if verifyMigration() {
		mylog.Printf("[idmap] 修复后校验通过")
		markMigrationComplete()
		finalizeOldDB()
	} else {
		mylog.Printf("[idmap] ❌ 修复后校验仍失败，保留旧库，请手动检查")
		mylog.Printf("[idmap] 手动修复: 删除 %s 和 %s，重启 Gensokyo 重新迁移", IdentityDBName, MsgDBName)
	}
}

// repairBucket 用旧库覆盖修复新库中不一致的条目
func repairBucket(oldBucket, newBucket string, newDB *bbolt.DB, label string) int {
	fixed := 0
	_ = newDB.Update(func(tx *bbolt.Tx) error {
		b := tx.Bucket([]byte(newBucket))
		_ = db.View(func(oldTx *bbolt.Tx) error {
			oldB := oldTx.Bucket([]byte(oldBucket))
			if oldB == nil {
				return nil
			}
			return oldB.ForEach(func(k, v []byte) error {
				key := string(k)
				if key == IdentityCounterKey || key == MsgCounterKey {
					return nil
				}
				existing := b.Get(k)
				if existing == nil || string(existing) != string(v) {
					b.Put(k, v)
					fixed++
				}
				return nil
			})
		})
		return nil
	})
	if fixed > 0 {
		mylog.Printf("[idmap] 修复 %s: %d 条不一致", label, fixed)
	}
	return fixed
}

func finalizeOldDB() {
	if !hasOldDB() {
		return
	}

	mylog.Printf("[idmap] ======== 迁移全部完成 ========")
	mylog.Printf("[idmap]   ✓ %s/ids       ── 永久身份映射", IdentityDBName)
	mylog.Printf("[idmap]   ✓ %s/config    ── 运行时配置", IdentityDBName)
	mylog.Printf("[idmap]   ✓ %s/UserInfo  ── 用户信息缓存", IdentityDBName)
	mylog.Printf("[idmap]   ◉ idmap-msg.db  ── 消息 ID 缓存（跳过旧库迁移，从零开始，自动清理）")
	mylog.Printf("[idmap]")
	mylog.Printf("[idmap]   ◉ idmap.db     ── 旧库（所有数据已迁出，可安全删除）")
	mylog.Printf("[idmap]")
	mylog.Printf("[idmap] 旧库安全删除方法: 停止 Gensokyo → 删除 idmap.db → 重启")
	mylog.Printf("[idmap] =================================")
}

// StoreGroupID 存储群 OpenID → 虚拟群 ID
func StoreGroupID(groupOpenID string) (int64, error) {
	return storeIdentity(groupOpenID)
}

// StoreUserID 存储用户 OpenID → 虚拟用户 ID
func StoreUserID(userOpenID string) (int64, error) {
	return storeIdentity(userOpenID)
}

// RetrieveGroupID 虚拟群 ID → 真实群 OpenID
func RetrieveGroupID(virtualID string) (string, error) {
	return retrieveIdentity(virtualID)
}

// RetrieveUserID 虚拟用户 ID → 真实用户 OpenID
func RetrieveUserID(virtualID string) (string, error) {
	return retrieveIdentity(virtualID)
}

// StoreMsgID 存储真实消息 ID → 虚拟消息 ID
func StoreMsgID(realMsgID string) (int64, error) {
	initNewDBs()

	var newRow int64
	key := uinKey(realMsgID)
	revPrefix := uinRowKey("")

	err := msgDB.Update(func(tx *bbolt.Tx) error {
		b := tx.Bucket([]byte(MsgBucketName))

		existing := b.Get([]byte(key))
		if existing != nil {
			newRow = int64(binary.BigEndian.Uint64(existing))
			return nil
		}

		currentRowBytes := b.Get([]byte(MsgCounterKey))
		if currentRowBytes == nil {
			newRow = 1
		} else {
			newRow = int64(binary.BigEndian.Uint64(currentRowBytes)) + 1
		}

		rowBytes := make([]byte, 8)
		binary.BigEndian.PutUint64(rowBytes, uint64(newRow))
		b.Put([]byte(MsgCounterKey), rowBytes)
		b.Put([]byte(key), rowBytes)
		b.Put([]byte(revPrefix+strconv.FormatInt(newRow, 10)), []byte(key))

		// 写入时间戳（用于自动过期清理）
		timeBytes := make([]byte, 8)
		binary.BigEndian.PutUint64(timeBytes, uint64(time.Now().Unix()))
		b.Put([]byte("ts:"+revPrefix+strconv.FormatInt(newRow, 10)), timeBytes)

		// 惰性迁移期：同时写旧 cache 桶（迁移完成后跳过）
		if hasOldDB() && !isMigrationComplete() {
			_ = db.Update(func(tx2 *bbolt.Tx) error {
				b2 := tx2.Bucket([]byte(CacheBucketName))
				if b2.Get([]byte(key)) == nil {
					b2.Put([]byte(key), rowBytes)
					b2.Put([]byte(revPrefix+strconv.FormatInt(newRow, 10)), []byte(key))
				}
				return nil
			})
		}
		return nil
	})

	return newRow, err
}

// RetrieveMsgID 虚拟消息 ID → 真实消息 ID
func RetrieveMsgID(virtualID string) (string, error) {
	initNewDBs()

	var id string
	revKey := uinRowKey(virtualID)

	err := msgDB.View(func(tx *bbolt.Tx) error {
		b := tx.Bucket([]byte(MsgBucketName))
		idBytes := b.Get([]byte(revKey))
		if idBytes == nil {
			return ErrKeyNotFound
		}
		id = stripUinPrefix(string(idBytes))
		return nil
	})
	if err == nil && id != "" {
		return id, nil
	}

	// 惰性迁移
	if hasOldDB() {
		id, err = RetrieveRowByCache(virtualID)
		if err == nil && id != "" {
			// 写回新库
			_ = msgDB.Update(func(tx *bbolt.Tx) error {
				b := tx.Bucket([]byte(MsgBucketName))
				key := uinKey(id)
				rowBytes := make([]byte, 8)
				vID, _ := strconv.ParseInt(virtualID, 10, 64)
				binary.BigEndian.PutUint64(rowBytes, uint64(vID))
				b.Put([]byte(key), rowBytes)
				b.Put([]byte(revKey), []byte(key))
				timeBytes := make([]byte, 8)
				binary.BigEndian.PutUint64(timeBytes, uint64(time.Now().Unix()))
				b.Put([]byte("ts:"+string(revKey)), timeBytes)
				return nil
			})
			return id, nil
		}
	}

	return "", ErrKeyNotFound
}

// CleanMsgDB 清理消息 ID 缓存 DB（可安全删除）
func CleanMsgDB() error {
	initNewDBs()
	return msgDB.Update(func(tx *bbolt.Tx) error {
		b := tx.Bucket([]byte(MsgBucketName))
		return b.ForEach(func(k, v []byte) error {
			return b.Delete(k)
		})
	})
}

// newDBStore 由旧 StoreIDv2 调用，双写到新 identity DB
func newDBStore(openID string, virtualID int64) {
	initNewDBs()
	key := uinKey(openID)
	revPrefix := uinRowKey("")

	_ = identityDB.Update(func(tx *bbolt.Tx) error {
		b := tx.Bucket([]byte(IdentityBucketName))
		rowBytes := make([]byte, 8)
		binary.BigEndian.PutUint64(rowBytes, uint64(virtualID))
		if b.Get([]byte(key)) == nil {
			b.Put([]byte(key), rowBytes)
			b.Put([]byte(revPrefix+strconv.FormatInt(virtualID, 10)), []byte(key))
		}
		return nil
	})
}

// newDBLookup 由旧 RetrieveRowByIDv2 调用，优先查新 identity DB
func newDBLookup(virtualID string) (string, bool) {
	initNewDBs()
	revKey := uinRowKey(virtualID)
	var result string

	err := identityDB.View(func(tx *bbolt.Tx) error {
		b := tx.Bucket([]byte(IdentityBucketName))
		v := b.Get([]byte(revKey))
		if v == nil {
			return ErrKeyNotFound
		}
		result = stripUinPrefix(string(v))
		return nil
	})

	if err == nil && result != "" {
		return result, true
	}
	return "", false
}

// newDBMsgStore 由旧 StoreCachev2 调用，双写到新 msg DB
func newDBMsgStore(realMsgID string, virtualID int64) {
	initNewDBs()
	key := uinKey(realMsgID)
	revPrefix := uinRowKey("")

	_ = msgDB.Update(func(tx *bbolt.Tx) error {
		b := tx.Bucket([]byte(MsgBucketName))
		rowBytes := make([]byte, 8)
		binary.BigEndian.PutUint64(rowBytes, uint64(virtualID))
		if b.Get([]byte(key)) == nil {
			b.Put([]byte(key), rowBytes)
			b.Put([]byte(revPrefix+strconv.FormatInt(virtualID, 10)), []byte(key))
			timeBytes := make([]byte, 8)
			binary.BigEndian.PutUint64(timeBytes, uint64(time.Now().Unix()))
			b.Put([]byte("ts:"+revPrefix+strconv.FormatInt(virtualID, 10)), timeBytes)
		}
		return nil
	})
}

// newDBMsgLookup 由旧 RetrieveRowByCachev2 调用，优先查新 msg DB
func newDBMsgLookup(virtualID string) (string, bool) {
	initNewDBs()
	revKey := uinRowKey(virtualID)
	var result string

	err := msgDB.View(func(tx *bbolt.Tx) error {
		b := tx.Bucket([]byte(MsgBucketName))
		v := b.Get([]byte(revKey))
		if v == nil {
			return ErrKeyNotFound
		}
		result = stripUinPrefix(string(v))
		return nil
	})

	if err == nil && result != "" {
		return result, true
	}
	return "", false
}

// configAndUserInfoDB 返回 config/UserInfo 桶当前应使用的 DB
// 迁移完成后返回 identityDB，否则返回旧 db（兼容期）
func configAndUserInfoDB() *bbolt.DB {
	if isMigrationComplete() {
		initNewDBs()
		return identityDB
	}
	return db
}

// startMsgCleanup 启动 msg_id 缓存自动清理协程（每分钟扫描一次）
func startMsgCleanup() {
	go func() {
		for {
			time.Sleep(1 * time.Minute)
			cleanExpiredMsgIDs()
		}
	}()
}

// cleanExpiredMsgIDs 扫描 msg DB，删除存在时间 ≥ 6 分钟的 msg_id 映射
func cleanExpiredMsgIDs() {
	initNewDBs()
	cutoff := time.Now().Add(-6 * time.Minute).Unix()

	_ = msgDB.Update(func(tx *bbolt.Tx) error {
		b := tx.Bucket([]byte(MsgBucketName))
		if b == nil {
			return nil
		}

		tsPrefix := []byte("ts:")
		c := b.Cursor()
		for k, v := c.Seek(tsPrefix); k != nil && bytes.HasPrefix(k, tsPrefix); k, v = c.Next() {
			ts := int64(binary.BigEndian.Uint64(v))
			if ts < cutoff {
				// 从时间戳 key "ts:row-123" 中提取反向 key "row-123"
				revKey := k[len(tsPrefix):]
				// 读取反向 key 获取前向 key
				if fwdKey := b.Get(revKey); fwdKey != nil {
					b.Delete(fwdKey) // 删除前向映射
				}
				b.Delete(revKey) // 删除反向映射
				b.Delete(k)      // 删除时间戳
				mylog.Printf("[idmap] msg_id 缓存过期删除: %s", string(revKey))
			}
		}

		// 清理 latest_by_group_user / latest_expires_at 中的过期条目
		latestB := tx.Bucket([]byte(latestMsgBucket))
		latestExpB := tx.Bucket([]byte(latestMsgExpBucket))
		var expiredLatest [][]byte
		_ = latestExpB.ForEach(func(k, v []byte) error {
			exp := readInt64(v)
			if exp > 0 && exp < time.Now().Unix() {
				expiredLatest = append(expiredLatest, append([]byte(nil), k...))
			}
			return nil
		})
		for _, key := range expiredLatest {
			_ = latestB.Delete(key)
			_ = latestExpB.Delete(key)
		}
		return nil
	})
}

func closeNewDBs() {
	if identityDB != nil {
		identityDB.Close()
	}
	if msgDB != nil {
		msgDB.Close()
	}
}

func putInt64(bucket *bbolt.Bucket, key []byte, value int64) error {
	buf := make([]byte, 8)
	binary.BigEndian.PutUint64(buf, uint64(value))
	return bucket.Put(key, buf)
}

func readInt64(value []byte) int64 {
	if len(value) != 8 {
		return 0
	}
	return int64(binary.BigEndian.Uint64(value))
}

// ---------------------------------------------------------------------------
// Latest message tracking (needed by handlers/delete_group_msg.go)
// ---------------------------------------------------------------------------

// StoreLatestMsgID records the latest cloud message for a user in a group.
func StoreLatestMsgID(groupOpenID, userOpenID, realMsgID string) {
	if groupOpenID == "" || userOpenID == "" || realMsgID == "" {
		return
	}
	initNewDBs()
	expires := time.Now().Unix() + int64(config.GetMsgIDTTLSeconds())
	key := []byte(groupOpenID + ":" + userOpenID)
	if err := msgDB.Update(func(tx *bbolt.Tx) error {
		if err := tx.Bucket([]byte(latestMsgBucket)).Put(key, []byte(realMsgID)); err != nil {
			return err
		}
		return putInt64(tx.Bucket([]byte(latestMsgExpBucket)), key, expires)
	}); err != nil {
		mylog.Printf("[idmap] latest message store failed: group=%s user=%s error=%v", groupOpenID, userOpenID, err)
	}
}

// GetLatestMsgID returns the latest unexpired cloud message for a user in a group.
func GetLatestMsgID(groupOpenID, userOpenID string) (string, error) {
	initNewDBs()
	key := []byte(groupOpenID + ":" + userOpenID)
	var realMsgID string
	err := msgDB.View(func(tx *bbolt.Tx) error {
		exp := readInt64(tx.Bucket([]byte(latestMsgExpBucket)).Get(key))
		if exp == 0 || exp < time.Now().Unix() {
			return ErrKeyNotFound
		}
		value := tx.Bucket([]byte(latestMsgBucket)).Get(key)
		if value == nil {
			return ErrKeyNotFound
		}
		realMsgID = string(value)
		return nil
	})
	if err != nil || realMsgID == "" {
		return "", ErrKeyNotFound
	}
	return realMsgID, nil
}

// StoreLatestBotMsgID records the latest message sent by the QQ Bot itself in a group.
func StoreLatestBotMsgID(groupOpenID, realMsgID string) {
	StoreLatestMsgID(groupOpenID, latestBotMessageUserKey, realMsgID)
}

// GetLatestBotMsgID returns the latest unexpired message sent by the QQ Bot itself in a group.
func GetLatestBotMsgID(groupOpenID string) (string, error) {
	return GetLatestMsgID(groupOpenID, latestBotMessageUserKey)
}

package idmap

import (
	"bytes"
	"encoding/base32"
	"encoding/binary"
	"fmt"
	"strconv"
	"strings"
	"sync"
	"time"
	"unicode"

	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"go.etcd.io/bbolt"
)

const (
	IdentityDBName = "openid-map.db"
	MsgDBName      = "msgid-map.db"

	metaBucket           = "meta"
	identityToVuinBucket = "identity_to_vuin"
	vuinToIdentityBucket = "vuin_to_identity"
	lastSeenBucket       = "last_seen"

	msgMetaBucket      = "meta"
	msgToVirtualBucket = "msg_to_virtual"
	virtualToMsgBucket = "virtual_to_msg"
	msgExpiresBucket   = "expires_at"
	latestMsgBucket    = "latest_by_group_user"
	latestMsgExpBucket = "latest_expires_at"

	identityCounterKey   = "current_vuin"
	msgCounterKey        = "current_msg"
	migrationMarkerKey   = "legacy_idmap_migrated_v2"
	schemaVersionKey     = "schema_version"
	currentSchemaVersion = "2"
	defaultLastSeenScope = "default"
	legacyCompositeParts = 2
	legacyIsolatedParts  = 4

	latestBotMessageUserKey = "\x00qq_bot_self"
)

var (
	identityDB *bbolt.DB
	msgDB      *bbolt.DB
	newDBOnce  sync.Once
)

type normalizedIdentity struct {
	Key      string
	Raw      string
	Kind     string
	Platform string
}

type legacyMigrationStats struct {
	MaxVuin          int64
	ForwardEntries   int
	ReverseEntries   int
	CompositeEntries int
}

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

		for _, bucket := range []string{
			metaBucket,
			identityToVuinBucket,
			vuinToIdentityBucket,
			lastSeenBucket,
			ConfigBucket,
			UserInfoBucket,
		} {
			if err := ensureBucket(identityDB, bucket); err != nil {
				mylog.Fatalf("Error creating bucket %s in %s: %v", bucket, IdentityDBName, err)
			}
		}

		for _, bucket := range []string{
			msgMetaBucket,
			msgToVirtualBucket,
			virtualToMsgBucket,
			msgExpiresBucket,
			latestMsgBucket,
			latestMsgExpBucket,
		} {
			if err := ensureBucket(msgDB, bucket); err != nil {
				mylog.Fatalf("Error creating bucket %s in %s: %v", bucket, MsgDBName, err)
			}
		}

		_ = identityDB.Update(func(tx *bbolt.Tx) error {
			return tx.Bucket([]byte(metaBucket)).Put([]byte(schemaVersionKey), []byte(currentSchemaVersion))
		})

		mylog.Printf("idmap 数据库已就绪: %s, %s", IdentityDBName, MsgDBName)
		startMsgCleanup()
	})
}

func ensureBucket(database *bbolt.DB, bucket string) error {
	return database.Update(func(tx *bbolt.Tx) error {
		_, err := tx.CreateBucketIfNotExists([]byte(bucket))
		return err
	})
}

func StartMigration() {
	initNewDBs()
	if !hasOldDB() || isMigrationComplete() {
		return
	}
	if err := migrateLegacyIDMap(); err != nil {
		mylog.Fatalf("[idmap] legacy idmap migration failed: %v", err)
	}
}

func hasOldDB() bool {
	return db != nil
}

func isMigrationComplete() bool {
	initNewDBs()
	var done bool
	_ = identityDB.View(func(tx *bbolt.Tx) error {
		done = tx.Bucket([]byte(metaBucket)).Get([]byte(migrationMarkerKey)) != nil
		return nil
	})
	return done
}

func markMigrationComplete(maxVuin int64) error {
	return identityDB.Update(func(tx *bbolt.Tx) error {
		meta := tx.Bucket([]byte(metaBucket))
		if err := putInt64(meta, []byte(identityCounterKey), maxVuin); err != nil {
			return err
		}
		if err := meta.Put([]byte(schemaVersionKey), []byte(currentSchemaVersion)); err != nil {
			return err
		}
		return meta.Put([]byte(migrationMarkerKey), []byte(time.Now().Format(time.RFC3339)))
	})
}

func migrateLegacyIDMap() error {
	mylog.Printf("[idmap] 检测到旧 %s，开始一次性转换到 %s / %s", DBName, IdentityDBName, MsgDBName)

	stats, err := migrateLegacyIdentities()
	if err != nil {
		return err
	}
	mylog.Printf(
		"[idmap] 旧 ids 转换完成: forward=%d reverse=%d composite=%d max_vuin=%d",
		stats.ForwardEntries,
		stats.ReverseEntries,
		stats.CompositeEntries,
		stats.MaxVuin,
	)
	if err := copyLegacyBucket(ConfigBucket, identityDB, ConfigBucket); err != nil {
		return err
	}
	if err := copyLegacyBucket(UserInfoBucket, identityDB, UserInfoBucket); err != nil {
		return err
	}
	if msgMax, err := syncLegacyMsgCounter(); err == nil && msgMax > 0 {
		mylog.Printf("[idmap] 已同步旧 message_id counter: %d", msgMax)
	}
	if err := markMigrationComplete(stats.MaxVuin); err != nil {
		return err
	}

	mylog.Printf("[idmap] 旧库转换完成，最大 vUIN=%d；旧 %s 已保留，可确认稳定后手动备份或删除", stats.MaxVuin, DBName)
	return nil
}

func migrateLegacyIdentities() (legacyMigrationStats, error) {
	var stats legacyMigrationStats
	err := db.View(func(tx *bbolt.Tx) error {
		b := tx.Bucket([]byte(BucketName))
		if b == nil {
			return nil
		}
		return b.ForEach(func(k, v []byte) error {
			key := string(k)
			if key == CounterKey {
				if len(v) == 8 {
					stats.MaxVuin = max(stats.MaxVuin, int64(binary.BigEndian.Uint64(v)))
				}
				return nil
			}

			if strings.HasPrefix(key, "row-") {
				vuin, ok := parseLegacyRowKey(key)
				if !ok {
					return nil
				}
				stats.MaxVuin = max(stats.MaxVuin, vuin)
				stats.ReverseEntries++
				return bindIdentityToVuinInternal(string(v), vuin)
			}

			if len(v) == 8 {
				vuin := int64(binary.BigEndian.Uint64(v))
				stats.MaxVuin = max(stats.MaxVuin, vuin)
				stats.ForwardEntries++
				return bindIdentityToVuinInternal(key, vuin)
			}

			if ids, vuins, ok := parseLegacyComposite(key, string(v)); ok {
				for i := range ids {
					stats.MaxVuin = max(stats.MaxVuin, vuins[i])
					stats.CompositeEntries++
					if err := bindIdentityToVuinInternal(ids[i], vuins[i]); err != nil {
						return err
					}
				}
			}
			return nil
		})
	})
	return stats, err
}

func parseLegacyRowKey(key string) (int64, bool) {
	row := strings.TrimPrefix(key, "row-")
	if idx := strings.LastIndex(row, ":"); idx >= 0 {
		row = row[idx+1:]
	}
	vuin, err := strconv.ParseInt(row, 10, 64)
	return vuin, err == nil && vuin > 0
}

func parseLegacyComposite(key, value string) ([]string, []int64, bool) {
	valueParts := strings.Split(value, ":")
	if len(valueParts) != legacyCompositeParts || !isDigits(valueParts[0]) || !isDigits(valueParts[1]) {
		return nil, nil, false
	}

	keyParts := strings.Split(key, ":")
	var ids []string
	switch len(keyParts) {
	case legacyCompositeParts:
		ids = []string{keyParts[0], keyParts[1]}
	case legacyIsolatedParts:
		ids = []string{keyParts[1], keyParts[3]}
	default:
		return nil, nil, false
	}

	first, err1 := strconv.ParseInt(valueParts[0], 10, 64)
	second, err2 := strconv.ParseInt(valueParts[1], 10, 64)
	if err1 != nil || err2 != nil {
		return nil, nil, false
	}
	return ids, []int64{first, second}, true
}

func copyLegacyBucket(oldBucket string, newDB *bbolt.DB, newBucket string) error {
	if !hasOldDB() {
		return nil
	}
	return newDB.Update(func(newTx *bbolt.Tx) error {
		newB := newTx.Bucket([]byte(newBucket))
		return db.View(func(oldTx *bbolt.Tx) error {
			oldB := oldTx.Bucket([]byte(oldBucket))
			if oldB == nil {
				return nil
			}
			return oldB.ForEach(func(k, v []byte) error {
				if newB.Get(k) != nil {
					return nil
				}
				return newB.Put(k, v)
			})
		})
	})
}

func syncLegacyMsgCounter() (int64, error) {
	if !hasOldDB() {
		return 0, nil
	}

	var counter int64
	err := db.View(func(tx *bbolt.Tx) error {
		b := tx.Bucket([]byte(CacheBucketName))
		if b == nil {
			return nil
		}
		if v := b.Get([]byte(CounterKey)); len(v) == 8 {
			counter = int64(binary.BigEndian.Uint64(v))
		}
		return nil
	})
	if err != nil || counter <= 0 {
		return counter, err
	}

	err = msgDB.Update(func(tx *bbolt.Tx) error {
		meta := tx.Bucket([]byte(msgMetaBucket))
		existing := readInt64(meta.Get([]byte(msgCounterKey)))
		if existing >= counter {
			return nil
		}
		return putInt64(meta, []byte(msgCounterKey), counter)
	})
	return counter, err
}

func storeIdentity(raw string) (int64, error) {
	initNewDBs()
	identity := normalizeIdentity(raw)

	var vuin int64
	created := false
	err := identityDB.Update(func(tx *bbolt.Tx) error {
		idB := tx.Bucket([]byte(identityToVuinBucket))
		if existing := idB.Get([]byte(identity.Key)); len(existing) == 8 {
			vuin = int64(binary.BigEndian.Uint64(existing))
			return touchLastSeenTx(tx, vuin, identity)
		}

		next, err := allocateVuinTx(tx, raw)
		if err != nil {
			return err
		}
		vuin = next
		created = true
		return putIdentityMappingTx(tx, identity, vuin)
	})
	if err == nil {
		if created {
			mylog.Printf("[idmap] identity create: %s -> vUIN=%d", identityLogValue(identity), vuin)
		} else {
			mylog.Printf("[idmap] identity hit: %s -> vUIN=%d", identityLogValue(identity), vuin)
		}
	}
	return vuin, err
}

func bindIdentityToVuinInternal(raw string, vuin int64) error {
	initNewDBs()
	identity := normalizeIdentity(raw)
	return identityDB.Update(func(tx *bbolt.Tx) error {
		return putIdentityMappingTx(tx, identity, vuin)
	})
}

func bindVuin(oldVuin, newVuin int64) error {
	initNewDBs()
	if oldVuin == newVuin {
		mylog.Printf("[idmap] bind skipped: old_vuin=%d new_vuin=%d", oldVuin, newVuin)
		return nil
	}

	movedCount := 0
	err := identityDB.Update(func(tx *bbolt.Tx) error {
		revB := tx.Bucket([]byte(vuinToIdentityBucket))
		prefix := []byte(vuinPrefix(oldVuin))
		var moved []normalizedIdentity

		c := revB.Cursor()
		for k, v := c.Seek(prefix); k != nil && bytes.HasPrefix(k, prefix); k, v = c.Next() {
			identityKey := strings.TrimPrefix(string(k), string(prefix))
			moved = append(moved, normalizedIdentity{Key: identityKey, Raw: string(v)})
		}
		if len(moved) == 0 {
			return ErrKeyNotFound
		}
		for _, identity := range moved {
			if err := revB.Delete([]byte(vuinIdentityKey(oldVuin, identity.Key))); err != nil {
				return err
			}
			if err := putIdentityMappingTx(tx, identity, newVuin); err != nil {
				return err
			}
		}
		movedCount = len(moved)
		_ = tx.Bucket([]byte(lastSeenBucket)).Delete([]byte(lastSeenKey(oldVuin)))
		return nil
	})
	if err != nil {
		mylog.Printf("[idmap] bind failed: old_vuin=%d new_vuin=%d error=%v", oldVuin, newVuin, err)
		return err
	}
	if movedCount > 0 {
		mylog.Printf("[idmap] bind moved: old_vuin=%d new_vuin=%d identities=%d", oldVuin, newVuin, movedCount)
	}
	return nil
}

func lookupVirtualIdentity(raw string) (int64, bool) {
	initNewDBs()
	identity := normalizeIdentity(raw)
	var vuin int64
	_ = identityDB.View(func(tx *bbolt.Tx) error {
		v := tx.Bucket([]byte(identityToVuinBucket)).Get([]byte(identity.Key))
		if len(v) == 8 {
			vuin = int64(binary.BigEndian.Uint64(v))
		}
		return nil
	})
	return vuin, vuin > 0
}

func retrieveIdentity(virtualID string) (string, error) {
	initNewDBs()
	vuin, err := strconv.ParseInt(virtualID, 10, 64)
	if err != nil || vuin <= 0 {
		mylog.Printf("[idmap] identity lookup invalid: vUIN=%s", virtualID)
		return "", ErrKeyNotFound
	}

	var selected string
	err = identityDB.View(func(tx *bbolt.Tx) error {
		if raw := readLastSeenTx(tx, vuin); raw != "" {
			identity := normalizeIdentity(raw)
			if mapped := tx.Bucket([]byte(identityToVuinBucket)).Get([]byte(identity.Key)); readInt64(mapped) == vuin {
				selected = raw
				return nil
			}
		}

		revB := tx.Bucket([]byte(vuinToIdentityBucket))
		prefix := []byte(vuinPrefix(vuin))
		var fallback string
		c := revB.Cursor()
		for k, v := c.Seek(prefix); k != nil && bytes.HasPrefix(k, prefix); k, v = c.Next() {
			key := strings.TrimPrefix(string(k), string(prefix))
			raw := string(v)
			if fallback == "" {
				fallback = raw
			}
			if strings.HasPrefix(key, "openid:") || (len(raw) == 32 && !strings.HasPrefix(raw, "rUIN-")) {
				selected = raw
				return nil
			}
		}
		selected = fallback
		return nil
	})
	if err != nil || selected == "" {
		mylog.Printf("[idmap] identity lookup miss: vUIN=%s", virtualID)
		return "", ErrKeyNotFound
	}
	mylog.Printf("[idmap] identity lookup hit: vUIN=%s -> %s", virtualID, selected)
	return selected, nil
}

func putIdentityMappingTx(tx *bbolt.Tx, identity normalizedIdentity, vuin int64) error {
	idB := tx.Bucket([]byte(identityToVuinBucket))
	revB := tx.Bucket([]byte(vuinToIdentityBucket))

	if existing := idB.Get([]byte(identity.Key)); len(existing) == 8 {
		oldVuin := int64(binary.BigEndian.Uint64(existing))
		if oldVuin != vuin {
			_ = revB.Delete([]byte(vuinIdentityKey(oldVuin, identity.Key)))
		}
	}

	if err := putInt64(idB, []byte(identity.Key), vuin); err != nil {
		return err
	}
	if err := revB.Put([]byte(vuinIdentityKey(vuin, identity.Key)), []byte(identity.Raw)); err != nil {
		return err
	}
	return touchLastSeenTx(tx, vuin, identity)
}

func allocateVuinTx(tx *bbolt.Tx, raw string) (int64, error) {
	meta := tx.Bucket([]byte(metaBucket))
	revB := tx.Bucket([]byte(vuinToIdentityBucket))

	if config.GetHashIDValue() {
		for digits := 9; digits <= 18; digits++ {
			vuin, err := GenerateRowID(raw, digits)
			if err != nil {
				return 0, err
			}
			if !vuinExistsTx(revB, vuin) {
				if vuin > readInt64(meta.Get([]byte(identityCounterKey))) {
					if err := putInt64(meta, []byte(identityCounterKey), vuin); err != nil {
						return 0, err
					}
				}
				return vuin, nil
			}
		}
		return 0, fmt.Errorf("unable to find unique vUIN")
	}

	next := readInt64(meta.Get([]byte(identityCounterKey))) + 1
	if next <= 0 {
		next = 1
	}
	for vuinExistsTx(revB, next) {
		next++
	}
	if err := putInt64(meta, []byte(identityCounterKey), next); err != nil {
		return 0, err
	}
	return next, nil
}

func vuinExistsTx(revB *bbolt.Bucket, vuin int64) bool {
	prefix := []byte(vuinPrefix(vuin))
	k, _ := revB.Cursor().Seek(prefix)
	return k != nil && bytes.HasPrefix(k, prefix)
}

func touchLastSeenTx(tx *bbolt.Tx, vuin int64, identity normalizedIdentity) error {
	return tx.Bucket([]byte(lastSeenBucket)).Put([]byte(lastSeenKey(vuin)), []byte(identity.Raw))
}

func readLastSeenTx(tx *bbolt.Tx, vuin int64) string {
	if raw := tx.Bucket([]byte(lastSeenBucket)).Get([]byte(lastSeenKey(vuin))); raw != nil {
		return string(raw)
	}
	return ""
}

func lastSeenKey(vuin int64) string {
	return fmt.Sprintf("%s:%d", defaultLastSeenScope, vuin)
}

func normalizeIdentity(raw string) normalizedIdentity {
	raw = stripUinPrefix(raw)
	if identity, ok := parseRUIN(raw); ok {
		return identity
	}
	if len(raw) == 32 {
		return normalizedIdentity{
			Key:      "openid:QQ:" + config.GetAppIDStr() + ":" + raw,
			Raw:      raw,
			Kind:     "openid",
			Platform: "QQ",
		}
	}
	return normalizedIdentity{
		Key:  "raw:" + raw,
		Raw:  raw,
		Kind: "raw",
	}
}

func identityLogValue(identity normalizedIdentity) string {
	if identity.Platform == "" {
		return fmt.Sprintf("kind=%s raw=%s key=%s", identity.Kind, identity.Raw, identity.Key)
	}
	return fmt.Sprintf("kind=%s platform=%s raw=%s key=%s", identity.Kind, identity.Platform, identity.Raw, identity.Key)
}

func parseRUIN(raw string) (normalizedIdentity, bool) {
	if !strings.HasPrefix(strings.ToLower(raw), "ruin-") {
		return normalizedIdentity{}, false
	}
	parts := strings.SplitN(raw, "-", 4)
	if len(parts) != 4 {
		return normalizedIdentity{}, false
	}

	platform := parts[1]
	index := parts[2]
	id := parts[3]
	if platform == "QQ" && !isDigits(id) {
		if decoded, ok := decodeBase32ID(id); ok {
			id = decoded
		}
	}
	return normalizedIdentity{
		Key:      "ruin:" + platform + ":" + index + ":" + id,
		Raw:      fmt.Sprintf("rUIN-%s-%s-%s", platform, index, id),
		Kind:     "ruin",
		Platform: platform,
	}, true
}

func decodeBase32ID(id string) (string, bool) {
	encoding := base32.StdEncoding.WithPadding(base32.NoPadding)
	decoded, err := encoding.DecodeString(strings.ToUpper(id))
	if err != nil {
		return "", false
	}
	out := string(decoded)
	if out == "" || !isDigits(out) {
		return "", false
	}
	return out, true
}

func StoreGroupID(groupOpenID string) (int64, error) {
	return storeIdentity(groupOpenID)
}

func StoreUserID(userOpenID string) (int64, error) {
	return storeIdentity(userOpenID)
}

func RetrieveGroupID(virtualID string) (string, error) {
	return retrieveIdentity(virtualID)
}

func RetrieveUserID(virtualID string) (string, error) {
	return retrieveIdentity(virtualID)
}

func StoreMsgID(realMsgID string) (int64, error) {
	initNewDBs()
	now := time.Now().Unix()
	expires := now + int64(config.GetMsgIDTTLSeconds())
	if expires <= now {
		expires = now + 3600
	}

	var virtualID int64
	created := false
	err := msgDB.Update(func(tx *bbolt.Tx) error {
		msgB := tx.Bucket([]byte(msgToVirtualBucket))
		revB := tx.Bucket([]byte(virtualToMsgBucket))
		expB := tx.Bucket([]byte(msgExpiresBucket))

		if existing := msgB.Get([]byte(realMsgID)); len(existing) == 8 {
			virtualID = int64(binary.BigEndian.Uint64(existing))
			if err := putInt64(expB, []byte(strconv.FormatInt(virtualID, 10)), expires); err != nil {
				return err
			}
			return revB.Put([]byte(strconv.FormatInt(virtualID, 10)), []byte(realMsgID))
		}

		next, err := allocateMsgIDTx(tx)
		if err != nil {
			return err
		}
		virtualID = next
		created = true
		if err := putInt64(msgB, []byte(realMsgID), virtualID); err != nil {
			return err
		}
		if err := revB.Put([]byte(strconv.FormatInt(virtualID, 10)), []byte(realMsgID)); err != nil {
			return err
		}
		return putInt64(expB, []byte(strconv.FormatInt(virtualID, 10)), expires)
	})
	if err == nil {
		if created {
			mylog.Printf("[idmap] msg_id create: real=%s -> virtual=%d ttl_seconds=%d", realMsgID, virtualID, expires-now)
		} else {
			mylog.Printf("[idmap] msg_id hit: real=%s -> virtual=%d ttl_refreshed_seconds=%d", realMsgID, virtualID, expires-now)
		}
	}
	return virtualID, err
}

func RetrieveMsgID(virtualID string) (string, error) {
	initNewDBs()
	var real string
	now := time.Now().Unix()
	err := msgDB.View(func(tx *bbolt.Tx) error {
		expB := tx.Bucket([]byte(msgExpiresBucket))
		if exp := readInt64(expB.Get([]byte(virtualID))); exp > 0 && exp < now {
			mylog.Printf("[idmap] msg_id lookup expired: virtual=%s expired_at=%d now=%d", virtualID, exp, now)
			return ErrKeyNotFound
		}
		if v := tx.Bucket([]byte(virtualToMsgBucket)).Get([]byte(virtualID)); v != nil {
			real = string(v)
			return nil
		}
		return ErrKeyNotFound
	})
	if err != nil || real == "" {
		mylog.Printf("[idmap] msg_id lookup miss: virtual=%s", virtualID)
		return "", ErrKeyNotFound
	}
	mylog.Printf("[idmap] msg_id lookup hit: virtual=%s -> real=%s", virtualID, real)
	return real, nil
}

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

func CleanMsgDB() error {
	initNewDBs()
	err := msgDB.Update(func(tx *bbolt.Tx) error {
		for _, bucket := range []string{
			msgToVirtualBucket,
			virtualToMsgBucket,
			msgExpiresBucket,
			latestMsgBucket,
			latestMsgExpBucket,
		} {
			if err := recreateBucket(tx, bucket); err != nil {
				return err
			}
		}
		return tx.Bucket([]byte(msgMetaBucket)).Delete([]byte(msgCounterKey))
	})
	if err == nil {
		mylog.Printf("[idmap] msg_id cache cleaned: %s", MsgDBName)
	}
	return err
}

func allocateMsgIDTx(tx *bbolt.Tx) (int64, error) {
	meta := tx.Bucket([]byte(msgMetaBucket))
	revB := tx.Bucket([]byte(virtualToMsgBucket))
	next := readInt64(meta.Get([]byte(msgCounterKey))) + 1
	if next <= 0 {
		next = 1
	}
	for revB.Get([]byte(strconv.FormatInt(next, 10))) != nil {
		next++
	}
	if err := putInt64(meta, []byte(msgCounterKey), next); err != nil {
		return 0, err
	}
	return next, nil
}

func recreateBucket(tx *bbolt.Tx, bucket string) error {
	_ = tx.DeleteBucket([]byte(bucket))
	_, err := tx.CreateBucket([]byte(bucket))
	return err
}

func newDBStore(openID string, virtualID int64) {
	_ = bindIdentityToVuinInternal(openID, virtualID)
}

func newDBLookup(virtualID string) (string, bool) {
	id, err := retrieveIdentity(virtualID)
	return id, err == nil
}

func newDBMsgStore(realMsgID string, virtualID int64) {
	initNewDBs()
	expires := time.Now().Unix() + int64(config.GetMsgIDTTLSeconds())
	_ = msgDB.Update(func(tx *bbolt.Tx) error {
		if err := putInt64(tx.Bucket([]byte(msgToVirtualBucket)), []byte(realMsgID), virtualID); err != nil {
			return err
		}
		if err := tx.Bucket([]byte(virtualToMsgBucket)).Put([]byte(strconv.FormatInt(virtualID, 10)), []byte(realMsgID)); err != nil {
			return err
		}
		return putInt64(tx.Bucket([]byte(msgExpiresBucket)), []byte(strconv.FormatInt(virtualID, 10)), expires)
	})
}

func newDBMsgLookup(virtualID string) (string, bool) {
	id, err := RetrieveMsgID(virtualID)
	return id, err == nil
}

func configAndUserInfoDB() *bbolt.DB {
	initNewDBs()
	return identityDB
}

func startMsgCleanup() {
	go func() {
		for {
			time.Sleep(time.Minute)
			cleanExpiredMsgIDs()
		}
	}()
}

func cleanExpiredMsgIDs() {
	initNewDBs()
	now := time.Now().Unix()
	_ = msgDB.Update(func(tx *bbolt.Tx) error {
		msgB := tx.Bucket([]byte(msgToVirtualBucket))
		revB := tx.Bucket([]byte(virtualToMsgBucket))
		expB := tx.Bucket([]byte(msgExpiresBucket))

		var expired []string
		_ = expB.ForEach(func(k, v []byte) error {
			if exp := readInt64(v); exp > 0 && exp < now {
				expired = append(expired, string(k))
			}
			return nil
		})

		for _, virtualID := range expired {
			if real := revB.Get([]byte(virtualID)); real != nil {
				_ = msgB.Delete(real)
			}
			_ = revB.Delete([]byte(virtualID))
			_ = expB.Delete([]byte(virtualID))
		}

		latestB := tx.Bucket([]byte(latestMsgBucket))
		latestExpB := tx.Bucket([]byte(latestMsgExpBucket))
		var expiredLatest [][]byte
		_ = latestExpB.ForEach(func(k, v []byte) error {
			if exp := readInt64(v); exp > 0 && exp < now {
				expiredLatest = append(expiredLatest, append([]byte(nil), k...))
			}
			return nil
		})
		for _, key := range expiredLatest {
			_ = latestB.Delete(key)
			_ = latestExpB.Delete(key)
		}
		if len(expired) > 0 {
			mylog.Printf("[idmap] msg_id expired cleanup: %d entries", len(expired))
		}
		return nil
	})
}

func closeNewDBs() {
	if identityDB != nil {
		_ = identityDB.Close()
	}
	if msgDB != nil {
		_ = msgDB.Close()
	}
}

func vuinPrefix(vuin int64) string {
	return strconv.FormatInt(vuin, 10) + ":"
}

func vuinIdentityKey(vuin int64, identityKey string) string {
	return vuinPrefix(vuin) + identityKey
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

func isDigits(value string) bool {
	if value == "" {
		return false
	}
	for _, r := range value {
		if !unicode.IsDigit(r) {
			return false
		}
	}
	return true
}

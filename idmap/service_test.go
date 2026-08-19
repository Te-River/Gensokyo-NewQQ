package idmap

import (
	"encoding/binary"
	"os"
	"testing"
	"time"

	"go.etcd.io/bbolt"
)

// TestListAllIdentitiesBasic 测试批量导出身份映射基本功能
func TestListAllIdentitiesBasic(t *testing.T) {
	// 保存原始 identityDB
	origDB := identityDB
	defer func() { identityDB = origDB }()

	// 创建临时数据库
	tmpFile := "test-list-identities.db"
	db, err := bbolt.Open(tmpFile, 0600, nil)
	if err != nil {
		t.Fatalf("Failed to open test DB: %v", err)
	}
	defer func() {
		db.Close()
		os.Remove(tmpFile)
	}()

	// 创建 bucket
	err = db.Update(func(tx *bbolt.Tx) error {
		_, err := tx.CreateBucketIfNotExists([]byte(IdentityBucketName))
		return err
	})
	if err != nil {
		t.Fatalf("Failed to create bucket: %v", err)
	}

	// 设置全局 identityDB 为测试数据库
	identityDB = db

	// 插入测试数据
	testData := []struct {
		key       string
		virtualID int64
	}{
		{"openid1", 10001},
		{"openid2", 10002},
		{"openid3", 10003},
	}

	for _, td := range testData {
		err = db.Update(func(tx *bbolt.Tx) error {
			b := tx.Bucket([]byte(IdentityBucketName))
			rowBytes := make([]byte, 8)
			binary.BigEndian.PutUint64(rowBytes, uint64(td.virtualID))
			return b.Put([]byte(td.key), rowBytes)
		})
		if err != nil {
			t.Fatalf("Failed to insert test data: %v", err)
		}
	}

	// 测试 ListAllIdentities
	snapshot, err := ListAllIdentities()
	if err != nil {
		t.Fatalf("ListAllIdentities failed: %v", err)
	}

	// 验证结果
	if snapshot == nil {
		t.Fatal("Snapshot is nil")
	}

	if snapshot.Count != 3 {
		t.Errorf("Expected 3 mappings, got %d", snapshot.Count)
	}

	if len(snapshot.Mappings) != 3 {
		t.Errorf("Expected 3 mappings in slice, got %d", len(snapshot.Mappings))
	}

	// 验证时间戳
	if snapshot.Timestamp == 0 {
		t.Error("Timestamp should not be zero")
	}

	if snapshot.Timestamp > time.Now().Unix() {
		t.Error("Timestamp should not be in the future")
	}

	// 验证映射内容
	foundIDs := make(map[string]int64)
	for _, m := range snapshot.Mappings {
		foundIDs[m.RealID] = m.VirtualID
	}

	for _, td := range testData {
		if vuin, ok := foundIDs[td.key]; !ok {
			t.Errorf("Expected ID %s not found", td.key)
		} else if vuin != td.virtualID {
			t.Errorf("ID %s: expected vUIN %d, got %d", td.key, td.virtualID, vuin)
		}
	}
}

// TestListAllIdentitiesSkipsNonForward 测试跳过非正向映射
func TestListAllIdentitiesSkipsNonForward(t *testing.T) {
	// 保存原始 identityDB
	origDB := identityDB
	defer func() { identityDB = origDB }()

	// 创建临时数据库
	tmpFile := "test-list-skip.db"
	db, err := bbolt.Open(tmpFile, 0600, nil)
	if err != nil {
		t.Fatalf("Failed to open test DB: %v", err)
	}
	defer func() {
		db.Close()
		os.Remove(tmpFile)
	}()

	// 创建 bucket
	err = db.Update(func(tx *bbolt.Tx) error {
		_, err := tx.CreateBucketIfNotExists([]byte(IdentityBucketName))
		return err
	})
	if err != nil {
		t.Fatalf("Failed to create bucket: %v", err)
	}

	// 设置全局 identityDB 为测试数据库
	identityDB = db

	// 插入各种应该被跳过的条目
	err = db.Update(func(tx *bbolt.Tx) error {
		b := tx.Bucket([]byte(IdentityBucketName))
		rowBytes := make([]byte, 8)
		binary.BigEndian.PutUint64(rowBytes, uint64(10001))

		// 正向映射（应该被包含）
		b.Put([]byte("openid1"), rowBytes)

		// 反向映射（应该被跳过）
		b.Put([]byte("row-10001"), []byte("openid1"))
		b.Put([]byte("uin:row-10002"), []byte("uin:openid2"))

		// 计数器（应该被跳过）
		b.Put([]byte(IdentityCounterKey), rowBytes)
		b.Put([]byte(CounterKey), rowBytes)

		// 迁移标记（应该被跳过）
		b.Put([]byte(migrationMarkerKey), []byte("1"))

		// 复合键（应该被跳过）
		b.Put([]byte("group1:user1"), rowBytes)

		// 非 8 字节值（应该被跳过）
		b.Put([]byte("openid_short"), []byte("short"))

		return nil
	})
	if err != nil {
		t.Fatalf("Failed to insert test data: %v", err)
	}

	// 测试 ListAllIdentities
	snapshot, err := ListAllIdentities()
	if err != nil {
		t.Fatalf("ListAllIdentities failed: %v", err)
	}

	// 验证结果：应该只包含 1 条正向映射
	if snapshot.Count != 1 {
		t.Errorf("Expected 1 mapping, got %d", snapshot.Count)
	}

	if len(snapshot.Mappings) != 1 {
		t.Errorf("Expected 1 mapping in slice, got %d", len(snapshot.Mappings))
	}

	if snapshot.Mappings[0].RealID != "openid1" {
		t.Errorf("Expected RealID 'openid1', got '%s'", snapshot.Mappings[0].RealID)
	}
}

// TestListAllIdentitiesEmpty 测试空数据库
func TestListAllIdentitiesEmpty(t *testing.T) {
	// 保存原始 identityDB
	origDB := identityDB
	defer func() { identityDB = origDB }()

	// 创建临时数据库
	tmpFile := "test-list-empty.db"
	db, err := bbolt.Open(tmpFile, 0600, nil)
	if err != nil {
		t.Fatalf("Failed to open test DB: %v", err)
	}
	defer func() {
		db.Close()
		os.Remove(tmpFile)
	}()

	// 创建 bucket
	err = db.Update(func(tx *bbolt.Tx) error {
		_, err := tx.CreateBucketIfNotExists([]byte(IdentityBucketName))
		return err
	})
	if err != nil {
		t.Fatalf("Failed to create bucket: %v", err)
	}

	// 设置全局 identityDB 为测试数据库
	identityDB = db

	// 测试 ListAllIdentities
	snapshot, err := ListAllIdentities()
	if err != nil {
		t.Fatalf("ListAllIdentities failed: %v", err)
	}

	// 验证结果
	if snapshot == nil {
		t.Fatal("Snapshot should not be nil")
	}

	if snapshot.Count != 0 {
		t.Errorf("Expected 0 mappings, got %d", snapshot.Count)
	}

	if len(snapshot.Mappings) != 0 {
		t.Errorf("Expected empty mappings slice, got %d items", len(snapshot.Mappings))
	}
}

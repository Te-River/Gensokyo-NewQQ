package idmap

import (
	"bytes"
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"io"
	"os"
	"path/filepath"
	"testing"

	"go.etcd.io/bbolt"
)

const testOpenID = "0123456789abcdef0123456789abcdef"

func u64Bytes(n int64) []byte {
	b := make([]byte, 8)
	binary.BigEndian.PutUint64(b, uint64(n))
	return b
}

// createDB 建库并写入 fixture 后关闭，供导出器以只读方式打开。
func createDB(t *testing.T, path string, fill func(t *testing.T, tx *bbolt.Tx)) {
	t.Helper()
	db, err := bbolt.Open(path, 0600, nil)
	if err != nil {
		t.Fatalf("打开测试库 %s: %v", path, err)
	}
	if err := db.Update(func(tx *bbolt.Tx) error {
		fill(t, tx)
		return nil
	}); err != nil {
		t.Fatalf("写入测试库 %s: %v", path, err)
	}
	if err := db.Close(); err != nil {
		t.Fatalf("关闭测试库 %s: %v", path, err)
	}
}

func put(t *testing.T, tx *bbolt.Tx, bucket, key string, value []byte) {
	t.Helper()
	b, err := tx.CreateBucketIfNotExists([]byte(bucket))
	if err != nil {
		t.Fatalf("创建桶 %s: %v", bucket, err)
	}
	if err := b.Put([]byte(key), value); err != nil {
		t.Fatalf("写入 %s/%s: %v", bucket, key, err)
	}
}

func totalRecords(records []map[string]any, kinds ...string) int {
	want := 0
	for _, kind := range kinds {
		for _, rec := range records {
			if rec["kind"] == kind {
				want++
			}
		}
	}
	return want
}

func firstOfKind(t *testing.T, records []map[string]any, kind string) map[string]any {
	t.Helper()
	for _, rec := range records {
		if rec["kind"] == kind {
			return rec
		}
	}
	t.Fatalf("导出结果缺少 kind=%s 的记录：%v", kind, records)
	return nil
}

func strField(t *testing.T, rec map[string]any, key string) string {
	t.Helper()
	value, ok := rec[key].(string)
	if !ok {
		t.Fatalf("字段 %s 不是字符串：%v", key, rec)
	}
	return value
}

func boolField(t *testing.T, rec map[string]any, key string) bool {
	t.Helper()
	value, ok := rec[key].(bool)
	if !ok {
		t.Fatalf("字段 %s 不是布尔：%v", key, rec)
	}
	return value
}

// numField 同时断言整数没有被写成浮点。
func numField(t *testing.T, rec map[string]any, key string) int64 {
	t.Helper()
	number, ok := rec[key].(json.Number)
	if !ok {
		t.Fatalf("字段 %s 不是数字：%v", key, rec)
	}
	value, err := number.Int64()
	if err != nil {
		t.Fatalf("字段 %s 不是整数(%s)：%v", key, number, rec)
	}
	return value
}

func readExport(t *testing.T, path string) ([]map[string]any, []byte) {
	t.Helper()
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("读取导出文件: %v", err)
	}
	if bytes.HasPrefix(data, []byte{0xEF, 0xBB, 0xBF}) {
		t.Fatal("导出文件带 BOM")
	}
	if bytes.Contains(data, []byte("\r\n")) {
		t.Fatal("导出文件含 CRLF，应为 LF")
	}
	var records []map[string]any
	dec := json.NewDecoder(bytes.NewReader(data))
	dec.UseNumber()
	for {
		var rec map[string]any
		if err := dec.Decode(&rec); err == io.EOF {
			break
		} else if err != nil {
			t.Fatalf("解析导出 JSONL: %v", err)
		}
		records = append(records, rec)
	}
	return records, data
}

// assertSha256 校验 end 行的 sha256 等于其之前全部字节（含行尾 \n）的摘要。
func assertSha256(t *testing.T, records []map[string]any, data []byte) {
	t.Helper()
	end := records[len(records)-1]
	if end["kind"] != "end" {
		t.Fatalf("最后一行不是 end：%v", end)
	}
	body := bytes.TrimSuffix(data, []byte("\n"))
	idx := bytes.LastIndexByte(body, '\n')
	if idx < 0 {
		t.Fatal("导出文件没有 end 之前的行")
	}
	sum := sha256.Sum256(body[:idx+1])
	if want := hex.EncodeToString(sum[:]); strField(t, end, "sha256") != want {
		t.Fatalf("sha256 不匹配：记录 %s，期望 %s", strField(t, end, "sha256"), want)
	}
}

func TestExportLegacyCounterMode(t *testing.T) {
	dir := t.TempDir()
	out := filepath.Join(dir, "out.jsonl")

	createDB(t, filepath.Join(dir, IdentityDBName), func(t *testing.T, tx *bbolt.Tx) {
		put(t, tx, IdentityBucketName, testOpenID, u64Bytes(10001))
		put(t, tx, IdentityBucketName, "row-10001", []byte(testOpenID))
		put(t, tx, IdentityBucketName, IdentityCounterKey, u64Bytes(10001))
		put(t, tx, IdentityBucketName, "svc:ops", u64Bytes(10002))      // 键含冒号
		put(t, tx, IdentityBucketName, migrationMarkerKey, []byte("1")) // 非 8 字节，跳过
		put(t, tx, ConfigBucket, "10001:type", []byte("group"))
		put(t, tx, ConfigBucket, testOpenID+":type", []byte("private"))
		put(t, tx, ConfigBucket, "stun:addr", []byte("127.0.0.1")) // stun 段丢弃
		put(t, tx, UserInfoBucket, testOpenID+":10001", []byte(`{"nickname":"n","remark":"","user_id":"10001"}`))
	})
	createDB(t, filepath.Join(dir, MsgDBName), func(t *testing.T, tx *bbolt.Tx) {
		put(t, tx, MsgBucketName, "REALMSG1", u64Bytes(5))
		put(t, tx, MsgBucketName, "row-5", []byte("REALMSG1"))
		put(t, tx, MsgBucketName, MsgCounterKey, u64Bytes(5))
		put(t, tx, MsgBucketName, "ts:1000000", u64Bytes(7))
		put(t, tx, latestMsgBucket, "GROUP1:USER1", []byte("REALMSG1"))
	})

	res, err := ExportLegacy(ExportOptions{Dir: dir, Out: out, AppID: "102000001"})
	if err != nil {
		t.Fatalf("导出失败: %v", err)
	}
	records, data := readExport(t, out)

	meta := records[0]
	if strField(t, meta, "kind") != "meta" || strField(t, meta, "format") != exportFormat || numField(t, meta, "version") != 1 {
		t.Fatalf("meta 头不符：%v", meta)
	}
	if strField(t, meta, "source_schema") != schemaLegacyV1 || strField(t, meta, "allocator") != allocatorCounter || boolField(t, meta, "hash_id") {
		t.Fatalf("meta 模式字段不符：%v", meta)
	}
	if strField(t, meta, "app_id") != "102000001" || strField(t, meta, "uin_prefix") != "" || boolField(t, meta, "idmap_isolation") {
		t.Fatalf("meta 隔离字段不符：%v", meta)
	}
	counts := meta["counts"].(map[string]any)
	if numField(t, counts, "identity") != 2 || numField(t, counts, "config") != 2 || numField(t, counts, "user_info") != 1 {
		t.Fatalf("meta counts 不符：%v", counts)
	}

	identities := records[1:3]
	if strField(t, identities[0], "key") != "openid:QQ:102000001:"+testOpenID ||
		strField(t, identities[0], "raw") != testOpenID ||
		strField(t, identities[0], "itype") != "openid" ||
		strField(t, identities[0], "platform") != "QQ" ||
		strField(t, identities[0], "app_id") != "102000001" ||
		numField(t, identities[0], "vuin") != 10001 {
		t.Fatalf("openid 身份行不符：%v", identities[0])
	}
	if strField(t, identities[1], "key") != "raw:svc:ops" || strField(t, identities[1], "itype") != "raw" || numField(t, identities[1], "vuin") != 10002 {
		t.Fatalf("冒号 raw 身份行不符：%v", identities[1])
	}

	vuinCounter := firstOfKind(t, records, "vuin_counter")
	if numField(t, vuinCounter, "counter") != 10001 || numField(t, vuinCounter, "max_vuin") != 10002 {
		t.Fatalf("vuin_counter 不符：%v", vuinCounter)
	}
	msgCounter := firstOfKind(t, records, "msg_counter")
	if numField(t, msgCounter, "counter") != 5 {
		t.Fatalf("msg_counter 不符：%v", msgCounter)
	}

	configs := []map[string]any{}
	for _, rec := range records {
		if rec["kind"] == "config" {
			configs = append(configs, rec)
		}
	}
	if len(configs) != 2 {
		t.Fatalf("config 行数不符（stun 应丢弃）：%v", configs)
	}
	// bbolt 按字节序返回，OpenID 段（数字 0 开头）排在 vUIN 段之前。
	if strField(t, configs[0], "section") != testOpenID || strField(t, configs[0], "section_kind") != "openid" || strField(t, configs[0], "value") != "private" {
		t.Fatalf("OpenID 段 config 不符：%v", configs[0])
	}
	if strField(t, configs[1], "section") != "10001" || strField(t, configs[1], "section_kind") != "vuin" || strField(t, configs[1], "key") != "type" {
		t.Fatalf("vUIN 段 config 不符：%v", configs[1])
	}

	userInfo := firstOfKind(t, records, "user_info")
	if strField(t, userInfo, "raw") != testOpenID || strField(t, userInfo, "user_id") != "10001" {
		t.Fatalf("user_info 不符：%v", userInfo)
	}
	if _, ok := userInfo["data"].(map[string]any); !ok {
		t.Fatalf("user_info.data 应内嵌 JSON 对象：%v", userInfo)
	}

	if got := totalRecords(records, "msg_id", "latest"); got != 0 {
		t.Fatalf("默认不应导出 msg_id/latest，实际 %d 条", got)
	}

	assertSha256(t, records, data)
	if res.Counts.Identity != 2 || res.Counts.Config != 2 || res.Counts.MsgID != 0 {
		t.Fatalf("返回摘要不符：%+v", res.Counts)
	}
}

func TestExportLegacyIsolationPrefix(t *testing.T) {
	dir := t.TempDir()
	out := filepath.Join(dir, "out.jsonl")
	if err := os.WriteFile(filepath.Join(dir, "config.yml"), []byte(
		"version: 1\nsettings:\n  app_id: 102000001\n  uin: 123456\n  idmap_isolation: true\n  hash_id: false\n"), 0600); err != nil {
		t.Fatalf("写 config.yml: %v", err)
	}

	createDB(t, filepath.Join(dir, IdentityDBName), func(t *testing.T, tx *bbolt.Tx) {
		put(t, tx, IdentityBucketName, "123456:"+testOpenID, u64Bytes(10001))
		put(t, tx, IdentityBucketName, "row-123456:10001", []byte("123456:"+testOpenID))
		put(t, tx, IdentityBucketName, "123456:10001", u64Bytes(999)) // 隔离别名，必须跳过
		put(t, tx, IdentityBucketName, IdentityCounterKey, u64Bytes(10001))
	})

	res, err := ExportLegacy(ExportOptions{Dir: dir, Out: out})
	if err != nil {
		t.Fatalf("导出失败: %v", err)
	}
	records, data := readExport(t, out)

	meta := records[0]
	if strField(t, meta, "app_id") != "102000001" || strField(t, meta, "uin_prefix") != "123456:" || !boolField(t, meta, "idmap_isolation") {
		t.Fatalf("meta 未反映隔离配置：%v", meta)
	}
	if got := totalRecords(records, "identity"); got != 1 {
		t.Fatalf("别名键应被跳过，identity 应为 1 条，实际 %d 条", got)
	}
	identity := firstOfKind(t, records, "identity")
	if strField(t, identity, "key") != "openid:QQ:102000001:"+testOpenID || strField(t, identity, "raw") != testOpenID {
		t.Fatalf("隔离前缀未剥离：%v", identity)
	}
	if len(res.Warnings) == 0 {
		t.Fatal("跳过的别名应产生提示")
	}
	assertSha256(t, records, data)
}

func TestExportLegacyHashMode(t *testing.T) {
	dir := t.TempDir()
	out := filepath.Join(dir, "out.jsonl")

	createDB(t, filepath.Join(dir, IdentityDBName), func(t *testing.T, tx *bbolt.Tx) {
		// hash_id 模式不写 currentRow
		put(t, tx, IdentityBucketName, testOpenID, u64Bytes(987654321))
		put(t, tx, IdentityBucketName, "row-987654321", []byte(testOpenID))
		put(t, tx, IdentityBucketName, "another-raw", u64Bytes(5))
	})

	_, err := ExportLegacy(ExportOptions{Dir: dir, Out: out})
	if err == nil {
		t.Fatal("缺 config.yml 且未给 --app-id 时应报错")
	}

	res, err := ExportLegacy(ExportOptions{Dir: dir, Out: out, AppID: "102000001"})
	if err != nil {
		t.Fatalf("导出失败: %v", err)
	}
	records, data := readExport(t, out)

	meta := records[0]
	if strField(t, meta, "allocator") != allocatorHash || !boolField(t, meta, "hash_id") {
		t.Fatalf("无 currentRow 时应判定为 hash 模式：%v", meta)
	}
	vuinCounter := firstOfKind(t, records, "vuin_counter")
	if numField(t, vuinCounter, "counter") != 987654321 || numField(t, vuinCounter, "max_vuin") != 987654321 {
		t.Fatalf("hash 模式 counter 应取 max_vuin：%v", vuinCounter)
	}
	if len(res.Warnings) == 0 {
		t.Fatal("缺 config.yml 应有提示")
	}
	assertSha256(t, records, data)
}

func TestExportLegacyIncludeMsgIDs(t *testing.T) {
	dir := t.TempDir()
	out := filepath.Join(dir, "out.jsonl")

	createDB(t, filepath.Join(dir, IdentityDBName), func(t *testing.T, tx *bbolt.Tx) {
		put(t, tx, IdentityBucketName, testOpenID, u64Bytes(10001))
	})
	createDB(t, filepath.Join(dir, MsgDBName), func(t *testing.T, tx *bbolt.Tx) {
		put(t, tx, MsgBucketName, "REALMSG1", u64Bytes(5))
		put(t, tx, MsgBucketName, MsgCounterKey, u64Bytes(5))
		put(t, tx, latestMsgBucket, "GROUP1:USER1", []byte("REALMSG1"))
	})

	res, err := ExportLegacy(ExportOptions{Dir: dir, Out: out, AppID: "102000001", IncludeMsgIDs: true})
	if err != nil {
		t.Fatalf("导出失败: %v", err)
	}
	records, data := readExport(t, out)

	msgID := firstOfKind(t, records, "msg_id")
	if strField(t, msgID, "real") != "REALMSG1" || numField(t, msgID, "virtual") != 5 {
		t.Fatalf("msg_id 行不符：%v", msgID)
	}
	latest := firstOfKind(t, records, "latest")
	if strField(t, latest, "group") != "GROUP1" || strField(t, latest, "user") != "USER1" || strField(t, latest, "msg_id") != "REALMSG1" {
		t.Fatalf("latest 行不符：%v", latest)
	}
	if res.Counts.MsgID != 1 || res.Counts.Latest != 1 {
		t.Fatalf("摘要计数不符：%+v", res.Counts)
	}
	meta := records[0]
	if counts := meta["counts"].(map[string]any); numField(t, counts, "msg_id") != 1 || numField(t, counts, "latest") != 1 {
		t.Fatalf("meta counts 未计入 msg_id/latest：%v", counts)
	}
	assertSha256(t, records, data)
}

func TestExportLegacyNoUserInfo(t *testing.T) {
	dir := t.TempDir()
	out := filepath.Join(dir, "out.jsonl")

	createDB(t, filepath.Join(dir, IdentityDBName), func(t *testing.T, tx *bbolt.Tx) {
		put(t, tx, IdentityBucketName, testOpenID, u64Bytes(10001))
		put(t, tx, UserInfoBucket, testOpenID+":10001", []byte(`{"user_id":"10001"}`))
	})

	records, _ := mustExport(t, ExportOptions{Dir: dir, Out: out, AppID: "102000001", NoUserInfo: true})
	if got := totalRecords(records, "user_info"); got != 0 {
		t.Fatalf("--no-user-info 时不应有 user_info 行，实际 %d 条", got)
	}
	if counts := records[0]["counts"].(map[string]any); numField(t, counts, "user_info") != 0 {
		t.Fatalf("meta counts 应记 0：%v", counts)
	}
}

func TestExportLegacyV2Source(t *testing.T) {
	dir := t.TempDir()
	out := filepath.Join(dir, "out.jsonl")

	createDB(t, filepath.Join(dir, v2IdentityDBName), func(t *testing.T, tx *bbolt.Tx) {
		put(t, tx, v2IdentityBucket, "openid:QQ:102000001:"+testOpenID, u64Bytes(10001))
		put(t, tx, v2IdentityBucket, "ruin:QQ:0:2870338968", u64Bytes(10002))
		put(t, tx, v2IdentityBucket, "raw:svc", u64Bytes(10003))
		put(t, tx, v2MetaBucket, v2IdentityCounter, u64Bytes(10005))
		put(t, tx, v2LastSeenBucket, "default:10001", []byte(testOpenID))
	})

	res, err := ExportLegacy(ExportOptions{Dir: dir, Out: out, AppID: "102000001"})
	if err != nil {
		t.Fatalf("导出失败: %v", err)
	}
	records, data := readExport(t, out)

	if strField(t, records[0], "source_schema") != schemaV2 {
		t.Fatalf("未识别 v2 源：%v", records[0])
	}
	identities := []map[string]any{}
	for _, rec := range records {
		if rec["kind"] == "identity" {
			identities = append(identities, rec)
		}
	}
	if len(identities) != 3 {
		t.Fatalf("v2 身份行数不符：%v", identities)
	}
	// bbolt 字节序：openid:... < raw:... < ruin:...
	if strField(t, identities[0], "key") != "openid:QQ:102000001:"+testOpenID || strField(t, identities[0], "itype") != "openid" || numField(t, identities[0], "vuin") != 10001 {
		t.Fatalf("v2 openid 键应原样保留：%v", identities[0])
	}
	if strField(t, identities[1], "key") != "raw:svc" || strField(t, identities[1], "itype") != "raw" {
		t.Fatalf("v2 raw 行不符：%v", identities[1])
	}
	if strField(t, identities[2], "key") != "ruin:QQ:0:2870338968" || strField(t, identities[2], "raw") != "rUIN-QQ-0-2870338968" || strField(t, identities[2], "itype") != "ruin" {
		t.Fatalf("v2 ruin 行不符：%v", identities[2])
	}
	vuinCounter := firstOfKind(t, records, "vuin_counter")
	if numField(t, vuinCounter, "counter") != 10005 || numField(t, vuinCounter, "max_vuin") != 10003 {
		t.Fatalf("v2 计数器不符：%v", vuinCounter)
	}
	lastSeen := firstOfKind(t, records, "last_seen")
	if numField(t, lastSeen, "vuin") != 10001 || strField(t, lastSeen, "raw") != testOpenID {
		t.Fatalf("last_seen 行不符：%v", lastSeen)
	}
	if res.Counts.LastSeen != 1 {
		t.Fatalf("摘要 last_seen 计数不符：%+v", res.Counts)
	}
	assertSha256(t, records, data)
}

func TestExportLegacyEmptyDB(t *testing.T) {
	dir := t.TempDir()
	out := filepath.Join(dir, "out.jsonl")

	createDB(t, filepath.Join(dir, IdentityDBName), func(t *testing.T, tx *bbolt.Tx) {
		if _, err := tx.CreateBucketIfNotExists([]byte(IdentityBucketName)); err != nil {
			t.Fatalf("创建空桶: %v", err)
		}
	})

	records, data := mustExport(t, ExportOptions{Dir: dir, Out: out, AppID: "102000001"})
	if len(records) != 4 {
		t.Fatalf("空库应只有 meta+两行计数器+end，实际 %d 行：%v", len(records), records)
	}
	vuinCounter := firstOfKind(t, records, "vuin_counter")
	if numField(t, vuinCounter, "counter") != 0 || numField(t, vuinCounter, "max_vuin") != 0 {
		t.Fatalf("空库 vuin_counter 应为 0：%v", vuinCounter)
	}
	if counts := records[0]["counts"].(map[string]any); numField(t, counts, "identity") != 0 {
		t.Fatalf("空库 counts 应为 0：%v", counts)
	}
	assertSha256(t, records, data)
}

func TestExportLegacyMissingDB(t *testing.T) {
	dir := t.TempDir()
	if _, err := ExportLegacy(ExportOptions{Dir: dir, Out: filepath.Join(dir, "out.jsonl"), AppID: "102000001"}); err == nil {
		t.Fatal("库文件不存在时应报错")
	}
}

func mustExport(t *testing.T, opts ExportOptions) ([]map[string]any, []byte) {
	t.Helper()
	if _, err := ExportLegacy(opts); err != nil {
		t.Fatalf("导出失败: %v", err)
	}
	return readExport(t, opts.Out)
}

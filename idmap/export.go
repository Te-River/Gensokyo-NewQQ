package idmap

import (
	"bufio"
	"crypto/sha256"
	"encoding/base32"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"hash"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	ididentity "github.com/hoshinonyaruko/gensokyo/internal/domain/identity"
	"go.etcd.io/bbolt"
	"gopkg.in/yaml.v3"
)

// 导出的 JSONL v1 格式，供 Rust 侧消费（docs/rust-m0.md）。
const (
	exportFormat  = "gensokyo-idmap"
	exportVersion = 1

	schemaLegacyV1 = "legacy-v1"
	schemaV2       = "v2"

	allocatorHash    = "hash"
	allocatorCounter = "counter"

	// v2 布局（go build -tags map_idmap，见 idmap/map_service.go）。导出器按文件/桶名直读，
	// 不引用会被 build tag 切换的同名包级常量。
	v2IdentityDBName  = "openid-map.db"
	v2MsgDBName       = "msgid-map.db"
	v2IdentityBucket  = "identity_to_vuin"
	v2MetaBucket      = "meta"
	v2IdentityCounter = "current_vuin"
	v2MsgCounter      = "current_msg"
	v2LastSeenBucket  = "last_seen"
	v2MsgBucket       = "msg_to_virtual"

	reverseKeyPrefix = "row-"
	tsKeyPrefix      = "ts:"
	stunSection      = "stun"
)

// ExportOptions 描述一次 legacy-export 的输入。
type ExportOptions struct {
	Dir           string // 数据库所在目录，空为当前目录
	Out           string // 输出 JSONL 路径
	AppID         string // 覆盖 config.yml 中的 app_id
	IncludeMsgIDs bool   // 额外导出 msg_id / latest 映射
	NoUserInfo    bool   // 跳过 UserInfo 桶
}

// ExportCounts 是各类导出记录的条数，meta 与 end 行都会带上。
type ExportCounts struct {
	Identity int `json:"identity"`
	Config   int `json:"config"`
	UserInfo int `json:"user_info"`
	LastSeen int `json:"last_seen"`
	MsgID    int `json:"msg_id"`
	Latest   int `json:"latest"`
}

// ExportResult 是导出结束后回给调用方的摘要。
type ExportResult struct {
	Path     string
	Counts   ExportCounts
	Sha256   string
	Warnings []string
}

type exportMeta struct {
	Kind           string       `json:"kind"`
	Format         string       `json:"format"`
	Version        int          `json:"version"`
	SourceSchema   string       `json:"source_schema"`
	ExportedAt     int64        `json:"exported_at"`
	AppID          string       `json:"app_id"`
	UinPrefix      string       `json:"uin_prefix"`
	IdmapIsolation bool         `json:"idmap_isolation"`
	Allocator      string       `json:"allocator"`
	HashID         bool         `json:"hash_id"`
	Counts         ExportCounts `json:"counts"`
}

type exportIdentity struct {
	Kind     string `json:"kind"`
	Key      string `json:"key"`
	Raw      string `json:"raw"`
	IType    string `json:"itype"`
	Platform string `json:"platform,omitempty"`
	AppID    string `json:"app_id,omitempty"`
	Vuin     int64  `json:"vuin"`
}

type exportCounter struct {
	Kind    string `json:"kind"`
	Counter int64  `json:"counter"`
	MaxVuin int64  `json:"max_vuin"`
}

type exportConfigLine struct {
	Kind        string `json:"kind"`
	Section     string `json:"section"`
	SectionKind string `json:"section_kind"`
	Key         string `json:"key"`
	Value       string `json:"value"`
}

type exportUserInfo struct {
	Kind   string          `json:"kind"`
	Raw    string          `json:"raw"`
	UserID string          `json:"user_id"`
	Data   json.RawMessage `json:"data"`
}

type exportLastSeen struct {
	Kind string `json:"kind"`
	Vuin int64  `json:"vuin"`
	Raw  string `json:"raw"`
}

type exportMsgID struct {
	Kind    string `json:"kind"`
	Virtual int64  `json:"virtual"`
	Real    string `json:"real"`
}

type exportLatest struct {
	Kind  string `json:"kind"`
	Group string `json:"group"`
	User  string `json:"user"`
	MsgID string `json:"msg_id"`
}

type exportEnd struct {
	Kind   string       `json:"kind"`
	Counts ExportCounts `json:"counts"`
	Sha256 string       `json:"sha256"`
}

// exportConfig 是导出器需要的 config.yml 片段。
type exportConfig struct {
	AppID          string
	UinPrefix      string
	IdmapIsolation bool
	HashID         bool
	Loaded         bool
}

// exportTally 汇总一次扫描的计数、跳过数与库内计数器。
type exportTally struct {
	counts           ExportCounts
	maxVuin          int64
	maxMsgVirtual    int64
	vuinCounter      int64
	hasVuinCounter   bool
	msgCounter       int64
	skippedAliases   int
	skippedUserInfos int
}

// exportSource 是两个已只读打开的数据库及其布局。
type exportSource struct {
	schema   string
	identity *bbolt.DB // ids/config/UserInfo 所在库
	msg      *bbolt.DB
}

// ExportLegacy 把旧版 idmap 数据库导出成 JSONL v1。
// 数据库以只读方式打开：运行中的 Gensokyo 会独占文件锁，导出前需停机或复制 db 文件。
func ExportLegacy(opts ExportOptions) (*ExportResult, error) {
	if strings.TrimSpace(opts.Out) == "" {
		return nil, fmt.Errorf("必须用 --out 指定导出文件")
	}
	dir := opts.Dir
	if dir == "" {
		dir = "."
	}

	cfg, err := loadExportConfig(filepath.Join(dir, "config.yml"))
	if err != nil {
		return nil, err
	}

	appID := strings.TrimSpace(opts.AppID)
	if appID == "" {
		appID = cfg.AppID
	}
	if appID == "" {
		return nil, fmt.Errorf("无法确定 app_id：config.yml 缺 app_id，请用 --app-id 指定")
	}

	src, err := openExportSource(dir)
	if err != nil {
		return nil, err
	}
	defer src.close()

	tally, err := src.collect(appID, cfg.UinPrefix, opts)
	if err != nil {
		return nil, err
	}

	allocator := allocatorCounter
	if cfg.Loaded {
		if cfg.HashID {
			allocator = allocatorHash
		}
	} else if !tally.hasVuinCounter {
		allocator = allocatorHash
	}

	vuinCounter := exportCounter{Kind: "vuin_counter", Counter: tally.vuinCounter, MaxVuin: tally.maxVuin}
	if !tally.hasVuinCounter {
		// hash_id 模式不写 currentRow，计数器只能取已分配的最大 vUIN。
		vuinCounter.Counter = tally.maxVuin
	}
	msgCounter := exportCounter{Kind: "msg_counter", Counter: tally.msgCounter, MaxVuin: tally.maxMsgVirtual}

	meta := exportMeta{
		Kind:           "meta",
		Format:         exportFormat,
		Version:        exportVersion,
		SourceSchema:   src.schema,
		ExportedAt:     time.Now().Unix(),
		AppID:          appID,
		UinPrefix:      cfg.UinPrefix,
		IdmapIsolation: cfg.IdmapIsolation,
		Allocator:      allocator,
		HashID:         allocator == allocatorHash,
		Counts:         tally.counts,
	}

	w, err := newExportWriter(opts.Out)
	if err != nil {
		return nil, err
	}

	written, err := writeExport(w, src, appID, cfg.UinPrefix, opts, meta, vuinCounter, msgCounter)
	if err != nil {
		w.abort()
		return nil, err
	}
	sum := w.sumHex()
	if err := w.writeEndLine(exportEnd{Kind: "end", Counts: written, Sha256: sum}); err != nil {
		w.abort()
		return nil, err
	}
	if err := w.close(); err != nil {
		return nil, err
	}

	return &ExportResult{
		Path:     opts.Out,
		Counts:   written,
		Sha256:   sum,
		Warnings: tally.warnings(cfg),
	}, nil
}

// writeExport 按 meta → identity → 计数器 → config → user_info → last_seen → msg_id/latest 的顺序落盘。
func writeExport(w *exportWriter, src *exportSource, appID, uinPrefix string, opts ExportOptions, meta exportMeta, vuinCounter, msgCounter exportCounter) (ExportCounts, error) {
	// 写盘阶段的计数独立统计，end 行以实际写入为准。
	var tally exportTally

	if err := w.writeLine(meta); err != nil {
		return tally.counts, err
	}
	if err := src.scanIdentities(appID, uinPrefix, &tally, func(line exportIdentity) error { return w.writeLine(line) }); err != nil {
		return tally.counts, err
	}
	if err := w.writeLine(vuinCounter); err != nil {
		return tally.counts, err
	}
	if err := w.writeLine(msgCounter); err != nil {
		return tally.counts, err
	}
	if err := src.scanConfigs(&tally, func(line exportConfigLine) error { return w.writeLine(line) }); err != nil {
		return tally.counts, err
	}
	if !opts.NoUserInfo {
		if err := src.scanUserInfos(uinPrefix, &tally, func(line exportUserInfo) error { return w.writeLine(line) }); err != nil {
			return tally.counts, err
		}
	}
	if err := src.scanLastSeen(&tally, func(line exportLastSeen) error { return w.writeLine(line) }); err != nil {
		return tally.counts, err
	}
	if opts.IncludeMsgIDs {
		if err := src.scanMsgIDs(&tally, func(line exportMsgID) error { return w.writeLine(line) }); err != nil {
			return tally.counts, err
		}
		if err := src.scanLatest(&tally, func(line exportLatest) error { return w.writeLine(line) }); err != nil {
			return tally.counts, err
		}
	}
	return tally.counts, nil
}

// collect 在第一遍扫描里统计条数、最大 vUIN 与计数器，供 meta 行使用。
func (s *exportSource) collect(appID, uinPrefix string, opts ExportOptions) (*exportTally, error) {
	tally := &exportTally{}
	if err := s.scanIdentities(appID, uinPrefix, tally, func(exportIdentity) error { return nil }); err != nil {
		return nil, err
	}
	if err := s.scanConfigs(tally, func(exportConfigLine) error { return nil }); err != nil {
		return nil, err
	}
	if !opts.NoUserInfo {
		if err := s.scanUserInfos(uinPrefix, tally, func(exportUserInfo) error { return nil }); err != nil {
			return nil, err
		}
	}
	if err := s.scanLastSeen(tally, func(exportLastSeen) error { return nil }); err != nil {
		return nil, err
	}
	if opts.IncludeMsgIDs {
		if err := s.scanMsgIDs(tally, func(exportMsgID) error { return nil }); err != nil {
			return nil, err
		}
		if err := s.scanLatest(tally, func(exportLatest) error { return nil }); err != nil {
			return nil, err
		}
	}

	var err error
	if s.schema == schemaV2 {
		tally.vuinCounter, tally.hasVuinCounter, err = s.readCounter(s.identity, v2MetaBucket, v2IdentityCounter)
		if err != nil {
			return nil, err
		}
		tally.msgCounter, _, err = s.readCounter(s.msg, v2MetaBucket, v2MsgCounter)
		return tally, err
	}
	tally.vuinCounter, tally.hasVuinCounter, err = s.readCounter(s.identity, IdentityBucketName, IdentityCounterKey)
	if err != nil {
		return nil, err
	}
	tally.msgCounter, _, err = s.readCounter(s.msg, MsgBucketName, MsgCounterKey)
	return tally, err
}

func (t *exportTally) warnings(cfg exportConfig) []string {
	var out []string
	if !cfg.Loaded {
		out = append(out, "未找到 config.yml：hash_id 按库内 currentRow 推断，隔离前缀按空处理")
	}
	if t.skippedAliases > 0 {
		out = append(out, fmt.Sprintf("跳过 %d 条纯数字别名键（隔离模式 UpdateVirtualValue 的残留，不是身份）", t.skippedAliases))
	}
	if t.skippedUserInfos > 0 {
		out = append(out, fmt.Sprintf("跳过 %d 条无法解析的 UserInfo 记录", t.skippedUserInfos))
	}
	return out
}

func (s *exportSource) scanIdentities(appID, uinPrefix string, tally *exportTally, fn func(exportIdentity) error) error {
	if s.schema == schemaV2 {
		return s.forEachBucket(s.identity, v2IdentityBucket, func(k, v []byte) error {
			if len(v) != 8 {
				return nil
			}
			line := identityFromV2Key(string(k), int64(binary.BigEndian.Uint64(v)))
			tally.counts.Identity++
			tally.maxVuin = max64(tally.maxVuin, line.Vuin)
			return fn(line)
		})
	}
	return s.forEachBucket(s.identity, IdentityBucketName, func(k, v []byte) error {
		if len(v) != 8 {
			return nil
		}
		key := string(k)
		if key == IdentityCounterKey || strings.HasPrefix(key, reverseKeyPrefix) {
			return nil
		}
		display := stripExportUinPrefix(key, uinPrefix)
		if display == "" || isDigitsOnly(display) {
			// 隔离模式 UpdateVirtualValue 写入的 <UIN>:<旧vUIN> 别名，值不是身份。
			tally.skippedAliases++
			return nil
		}
		line := identityFromLegacyKey(display, appID, int64(binary.BigEndian.Uint64(v)))
		tally.counts.Identity++
		tally.maxVuin = max64(tally.maxVuin, line.Vuin)
		return fn(line)
	})
}

func (s *exportSource) scanConfigs(tally *exportTally, fn func(exportConfigLine) error) error {
	return s.forEachBucket(s.identity, ConfigBucket, func(k, v []byte) error {
		section, name, ok := strings.Cut(string(k), ":")
		if !ok || section == stunSection {
			return nil
		}
		tally.counts.Config++
		return fn(exportConfigLine{
			Kind:        "config",
			Section:     section,
			SectionKind: exportSectionKind(section),
			Key:         name,
			Value:       string(v),
		})
	})
}

func (s *exportSource) scanUserInfos(uinPrefix string, tally *exportTally, fn func(exportUserInfo) error) error {
	return s.forEachBucket(s.identity, UserInfoBucket, func(k, v []byte) error {
		key := string(k)
		idx := strings.LastIndex(key, ":")
		if idx <= 0 || !json.Valid(v) {
			tally.skippedUserInfos++
			return nil
		}
		tally.counts.UserInfo++
		return fn(exportUserInfo{
			Kind:   "user_info",
			Raw:    stripExportUinPrefix(key[:idx], uinPrefix),
			UserID: key[idx+1:],
			Data:   json.RawMessage(append([]byte(nil), v...)),
		})
	})
}

// scanLastSeen 只在 v2 源里存在（旧版单值表没有上下文概念）。
func (s *exportSource) scanLastSeen(tally *exportTally, fn func(exportLastSeen) error) error {
	if s.schema != schemaV2 {
		return nil
	}
	return s.forEachBucket(s.identity, v2LastSeenBucket, func(k, v []byte) error {
		_, rawVuin, ok := strings.Cut(string(k), ":")
		if !ok {
			return nil
		}
		vuin, err := strconv.ParseInt(rawVuin, 10, 64)
		if err != nil {
			return nil
		}
		tally.counts.LastSeen++
		return fn(exportLastSeen{Kind: "last_seen", Vuin: vuin, Raw: string(v)})
	})
}

func (s *exportSource) scanMsgIDs(tally *exportTally, fn func(exportMsgID) error) error {
	if s.schema == schemaV2 {
		return s.forEachBucket(s.msg, v2MsgBucket, func(k, v []byte) error {
			if len(v) != 8 {
				return nil
			}
			line := exportMsgID{Kind: "msg_id", Virtual: int64(binary.BigEndian.Uint64(v)), Real: string(k)}
			tally.counts.MsgID++
			tally.maxMsgVirtual = max64(tally.maxMsgVirtual, line.Virtual)
			return fn(line)
		})
	}
	return s.forEachBucket(s.msg, MsgBucketName, func(k, v []byte) error {
		if len(v) != 8 {
			return nil
		}
		key := string(k)
		if key == MsgCounterKey || strings.HasPrefix(key, reverseKeyPrefix) || strings.HasPrefix(key, tsKeyPrefix) {
			return nil
		}
		line := exportMsgID{Kind: "msg_id", Virtual: int64(binary.BigEndian.Uint64(v)), Real: key}
		tally.counts.MsgID++
		tally.maxMsgVirtual = max64(tally.maxMsgVirtual, line.Virtual)
		return fn(line)
	})
}

func (s *exportSource) scanLatest(tally *exportTally, fn func(exportLatest) error) error {
	return s.forEachBucket(s.msg, latestMsgBucket, func(k, v []byte) error {
		group, user, ok := strings.Cut(string(k), ":")
		if !ok {
			return nil
		}
		tally.counts.Latest++
		return fn(exportLatest{Kind: "latest", Group: group, User: user, MsgID: string(v)})
	})
}

func (s *exportSource) forEachBucket(db *bbolt.DB, bucket string, fn func(k, v []byte) error) error {
	if db == nil {
		return nil
	}
	return db.View(func(tx *bbolt.Tx) error {
		b := tx.Bucket([]byte(bucket))
		if b == nil {
			return nil
		}
		return b.ForEach(fn)
	})
}

func (s *exportSource) readCounter(db *bbolt.DB, bucket, key string) (int64, bool, error) {
	if db == nil {
		return 0, false, nil
	}
	var value int64
	var found bool
	err := db.View(func(tx *bbolt.Tx) error {
		b := tx.Bucket([]byte(bucket))
		if b == nil {
			return nil
		}
		raw := b.Get([]byte(key))
		if len(raw) != 8 {
			return nil
		}
		value = int64(binary.BigEndian.Uint64(raw))
		found = true
		return nil
	})
	return value, found, err
}

func (s *exportSource) close() {
	if s.identity != nil {
		_ = s.identity.Close()
	}
	if s.msg != nil {
		_ = s.msg.Close()
	}
}

// openExportSource 优先打开运行中的双库，其次才是 v2 布局的单库。
func openExportSource(dir string) (*exportSource, error) {
	for _, cand := range []struct{ identity, msg string }{
		{IdentityDBName, MsgDBName},
		{v2IdentityDBName, v2MsgDBName},
	} {
		if _, err := os.Stat(filepath.Join(dir, cand.identity)); err != nil {
			continue
		}
		identity, err := openExportDB(filepath.Join(dir, cand.identity))
		if err != nil {
			return nil, err
		}
		msg, err := openExportDB(filepath.Join(dir, cand.msg))
		if err != nil {
			_ = identity.Close()
			return nil, err
		}
		src := &exportSource{schema: schemaLegacyV1, identity: identity, msg: msg}
		if hasBucket(identity, v2IdentityBucket) {
			src.schema = schemaV2
		}
		return src, nil
	}
	return nil, fmt.Errorf("在 %s 找不到 %s 或 %s", dir, IdentityDBName, v2IdentityDBName)
}

// openExportDB 只读打开数据库；文件不存在时返回 nil（msg 库可以缺失）。
func openExportDB(path string) (*bbolt.DB, error) {
	if _, err := os.Stat(path); err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, fmt.Errorf("读取 %s 失败: %w", path, err)
	}
	db, err := bbolt.Open(path, 0600, &bbolt.Options{ReadOnly: true, Timeout: 2 * time.Second})
	if err != nil {
		return nil, fmt.Errorf("打开 %s 失败（Gensokyo 运行中会独占数据库：请先停机，或用 --db-dir 指向复制出来的库）: %w", filepath.Base(path), err)
	}
	return db, nil
}

func hasBucket(db *bbolt.DB, bucket string) bool {
	found := false
	_ = db.View(func(tx *bbolt.Tx) error {
		found = tx.Bucket([]byte(bucket)) != nil
		return nil
	})
	return found
}

func loadExportConfig(path string) (exportConfig, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return exportConfig{}, nil
		}
		return exportConfig{}, fmt.Errorf("读取 %s 失败: %w", path, err)
	}
	var file struct {
		Settings struct {
			AppID          yaml.Node `yaml:"app_id"`
			Uin            int64     `yaml:"uin"`
			IdmapIsolation bool      `yaml:"idmap_isolation"`
			HashID         bool      `yaml:"hash_id"`
		} `yaml:"settings"`
	}
	if err := yaml.Unmarshal(data, &file); err != nil {
		return exportConfig{}, fmt.Errorf("解析 %s 失败: %w", path, err)
	}
	cfg := exportConfig{
		AppID:          strings.TrimSpace(file.Settings.AppID.Value),
		IdmapIsolation: file.Settings.IdmapIsolation,
		HashID:         file.Settings.HashID,
		Loaded:         true,
	}
	if cfg.IdmapIsolation {
		// 与 idmap.uinPrefix 一致：开关打开时前缀恒为 "<uin>:"。
		cfg.UinPrefix = strconv.FormatInt(file.Settings.Uin, 10) + ":"
	}
	return cfg, nil
}

// identityFromLegacyKey 把旧库的正表键（已剥隔离前缀）转成 v2 规范身份键。
func identityFromLegacyKey(display, appID string, vuin int64) exportIdentity {
	if ruin, ok := parseExportRUIN(display); ok {
		return exportIdentity{Kind: "identity", Key: ruin.key, Raw: ruin.raw, IType: "ruin", Platform: ruin.platform, Vuin: vuin}
	}
	if ididentity.IsOpenID(display) {
		return exportIdentity{
			Kind:     "identity",
			Key:      "openid:QQ:" + appID + ":" + display,
			Raw:      display,
			IType:    "openid",
			Platform: "QQ",
			AppID:    appID,
			Vuin:     vuin,
		}
	}
	return exportIdentity{Kind: "identity", Key: "raw:" + display, Raw: display, IType: "raw", Vuin: vuin}
}

// identityFromV2Key 原样保留 v2 规范键，只拆出展示用的字段。
func identityFromV2Key(key string, vuin int64) exportIdentity {
	if rest, ok := strings.CutPrefix(key, "openid:"); ok {
		if parts := strings.SplitN(rest, ":", 3); len(parts) == 3 {
			return exportIdentity{Kind: "identity", Key: key, Raw: parts[2], IType: "openid", Platform: parts[0], AppID: parts[1], Vuin: vuin}
		}
	}
	if rest, ok := strings.CutPrefix(key, "ruin:"); ok {
		if parts := strings.SplitN(rest, ":", 3); len(parts) == 3 {
			return exportIdentity{Kind: "identity", Key: key, Raw: "rUIN-" + parts[0] + "-" + parts[1] + "-" + parts[2], IType: "ruin", Platform: parts[0], Vuin: vuin}
		}
	}
	return exportIdentity{Kind: "identity", Key: key, Raw: strings.TrimPrefix(key, "raw:"), IType: "raw", Vuin: vuin}
}

type exportRUIN struct {
	key      string
	raw      string
	platform string
}

// parseExportRUIN 解析 rUIN-<平台>-<序号>-<ID>，口径与 idmap/map_service.go 的 parseRUIN 一致；
// 那个函数受 map_idmap build tag 约束，默认构建里不可见。
func parseExportRUIN(raw string) (exportRUIN, bool) {
	if !strings.HasPrefix(strings.ToLower(raw), "ruin-") {
		return exportRUIN{}, false
	}
	parts := strings.SplitN(raw, "-", 4)
	if len(parts) != 4 {
		return exportRUIN{}, false
	}
	platform, index, id := parts[1], parts[2], parts[3]
	if platform == "QQ" && !isDigitsOnly(id) {
		if decoded, err := base32.StdEncoding.WithPadding(base32.NoPadding).DecodeString(strings.ToUpper(id)); err == nil {
			id = string(decoded)
		}
	}
	return exportRUIN{key: "ruin:" + platform + ":" + index + ":" + id, raw: "rUIN-" + platform + "-" + index + "-" + id, platform: platform}, true
}

// exportSectionKind 只做分类，绝不做身份规范化（section 必须原样导出）。
func exportSectionKind(section string) string {
	if isDigitsOnly(section) {
		return "vuin"
	}
	if ididentity.IsOpenID(section) {
		return "openid"
	}
	return "other"
}

func stripExportUinPrefix(value, prefix string) string {
	if prefix == "" || !strings.HasPrefix(value, prefix) {
		return value
	}
	return strings.TrimPrefix(value, prefix)
}

func isDigitsOnly(s string) bool {
	if s == "" {
		return false
	}
	for _, r := range s {
		if r < '0' || r > '9' {
			return false
		}
	}
	return true
}

func max64(a, b int64) int64 {
	if a > b {
		return a
	}
	return b
}

// exportWriter 逐行写紧凑 JSON，并累计 end 行之前所有字节的 sha256（含行尾 \n）。
type exportWriter struct {
	file *os.File
	w    *bufio.Writer
	sum  hash.Hash
}

func newExportWriter(path string) (*exportWriter, error) {
	file, err := os.OpenFile(path, os.O_CREATE|os.O_WRONLY|os.O_TRUNC, 0600)
	if err != nil {
		return nil, fmt.Errorf("创建 %s 失败: %w", path, err)
	}
	// 覆盖已有文件时 OpenFile 不会改权限；文件含 OpenID，强制 0600。
	if err := file.Chmod(0600); err != nil {
		_ = file.Close()
		return nil, fmt.Errorf("设置 %s 权限失败: %w", path, err)
	}
	return &exportWriter{file: file, w: bufio.NewWriter(file), sum: sha256.New()}, nil
}

func (ew *exportWriter) writeLine(v any) error {
	data, err := json.Marshal(v)
	if err != nil {
		return fmt.Errorf("序列化导出记录失败: %w", err)
	}
	data = append(data, '\n')
	ew.sum.Write(data)
	return ew.writeRaw(data)
}

// writeEndLine 写 end 行：它自身不参与 sha256。
func (ew *exportWriter) writeEndLine(v any) error {
	data, err := json.Marshal(v)
	if err != nil {
		return fmt.Errorf("序列化导出记录失败: %w", err)
	}
	return ew.writeRaw(append(data, '\n'))
}

func (ew *exportWriter) writeRaw(data []byte) error {
	if _, err := ew.w.Write(data); err != nil {
		return fmt.Errorf("写导出文件失败: %w", err)
	}
	return nil
}

func (ew *exportWriter) sumHex() string {
	return hex.EncodeToString(ew.sum.Sum(nil))
}

func (ew *exportWriter) close() error {
	if err := ew.w.Flush(); err != nil {
		_ = ew.file.Close()
		return fmt.Errorf("落盘失败: %w", err)
	}
	return ew.file.Close()
}

func (ew *exportWriter) abort() {
	_ = ew.file.Close()
	// 半截文件不能被误当成合法导出：失败时删掉。
	_ = os.Remove(ew.file.Name())
}

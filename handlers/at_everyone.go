package handlers

import (
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"regexp"
	"strings"
	"sync"
	"time"

	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/mylog"
)

// ---------- @全员（qqbot-at-everyone） ----------
// 官方文本交互页（10/26/2026）：群聊 @全部成员用自闭合标签 <qqbot-at-everyone />，
// 文本与 markdown 两种消息都支持；限制为「每个群每天（自然日）仅可发送 1 次」，
// 且机器人需为群管理员（管理员判定交给上游，此处不做本地判定）。

const (
	// atEveryoneTag 是官方唯一自闭合写法（带空格），产出统一用这一条。
	atEveryoneTag = "<qqbot-at-everyone />"
	// atEveryoneSection 是计数落 idmap config 桶的 section 名。
	atEveryoneSection = "at_everyone"
)

var (
	// atEveryoneCQPattern 匹配 CQ 主入口 [CQ:at,qq=all] 与宽松别名 qq=everyone（大小写不敏感），
	// 兼容误带的附加参数（如 [CQ:at,qq=all,name=all]）。
	atEveryoneCQPattern = regexp.MustCompile(`(?i)\[CQ:at,qq=(?:all|everyone)(?:,[^\]]*)?\]`)
	// atEveryoneTagPattern 匹配官方标签，兼容手写无空格的 <qqbot-at-everyone/>。
	atEveryoneTagPattern = regexp.MustCompile(`<qqbot-at-everyone\s*/>`)
)

// atEveryoneZone 固定东八区：官方「自然日」按北京时间切分，
// 用主机本地时区会让 UTC/其他时区主机在跨日边界判错（两侧统一 +08:00）。
var atEveryoneZone = time.FixedZone("CST", 8*3600)

// atEveryoneNow 取当前时间并按固定时区换算，单测可替换以覆盖跨日边界。
var atEveryoneNow = func() time.Time { return time.Now().In(atEveryoneZone) }

// NormalizeAtEveryone 把 [CQ:at,qq=all|everyone] 归一为官方自闭合标签；
// 字面标签保持原样（照官方文档抄写的场景）。挂在 CQ 管道的 card 阶段之前，
// 保证文本与 markdown 两条出站路径同源。
// 判定与归一必须走同一正则（大小写不敏感）：早期快速路径用区分大小写的
// strings.Contains 预筛，[cq:at,qq=all] 会被检测到（计入额度）却不被归一（S2）。
func NormalizeAtEveryone(text string) string {
	if !atEveryoneCQPattern.MatchString(text) {
		return text
	}
	return atEveryoneCQPattern.ReplaceAllString(text, atEveryoneTag)
}

// atEveryonePresent 判断本次出站是否带 @全员：文本里有标签，
// 或 markdown 载荷（base64 或原始 JSON）里含标签。
func atEveryonePresent(messageText string, foundItems map[string][]string) bool {
	// S3：markdown 载荷本身会被拒（模板缺 content）时，本请求不会发出任何标签，
	// 提前返回 false，避免「先占额度、再因载荷非法失败」的白扣。
	if markdownItemsError(foundItems) != nil {
		return false
	}
	if atEveryoneTagPattern.MatchString(messageText) {
		return true
	}
	for _, item := range foundItems["markdown"] {
		if markdownContainsAtEveryone(item) {
			return true
		}
	}
	return false
}

// markdownContainsAtEveryone 检查单份 markdown 载荷（base64 优先，失败按原文匹配）。
// 归一发生在载荷入库时，这里再兜一层 CQ 码原文匹配：未归一的载荷一旦放行，
// 额度会被白扣而标签不会渲染，宁可提前拦下来。
func markdownContainsAtEveryone(raw string) bool {
	if raw == "" {
		return false
	}
	if atEveryoneTagPattern.MatchString(raw) || atEveryoneCQPattern.MatchString(raw) {
		return true
	}
	decoded, err := base64.StdEncoding.DecodeString(raw)
	if err != nil {
		return false
	}
	text := string(decoded)
	return atEveryoneTagPattern.MatchString(text) || atEveryoneCQPattern.MatchString(text)
}

// normalizeAtEveryoneInMarkdown 对单份 markdown 载荷做 @全员 归一。
// 载荷在 CQ 管道里先被 base64 编码，文本侧的 NormalizeAtEveryone 扫不到其内部 content，
// 若不在解码后归一，markdown 里的 [CQ:at,qq=all] 会既不计额也不渲染。
// 输入兼容 base64 与原始 JSON 两种形态；未命中 CQ 码时原样返回。
func normalizeAtEveryoneInMarkdown(raw string) string {
	if raw == "" {
		return raw
	}
	// 预筛同样走大小写不敏感的正则：检测口径与归一口径必须一致（S2）
	if decoded, err := base64.StdEncoding.DecodeString(raw); err == nil {
		text := string(decoded)
		if !atEveryoneCQPattern.MatchString(text) {
			return raw
		}
		normalized := NormalizeAtEveryone(text)
		if normalized == text {
			return raw
		}
		return base64.StdEncoding.EncodeToString([]byte(normalized))
	}
	if !atEveryoneCQPattern.MatchString(raw) {
		return raw
	}
	return NormalizeAtEveryone(raw)
}

// normalizeAtEveryoneInFoundItems 归一 foundItems 中所有 markdown 载荷内的 @全员 CQ 码。
// 收敛点：legacy 的 CQ 管道与 cqparse 新链都可能在载荷进库时已经 base64，
// 统一在解析出口调用一次，避免任一路径漏网。
func normalizeAtEveryoneInFoundItems(foundItems map[string][]string) {
	for i, raw := range foundItems["markdown"] {
		foundItems["markdown"][i] = normalizeAtEveryoneInMarkdown(raw)
	}
}

// atEveryoneRecord 是计数桶里存的值：自然日 + 次数（恒为 1）。
type atEveryoneRecord struct {
	Date  string `json:"date"`
	Count int    `json:"count"`
}

// atEveryoneCounter 抽象计数的存储，默认落 idmap config 桶；
// 抽成接口是为了单测可替换为内存实现，避免测试污染 idmap.db。
type atEveryoneCounter interface {
	Read(groupOpenID string) (string, error)
	Write(groupOpenID, value string) error
}

// idmapCounter 用现有 idmap.WriteConfig/ReadConfig 落盘（section=at_everyone, key=group_openid）。
type idmapCounter struct{}

func (idmapCounter) Read(groupOpenID string) (string, error) {
	return idmap.ReadConfig(atEveryoneSection, groupOpenID)
}

func (idmapCounter) Write(groupOpenID, value string) error {
	return idmap.WriteConfig(atEveryoneSection, groupOpenID, value)
}

var (
	atEveryoneStore atEveryoneCounter = idmapCounter{}
	// atEveryoneMutexes 进程内按群串行，避免并发请求同时通过同日判定。
	atEveryoneMutexes sync.Map // groupOpenID -> *sync.Mutex
	// atEveryoneLimitEnabled 读取开关，单测可覆盖（默认 true）。
	atEveryoneLimitEnabled = func() bool { return config.GetEnableAtEveryoneLimit() }
)

func atEveryoneMutex(groupOpenID string) *sync.Mutex {
	m, _ := atEveryoneMutexes.LoadOrStore(groupOpenID, &sync.Mutex{})
	return m.(*sync.Mutex)
}

// GuardAtEveryone 在群聊发送前占用「每群每自然日 1 次」的 @全员 额度。
// 语义：先占位写 count=1 → 发送；发送失败时调用返回的 rollback 归还旧值。
// 当日已占用时返回中文原因，调用方回 retcode=1400；开关关闭时不计数也不拦截。
// dryRun=true 用于 shadow 的 new 对比执行：不写计数、不拦截，直接返回空操作
// （shadow 下只做预演，额度由 legacy 真发送路径占用）。
func GuardAtEveryone(groupOpenID string, dryRun bool) (rollback func(), err error) {
	noop := func() {}
	if groupOpenID == "" {
		return noop, errors.New("缺少 group_openid，无法校验 @全员 次数")
	}
	if dryRun {
		// shadow 预演：不写库，也不比对新链结果（新链结果只用于 diff 日志）
		return noop, nil
	}
	if !atEveryoneLimitEnabled() {
		return noop, nil
	}

	mu := atEveryoneMutex(groupOpenID)
	mu.Lock()
	defer mu.Unlock()

	// 自然日固定按 +08:00 切分，不受主机本地时区影响
	today := atEveryoneNow().Format("2006-01-02")
	prev, readErr := atEveryoneStore.Read(groupOpenID)
	if readErr != nil {
		// 读失败按「未占用」处理：宁可放行也不要因存储抖动阻断业务
		prev = ""
	}
	var rec atEveryoneRecord
	if prev != "" {
		_ = json.Unmarshal([]byte(prev), &rec)
	}
	if rec.Date == today && rec.Count > 0 {
		return noop, errors.New("@全部成员 每群每自然日仅可发送 1 次，本群今日已使用")
	}

	encoded, marshalErr := json.Marshal(atEveryoneRecord{Date: today, Count: 1})
	if marshalErr != nil {
		return noop, fmt.Errorf("序列化 @全员 计数失败: %w", marshalErr)
	}
	if writeErr := atEveryoneStore.Write(groupOpenID, string(encoded)); writeErr != nil {
		return noop, fmt.Errorf("写入 @全员 计数失败: %w", writeErr)
	}

	var once sync.Once
	return func() {
		once.Do(func() {
			mu.Lock()
			defer mu.Unlock()
			if restoreErr := atEveryoneStore.Write(groupOpenID, prev); restoreErr != nil {
				mylog.Printf("[at_everyone] 回滚计数失败 group=%s: %v", groupOpenID, restoreErr)
			}
		})
	}, nil
}

// migrateAtEveryoneToMarkdown 在 markdown 优先时把文本里的 @全员 标签迁移进
// markdown.content，否则 content 被清空后标签会丢失（官方 markdown 同样支持该标签）。
func migrateAtEveryoneToMarkdown(content, messageText string) string {
	if content == "" || !atEveryoneTagPattern.MatchString(messageText) {
		// 防御性保留：模板 Markdown（Params 模式）没有 content 的载荷已在 parseMDData 被拒，
		// 且这类请求由 atEveryonePresent 提前判定为不计额（S3），不会出现「占额后丢标签」。
		return content
	}
	if atEveryoneTagPattern.MatchString(content) {
		return content
	}
	return strings.TrimSpace(content + " " + atEveryoneTag)
}

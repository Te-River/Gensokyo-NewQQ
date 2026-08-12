package echo

import (
	"fmt"
	"math/rand"
	"strconv"
	"sync"
	"time"

	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/tencent-connect/botgo/dto"
)

func init() {
	// 在 init 函数中运行清理逻辑
	startCleanupRoutine()
}

func startCleanupRoutine() {
	cleanupTicker = time.NewTicker(30 * time.Minute)
	go func() {
		for {
			<-cleanupTicker.C
			cleanupGlobalMaps()
		}
	}()
}

func cleanupGlobalMaps() {
	cleanupSyncMap(&globalSyncMapMsgid)
	cleanupSyncMap(&globalReverseMapMsgid)
	cleanupMessageGroupStack(globalMessageGroupStack)
	cleanupEchoMapping(globalEchoMapping)
	cleanupInt64ToIntMapping(globalInt64ToIntMapping)
	cleanupStringToIntMappingSeq(globalStringToIntMappingSeq)
}

func cleanupSyncMap(m *sync.Map) {
	m.Range(func(key, value interface{}) bool {
		m.Delete(key)
		return true
	})
}

func cleanupMessageGroupStack(stack *globalMessageGroup) {
	stack.stack = make([]MessageGroupPair, 0)
}

func cleanupEchoMapping(mapping *EchoMapping) {
	mapping.msgTypeMapping.Range(func(key, value interface{}) bool {
		mapping.msgTypeMapping.Delete(key)
		return true
	})
	mapping.msgIDMapping.Range(func(key, value interface{}) bool {
		mapping.msgIDMapping.Delete(key)
		return true
	})
	mapping.eventIDMapping.Range(func(key, value interface{}) bool {
		mapping.eventIDMapping.Delete(key)
		return true
	})
}

func cleanupInt64ToIntMapping(mapping *Int64ToIntMapping) {
	mapping.mapping.Range(func(key, value interface{}) bool {
		mapping.mapping.Delete(key)
		return true
	})
}

func cleanupStringToIntMappingSeq(mapping *StringToIntMappingSeq) {
	mapping.mapping.Range(func(key, value interface{}) bool {
		mapping.mapping.Delete(key)
		return true
	})
}

type EchoMapping struct {
	msgTypeMapping sync.Map
	msgIDMapping   sync.Map
	eventIDMapping sync.Map
}

// Int64ToIntMapping 用于存储 int64 到 int 的映射(递归计数器)
type Int64ToIntMapping struct {
	mapping sync.Map
}

// StringToIntMappingSeq 用于存储 string 到 int 的映射(seq对应)
type StringToIntMappingSeq struct {
	mapping sync.Map
	mu      sync.Mutex // 保护 seq 原子递增，避免多段回复读-改-写竞态触发 QQ 40054005 去重
}

// MessageGroupPair 用于存储 group 和 groupMessage
type MessageGroupPair struct {
	Group         string
	GroupMessage  *dto.MessageToCreate
	EnqueueTime   time.Time // 入队时间，用于诊断补发延迟
	CorrelationID string    // 关联标识，用于跨入队/补发边界的日志关联
}

// 定义全局栈的结构体
type globalMessageGroup struct {
	mu    sync.Mutex
	stack []MessageGroupPair
}

// 使用 sync.Map 作为内存存储
var (
	globalSyncMapMsgid    sync.Map
	globalReverseMapMsgid sync.Map // 用于存储反向键值对
	cleanupTicker         *time.Ticker
	onceMsgid             sync.Once
)

// 初始化一个全局栈实例
var globalMessageGroupStack = &globalMessageGroup{
	stack: make([]MessageGroupPair, 0),
}

var globalEchoMapping = &EchoMapping{
	msgTypeMapping: sync.Map{},
	msgIDMapping:   sync.Map{},
	eventIDMapping: sync.Map{},
}

var globalInt64ToIntMapping = &Int64ToIntMapping{
	mapping: sync.Map{},
}

var globalStringToIntMappingSeq = &StringToIntMappingSeq{
	mapping: sync.Map{},
}

func (e *EchoMapping) GenerateKey(appid string, s int64) string {
	return appid + "_" + strconv.FormatInt(s, 10)
}

func (e *EchoMapping) GenerateKeyv2(appid string, groupid int64, userid int64) string {
	return appid + "_" + strconv.FormatInt(groupid, 10) + "_" + strconv.FormatInt(userid, 10)
}

func (e *EchoMapping) GenerateKeyEventID(appid string, groupid int64) string {
	return appid + "_" + strconv.FormatInt(groupid, 10)
}

func (e *EchoMapping) GenerateKeyEventIDV2(appid string, groupid string) string {
	return appid + "_" + groupid
}

func (e *EchoMapping) GenerateKeyv3(appid string, s string) string {
	return appid + "_" + s
}

// 添加 echo 对应的类型
func AddMsgType(appid string, s int64, msgType string) {
	key := globalEchoMapping.GenerateKey(appid, s)
	globalEchoMapping.msgTypeMapping.Store(key, msgType)
}

// 添加echo对应的messageid
func AddMsgIDv3(appid string, s string, msgID string) {
	key := globalEchoMapping.GenerateKeyv3(appid, s)
	globalEchoMapping.msgIDMapping.Store(key, msgID)
}

// GetMsgIDv3 返回给定appid和s的msgID
func GetMsgIDv3(appid string, s string) string {
	key := globalEchoMapping.GenerateKeyv3(appid, s)
	value, ok := globalEchoMapping.msgIDMapping.Load(key)
	if !ok {
		return "" // 或者根据需要返回默认值或者错误处理
	}
	return value.(string)
}

// 添加group和userid对应的messageid
func AddMsgIDv2(appid string, groupid int64, userid int64, msgID string) {
	key := globalEchoMapping.GenerateKeyv2(appid, groupid, userid)
	globalEchoMapping.msgIDMapping.Store(key, msgID)
}

// 添加group对应的eventid
func AddEvnetID(appid string, groupid int64, eventID string) {
	key := globalEchoMapping.GenerateKeyEventID(appid, groupid)
	globalEchoMapping.eventIDMapping.Store(key, eventID)
}

// 添加group对应的eventid
func AddEvnetIDv2(appid string, groupid string, eventID string) {
	key := globalEchoMapping.GenerateKeyEventIDV2(appid, groupid)
	globalEchoMapping.eventIDMapping.Store(key, eventID)
}

// 添加echo对应的messageid
func AddMsgID(appid string, s int64, msgID string) {
	key := globalEchoMapping.GenerateKey(appid, s)
	globalEchoMapping.msgIDMapping.Store(key, msgID)
}

// 根据给定的key获取消息类型
func GetMsgTypeByKey(key string) string {
	value, _ := globalEchoMapping.msgTypeMapping.Load(key)
	if value == nil {
		return "" // 根据需要返回默认值或者进行错误处理
	}
	return value.(string)
}

// 根据给定的key获取消息ID
func GetMsgIDByKey(key string) string {
	value, _ := globalEchoMapping.msgIDMapping.Load(key)
	if value == nil {
		return "" // 根据需要返回默认值或者进行错误处理
	}
	return value.(string)
}

// 根据给定的key获取EventID
func GetEventIDByKey(key string) string {
	value, _ := globalEchoMapping.eventIDMapping.Load(key)
	if value == nil {
		return "" // 根据需要返回默认值或者进行错误处理
	}
	return value.(string)
}

// AddMapping 添加一个新的映射
func AddMapping(key int64, value int) {
	globalInt64ToIntMapping.mapping.Store(key, value)
}

// GetMapping 根据给定的 int64 键获取映射值
func GetMapping(key int64) int {
	value, _ := globalInt64ToIntMapping.mapping.Load(key)
	if value == nil {
		return 0 // 根据需要返回默认值或者进行错误处理
	}
	return value.(int)
}

// AddMappingSeq 添加一个新的映射。
// Deprecated: 新代码应使用 NextMappingSeq 或 CurrentAndIncrementMappingSeq，
// 以避免 GetMappingSeq 和 AddMappingSeq 组合产生读-改-写竞态。
func AddMappingSeq(key string, value int) {
	globalStringToIntMappingSeq.mu.Lock()
	defer globalStringToIntMappingSeq.mu.Unlock()
	globalStringToIntMappingSeq.mapping.Store(key, value)
}

// GetMappingSeq 根据给定的 string 键获取映射值
func GetMappingSeq(key string) int {
	value, ok := globalStringToIntMappingSeq.mapping.Load(key)
	if !ok {
		if config.GetRamDomSeq() {
			rng := rand.New(rand.NewSource(time.Now().UnixNano()))
			return rng.Intn(10000) + 1 // 生成 1 到 10000 的随机数
		}
		return 0 // 或者根据需要返回默认值或者进行错误处理
	}
	return value.(int)
}

// nextMappingSeqLocked 读取当前值并原子推进到下一个值。
// 调用方必须持有 globalStringToIntMappingSeq.mu。
func nextMappingSeqLocked(key string) (current, next int) {
	value, ok := globalStringToIntMappingSeq.mapping.Load(key)
	if ok {
		current = value.(int)
	} else if config.GetRamDomSeq() {
		rng := rand.New(rand.NewSource(time.Now().UnixNano()))
		current = rng.Intn(10000) + 1
	}
	next = current + 1
	globalStringToIntMappingSeq.mapping.Store(key, next)
	return current, next
}

// NextMappingSeq 原子递增 seq 并返回递增后的值。
func NextMappingSeq(key string) int {
	globalStringToIntMappingSeq.mu.Lock()
	defer globalStringToIntMappingSeq.mu.Unlock()
	_, next := nextMappingSeqLocked(key)
	return next
}

// CurrentAndIncrementMappingSeq 原子读取当前 seq，并推进内部值。
// 该接口用于兼容历史上“使用当前值、再写入下一个值”的调用路径。
func CurrentAndIncrementMappingSeq(key string) int {
	globalStringToIntMappingSeq.mu.Lock()
	defer globalStringToIntMappingSeq.mu.Unlock()
	current, _ := nextMappingSeqLocked(key)
	return current
}

// IncrementMappingSeq 保留旧 API，行为等同于 NextMappingSeq。
func IncrementMappingSeq(key string) int {
	return NextMappingSeq(key)
}

// ssmCounter 用于生成 SSM 补发队列的唯一关联标识
var ssmCounter uint64
var ssmMu sync.Mutex

// NextSSMCorrelationID 生成 SSM 补发队列的唯一关联标识
func NextSSMCorrelationID(groupID string) string {
	ssmMu.Lock()
	ssmCounter++
	c := ssmCounter
	ssmMu.Unlock()
	return fmt.Sprintf("ssm-%s-%d", groupID[:8], c)
}

// PushGlobalStack 向全局栈中添加一个新的 MessageGroupPair
func PushGlobalStack(pair MessageGroupPair) {
	pair.EnqueueTime = time.Now()
	if pair.CorrelationID == "" {
		pair.CorrelationID = NextSSMCorrelationID(pair.Group)
	}
	globalMessageGroupStack.mu.Lock()
	defer globalMessageGroupStack.mu.Unlock()
	globalMessageGroupStack.stack = append(globalMessageGroupStack.stack, pair)
}

// PopGlobalStackMulti 从全局栈中取出指定数量的 MessageGroupPair，但不删除它们
func PopGlobalStackMulti(count int) []MessageGroupPair {
	globalMessageGroupStack.mu.Lock()
	defer globalMessageGroupStack.mu.Unlock()

	if count == 0 || len(globalMessageGroupStack.stack) == 0 {
		return nil
	}

	if count > len(globalMessageGroupStack.stack) {
		count = len(globalMessageGroupStack.stack)
	}

	return globalMessageGroupStack.stack[:count]
}

// RemoveFromGlobalStack 从全局栈中删除指定下标的元素
func RemoveFromGlobalStack(index int) {
	globalMessageGroupStack.mu.Lock()
	defer globalMessageGroupStack.mu.Unlock()

	if index < 0 || index >= len(globalMessageGroupStack.stack) {
		return // 下标越界检查
	}

	globalMessageGroupStack.stack = append(globalMessageGroupStack.stack[:index], globalMessageGroupStack.stack[index+1:]...)
}

// StoreCacheInMemory 根据 ID 将映射存储在内存中的 sync.Map 中
func StoreCacheInMemory(id string) (int64, error) {
	var newRow int64

	// 检查是否已存在映射
	if value, ok := globalSyncMapMsgid.Load(id); ok {
		newRow = value.(int64)
		return newRow, nil
	}

	// 生成新的行号
	var err error
	maxDigits := 18 // int64 的位数上限-1
	for digits := 9; digits <= maxDigits; digits++ {
		newRow, err = idmap.GenerateRowID(id, digits)
		if err != nil {
			return 0, err
		}

		// 检查新生成的行号是否重复
		if _, exists := globalSyncMapMsgid.LoadOrStore(id, newRow); !exists {
			// 存储反向键值对
			globalReverseMapMsgid.Store(newRow, id)
			// 找到了一个唯一的行号，可以跳出循环
			break
		}

		// 如果到达了最大尝试次数还没有找到唯一的行号，则返回错误
		if digits == maxDigits {
			return 0, fmt.Errorf("unable to find a unique row ID after %d attempts", maxDigits-8)
		}
	}

	return newRow, nil
}

// GetIDFromRowID 根据行号获取原始 ID
func GetCacheIDFromMemoryByRowID(rowID string) (string, bool) {
	introwID, _ := strconv.ParseInt(rowID, 10, 64)
	if value, ok := globalReverseMapMsgid.Load(introwID); ok {
		return value.(string), true
	}
	return "", false
}

// StartCleanupRoutine 启动定时清理函数，每5分钟清空 globalSyncMapMsgid 和 globalReverseMapMsgid
func StartCleanupRoutine() {
	onceMsgid.Do(func() {
		cleanupTicker = time.NewTicker(5 * time.Minute)

		// 启动一个协程执行清理操作
		go func() {
			for range cleanupTicker.C {
				fmt.Println("Starting cleanup...")

				// 清空 sync.Map
				globalSyncMapMsgid.Range(func(key, value interface{}) bool {
					globalSyncMapMsgid.Delete(key)
					return true
				})

				// 清空反向映射 sync.Map
				globalReverseMapMsgid.Range(func(key, value interface{}) bool {
					globalReverseMapMsgid.Delete(key)
					return true
				})

				fmt.Println("Cleanup completed.")
			}
		}()
	})
}

// StopCleanupRoutine 停止定时清理函数
func StopCleanupRoutine() {
	if cleanupTicker != nil {
		cleanupTicker.Stop()
	}
}

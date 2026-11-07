package mylog

import (
	"fmt"
	"os"
	"path/filepath"
	"runtime/debug"
	"sync"
	"time"
)

// panicLogDir 是崩溃现场落盘目录，相对进程工作目录。
const panicLogDir = "logs"

// panicLogMutex 串行化多 goroutine 同时崩溃时的写入，避免记录交错。
var panicLogMutex sync.Mutex

// InstallCrashHandler 提升运行时默认崩溃输出：未捕获 panic 时打印所有 goroutine 的堆栈。
// 进程仍然按原语义崩溃退出，只是让现场可查。
func InstallCrashHandler() {
	debug.SetTraceback("all")
}

// RecoverPanic 供 defer 使用，捕获当前 goroutine 的 panic 并落盘。
// 用法：defer mylog.RecoverPanic("ws.dispatch")
func RecoverPanic(scope string) {
	if r := recover(); r != nil {
		RecordPanic(scope, r)
	}
}

// RecordPanic 记录一次 panic 值与其完整堆栈，同时写入 mylog 与 logs/panic.log。
// 本函数绝不 panic：即使落盘失败也只降级为 stderr 输出。
func RecordPanic(scope string, value interface{}) {
	stack := debug.Stack()
	// mylog 自身写日志时不能再把兜底逻辑炸一次
	func() {
		defer func() { _ = recover() }()
		Errorf("panic recovered [scope=%s]: %v\n%s", scope, value, stack)
	}()

	entry := fmt.Sprintf("\n=== panic @ %s scope=%s ===\npanic: %v\n%s",
		time.Now().Format("2006-01-02 15:04:05.000"), scope, value, stack)
	if err := appendPanicLog(entry); err != nil {
		// 落盘失败不能再引发崩溃，降级到 stderr
		_, _ = fmt.Fprintf(os.Stderr, "panic log write failed: %v%s", err, entry)
	}
}

// appendPanicLog 追加一条崩溃记录，自动创建 logs 目录。
func appendPanicLog(entry string) error {
	panicLogMutex.Lock()
	defer panicLogMutex.Unlock()

	if err := os.MkdirAll(panicLogDir, 0755); err != nil {
		return err
	}
	file, err := os.OpenFile(filepath.Join(panicLogDir, "panic.log"), os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0644)
	if err != nil {
		return err
	}
	// O_APPEND + 单次写保证整条记录连续，互斥锁保证不与其他崩溃记录交错
	if _, err := file.WriteString(entry); err != nil {
		_ = file.Close()
		return err
	}
	return file.Close()
}

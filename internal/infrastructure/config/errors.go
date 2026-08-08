// Package config 提供配置加载 → 迁移 → 校验 → 不可变快照的完整管线。
//
// 与旧 `github.com/hoshinonyaruko/gensokyo/config` 全局 singleton 双轨并存：
// 本包只提供基础设施，业务层迁移到 Snapshot 由后续阶段（P11 DI）完成。
//
// 管线：
//
//	Load / Parse → Migrate(yaml.Node) → Validate → buildRuntime → Snapshot
package config

import "fmt"

// Kind 配置错误的类别。
type Kind uint8

const (
	KindParse     Kind = iota + 1 // YAML 解析失败
	KindMigration                 // 版本迁移失败
	KindValidation                // Schema / 语义校验失败
	KindIO                        // 文件读写失败
)

// Error 带具体字段路径的配置错误。
type Error struct {
	Kind Kind
	Path string // 例如 "config.qq.app_id"
	Msg  string
	Err  error
}

func (e *Error) Error() string {
	if e.Path != "" {
		return fmt.Sprintf("%s: %s", e.Path, e.Msg)
	}
	return e.Msg
}

func (e *Error) Unwrap() error { return e.Err }

func newParseError(err error) *Error {
	return &Error{Kind: KindParse, Msg: "parse config: " + err.Error(), Err: err}
}

func newMigrationError(err error) *Error {
	return &Error{Kind: KindMigration, Msg: "migrate config: " + err.Error(), Err: err}
}

func newValidationError(path, msg string) *Error {
	return &Error{Kind: KindValidation, Path: path, Msg: msg}
}

func newIOError(err error) *Error {
	return &Error{Kind: KindIO, Msg: "config io: " + err.Error(), Err: err}
}

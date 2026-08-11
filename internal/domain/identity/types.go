// Package identity 提供身份标识的类型系统。
//
// 明确区分 OpenID / 虚拟用户ID / 虚拟群ID / UIN / AppID，避免全部使用 string
// 并靠开发者脑补语义。长度启发式（len(id)==32）一律收敛到本包的 legacy 函数，
// 身份推断只允许出现在转换边界（adapter）。
package identity

import "strconv"

// OpenID QQ OpenAPI 用户 OpenID（真实 ID，32 位字符串）。
type OpenID string

// OpenGroupID QQ OpenAPI 群 OpenID（真实 ID，32 位字符串）。
type OpenGroupID string

// VirtualUserID 下游 OneBot 用户虚拟 ID（数字字符串，映射自 OpenID）。
type VirtualUserID string

// VirtualGroupID 下游 OneBot 群虚拟 ID（数字字符串，映射自 OpenGroupID）。
type VirtualGroupID string

// UIN 机器人/用户的 QQ 号，可能未知（*UIN == nil）。
type UIN uint64

// AppID 机器人应用 ID。
type AppID uint64

func (id OpenID) String() string         { return string(id) }
func (id OpenGroupID) String() string    { return string(id) }
func (id VirtualUserID) String() string  { return string(id) }
func (id VirtualGroupID) String() string { return string(id) }

// Int64 虚拟用户 ID 转 int64（下游常见形态）。
func (id VirtualUserID) Int64() (int64, error) {
	return strconv.ParseInt(string(id), 10, 64)
}

// Int64 虚拟群 ID 转 int64。
func (id VirtualGroupID) Int64() (int64, error) {
	return strconv.ParseInt(string(id), 10, 64)
}

// Uint64 AppID 转 uint64。
func (id AppID) Uint64() uint64 { return uint64(id) }

// Uint64 UIN 转 uint64。
func (id UIN) Uint64() uint64 { return uint64(id) }

// VirtualUserIDFromInt64 从 int64 构造虚拟用户 ID。
func VirtualUserIDFromInt64(v int64) VirtualUserID {
	return VirtualUserID(strconv.FormatInt(v, 10))
}

// VirtualGroupIDFromInt64 从 int64 构造虚拟群 ID。
func VirtualGroupIDFromInt64(v int64) VirtualGroupID {
	return VirtualGroupID(strconv.FormatInt(v, 10))
}

// UINFromString 解析 UIN 字符串。
func UINFromString(s string) (UIN, error) {
	u, err := strconv.ParseUint(s, 10, 64)
	return UIN(u), err
}

// AppIDFromString 解析 AppID 字符串。
func AppIDFromString(s string) (AppID, error) {
	u, err := strconv.ParseUint(s, 10, 64)
	return AppID(u), err
}

package identity

import (
	"context"
	"errors"
)

// 映射错误。
var (
	// ErrNotFound 映射不存在。
	ErrNotFound = errors.New("identity: mapping not found")
	// ErrAmbiguous 存在多个候选映射。
	ErrAmbiguous = errors.New("identity: ambiguous mapping")
)

// UserRef 用户引用：OpenID 与 VirtualUserID 二选一，由 Resolver 判定并解析。
// 原始 Action 永远不变，禁止把真实 OpenID 写回原参数字段。
type UserRef struct {
	OpenID        *OpenID
	VirtualUserID *VirtualUserID
}

// GroupRef 群引用：OpenGroupID 与 VirtualGroupID 二选一。
type GroupRef struct {
	OpenID         *OpenGroupID
	VirtualGroupID *VirtualGroupID
}

// ResolvedUser 解析后的用户身份。
type ResolvedUser struct {
	OpenID        OpenID
	VirtualUserID VirtualUserID
	UIN           *UIN // 可能未知
}

// ResolvedGroup 解析后的群身份。
type ResolvedGroup struct {
	OpenID         OpenGroupID
	VirtualGroupID VirtualGroupID
}

// IdentityResolver 将用户/群引用解析为具体身份。
// 实现方（adapter）是唯一允许 string ↔ typed identity 转换的边界。
type IdentityResolver interface {
	ResolveUser(ctx context.Context, ref UserRef) (ResolvedUser, error)
	ResolveGroup(ctx context.Context, ref GroupRef) (ResolvedGroup, error)
}

// ---- 引用构造辅助（二选一字段） ----

// UserRefFromOpenID 从 OpenID 构造用户引用。
func UserRefFromOpenID(id OpenID) UserRef { return UserRef{OpenID: &id} }

// UserRefFromVirtualID 从虚拟用户 ID 构造用户引用。
func UserRefFromVirtualID(id VirtualUserID) UserRef {
	return UserRef{VirtualUserID: &id}
}

// GroupRefFromOpenID 从 OpenGroupID 构造群引用。
func GroupRefFromOpenID(id OpenGroupID) GroupRef { return GroupRef{OpenID: &id} }

// GroupRefFromVirtualID 从虚拟群 ID 构造群引用。
func GroupRefFromVirtualID(id VirtualGroupID) GroupRef {
	return GroupRef{VirtualGroupID: &id}
}

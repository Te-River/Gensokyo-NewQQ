// Package idmapresolver 基于现有 idmap 存储实现 identity.IdentityResolver。
//
// 这是 string ↔ typed identity 的转换边界：业务层一律通过 identity.IdentityResolver
// 解析身份，不得直接做长度启发式推断。
package idmapresolver

import (
	"context"
	"errors"

	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/internal/domain/identity"
)

// IDMapResolver 将用户/群引用解析为具体身份。
type IDMapResolver struct{}

// New 创建基于 idmap 的身份解析器。
func New() *IDMapResolver { return &IDMapResolver{} }

// ResolveUser 解析用户身份：OpenID → 虚拟用户 ID（写入映射），虚拟用户 ID → OpenID。
func (r *IDMapResolver) ResolveUser(_ context.Context, ref identity.UserRef) (identity.ResolvedUser, error) {
	if ref.OpenID != nil {
		virtualID, err := idmap.StoreUserID(string(*ref.OpenID))
		if err != nil {
			return identity.ResolvedUser{}, err
		}
		return identity.ResolvedUser{
			OpenID:        *ref.OpenID,
			VirtualUserID: identity.VirtualUserIDFromInt64(virtualID),
		}, nil
	}
	if ref.VirtualUserID != nil {
		openID, err := idmap.RetrieveUserID(string(*ref.VirtualUserID))
		if errors.Is(err, idmap.ErrKeyNotFound) {
			return identity.ResolvedUser{}, identity.ErrNotFound
		}
		if err != nil {
			return identity.ResolvedUser{}, err
		}
		return identity.ResolvedUser{
			OpenID:        identity.OpenID(openID),
			VirtualUserID: *ref.VirtualUserID,
		}, nil
	}
	return identity.ResolvedUser{}, identity.ErrNotFound
}

// ResolveGroup 解析群身份：OpenGroupID → 虚拟群 ID，虚拟群 ID → OpenGroupID。
func (r *IDMapResolver) ResolveGroup(_ context.Context, ref identity.GroupRef) (identity.ResolvedGroup, error) {
	if ref.OpenID != nil {
		virtualID, err := idmap.StoreGroupID(string(*ref.OpenID))
		if err != nil {
			return identity.ResolvedGroup{}, err
		}
		return identity.ResolvedGroup{
			OpenID:         *ref.OpenID,
			VirtualGroupID: identity.VirtualGroupIDFromInt64(virtualID),
		}, nil
	}
	if ref.VirtualGroupID != nil {
		openID, err := idmap.RetrieveGroupID(string(*ref.VirtualGroupID))
		if errors.Is(err, idmap.ErrKeyNotFound) {
			return identity.ResolvedGroup{}, identity.ErrNotFound
		}
		if err != nil {
			return identity.ResolvedGroup{}, err
		}
		return identity.ResolvedGroup{
			OpenID:         identity.OpenGroupID(openID),
			VirtualGroupID: *ref.VirtualGroupID,
		}, nil
	}
	return identity.ResolvedGroup{}, identity.ErrNotFound
}

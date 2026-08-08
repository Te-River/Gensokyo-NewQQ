package idmapresolver

import (
	"context"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/internal/domain/identity"
)

// TestIDMapResolverRoundTrip 真实 idmap 集成：OpenID → 虚拟 ID → OpenID 往返。
// 使用 t.Chdir 隔离 bbolt 数据库文件，避免污染工作目录。
func TestIDMapResolverRoundTrip(t *testing.T) {
	t.Chdir(t.TempDir())
	defer idmap.CloseDB()

	r := New()
	const userOpenID = identity.OpenID("01234567890123456789012345678901")
	const groupOpenID = identity.OpenGroupID("abcdef0123456789abcdef0123456789")

	ctx := context.Background()

	u, err := r.ResolveUser(ctx, identity.UserRefFromOpenID(userOpenID))
	if err != nil {
		t.Fatalf("ResolveUser(openid): %v", err)
	}
	if u.OpenID != userOpenID {
		t.Fatalf("OpenID = %q, want %q", u.OpenID, userOpenID)
	}
	if u.VirtualUserID == "" {
		t.Fatal("VirtualUserID is empty")
	}

	back, err := r.ResolveUser(ctx, identity.UserRefFromVirtualID(u.VirtualUserID))
	if err != nil {
		t.Fatalf("ResolveUser(virtual): %v", err)
	}
	if back.OpenID != userOpenID {
		t.Fatalf("reverse OpenID = %q, want %q", back.OpenID, userOpenID)
	}
	if back.VirtualUserID != u.VirtualUserID {
		t.Fatalf("reverse VirtualUserID = %q, want %q", back.VirtualUserID, u.VirtualUserID)
	}

	g, err := r.ResolveGroup(ctx, identity.GroupRefFromOpenID(groupOpenID))
	if err != nil {
		t.Fatalf("ResolveGroup(openid): %v", err)
	}
	if g.OpenID != groupOpenID || g.VirtualGroupID == "" {
		t.Fatalf("resolved group = %+v", g)
	}

	gback, err := r.ResolveGroup(ctx, identity.GroupRefFromVirtualID(g.VirtualGroupID))
	if err != nil {
		t.Fatalf("ResolveGroup(virtual): %v", err)
	}
	if gback.OpenID != groupOpenID {
		t.Fatalf("reverse group OpenID = %q, want %q", gback.OpenID, groupOpenID)
	}
}

// TestIDMapResolverNotFound 查询不存在的虚拟 ID 应返回 ErrNotFound。
func TestIDMapResolverNotFound(t *testing.T) {
	t.Chdir(t.TempDir())
	defer idmap.CloseDB()

	r := New()
	if _, err := r.ResolveUser(context.Background(), identity.UserRefFromVirtualID("999999999")); err != identity.ErrNotFound {
		t.Fatalf("err = %v, want ErrNotFound", err)
	}
}

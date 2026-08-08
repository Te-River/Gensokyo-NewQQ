package identity

import (
	"context"
	"errors"
	"testing"
)

// memResolver 内存版 Resolver，用于接口契约测试。
type memResolver struct {
	userByOpen map[OpenID]VirtualUserID
	groupByOpen map[OpenGroupID]VirtualGroupID
}

func newMemResolver() *memResolver {
	return &memResolver{
		userByOpen:  map[OpenID]VirtualUserID{},
		groupByOpen: map[OpenGroupID]VirtualGroupID{},
	}
}

func (m *memResolver) ResolveUser(_ context.Context, ref UserRef) (ResolvedUser, error) {
	if ref.OpenID != nil {
		vid, ok := m.userByOpen[*ref.OpenID]
		if !ok {
			return ResolvedUser{}, ErrNotFound
		}
		return ResolvedUser{OpenID: *ref.OpenID, VirtualUserID: vid}, nil
	}
	if ref.VirtualUserID != nil {
		for open, vid := range m.userByOpen {
			if vid == *ref.VirtualUserID {
				return ResolvedUser{OpenID: open, VirtualUserID: vid}, nil
			}
		}
		return ResolvedUser{}, ErrNotFound
	}
	return ResolvedUser{}, ErrNotFound
}

func (m *memResolver) ResolveGroup(_ context.Context, ref GroupRef) (ResolvedGroup, error) {
	if ref.OpenID != nil {
		vid, ok := m.groupByOpen[*ref.OpenID]
		if !ok {
			return ResolvedGroup{}, ErrNotFound
		}
		return ResolvedGroup{OpenID: *ref.OpenID, VirtualGroupID: vid}, nil
	}
	if ref.VirtualGroupID != nil {
		for open, vid := range m.groupByOpen {
			if vid == *ref.VirtualGroupID {
				return ResolvedGroup{OpenID: open, VirtualGroupID: vid}, nil
			}
		}
		return ResolvedGroup{}, ErrNotFound
	}
	return ResolvedGroup{}, ErrNotFound
}

func TestResolverResolveUserByOpenID(t *testing.T) {
	r := newMemResolver()
	r.userByOpen["01234567890123456789012345678901"] = "1001"

	got, err := r.ResolveUser(context.Background(), UserRefFromOpenID("01234567890123456789012345678901"))
	if err != nil {
		t.Fatalf("ResolveUser: %v", err)
	}
	if got.OpenID != "01234567890123456789012345678901" || got.VirtualUserID != "1001" {
		t.Fatalf("resolved = %+v", got)
	}
}

func TestResolverResolveUserByVirtualID(t *testing.T) {
	r := newMemResolver()
	r.userByOpen["01234567890123456789012345678901"] = "1001"

	got, err := r.ResolveUser(context.Background(), UserRefFromVirtualID("1001"))
	if err != nil {
		t.Fatalf("ResolveUser: %v", err)
	}
	if got.OpenID != "01234567890123456789012345678901" {
		t.Fatalf("openid = %q", got.OpenID)
	}
}

func TestResolverNotFound(t *testing.T) {
	r := newMemResolver()
	if _, err := r.ResolveUser(context.Background(), UserRefFromVirtualID("999")); !errors.Is(err, ErrNotFound) {
		t.Fatalf("err = %v, want ErrNotFound", err)
	}
	if _, err := r.ResolveGroup(context.Background(), GroupRefFromOpenID("nope")); !errors.Is(err, ErrNotFound) {
		t.Fatalf("err = %v, want ErrNotFound", err)
	}
}

func TestResolverEmptyRef(t *testing.T) {
	r := newMemResolver()
	if _, err := r.ResolveUser(context.Background(), UserRef{}); !errors.Is(err, ErrNotFound) {
		t.Fatalf("err = %v, want ErrNotFound", err)
	}
}

func TestResolvedTargetString(t *testing.T) {
	g := ResolvedGroup{OpenID: "g1", VirtualGroupID: "2001"}
	u := ResolvedUser{OpenID: "u1", VirtualUserID: "1001"}
	if got := (ResolvedTarget{Kind: TargetGroup, Group: &g}).String(); got != "group:2001" {
		t.Fatalf("target string = %q", got)
	}
	if got := (ResolvedTarget{Kind: TargetPrivate, User: &u}).String(); got != "private:1001" {
		t.Fatalf("target string = %q", got)
	}
}

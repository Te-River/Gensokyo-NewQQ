package Processor

// union_group_openid 透传测试（官方 2026-11-16 群事件新增字段）：
// 带字段的事件 → 上报 JSON 含 union_group_openid；不带 → 该 key 不出现。
//
// 文件名 zzzzzz 前缀：本文件经 archLoadConfig 注入 config 单例，
// 必须排在依赖 nil 单例的 processor_test.go 之后（Go 按文件名字母序执行）。

import (
	"testing"
	"time"

	"github.com/tencent-connect/botgo/dto"
)

// unionOf 取上报 map 中的 union_group_openid 与是否存在
func unionOf(payload map[string]interface{}) (string, bool) {
	v, ok := payload["union_group_openid"]
	if !ok {
		return "", false
	}
	s, _ := v.(string)
	return s, true
}

// waitPayload 等待 fakeServerClient 收到一条上报（上报在 BroadcastMessageToAll 的 goroutine 里）
func waitPayload(t *testing.T, f *fakeServerClient) map[string]interface{} {
	t.Helper()
	select {
	case <-f.ch:
	case <-time.After(2 * time.Second):
		t.Fatal("未收到任何上报")
	}
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.msgs[len(f.msgs)-1]
}

func TestGroupMemberUnionGroupOpenID(t *testing.T) {
	archLoadConfig(t, "legacy")

	t.Run("带字段", func(t *testing.T) {
		p, f := newArchProcessors()
		p.ProcessGroupMember(&dto.GroupMemberEvent{
			EventID:          "e-mem-1",
			GroupOpenID:      "group-openid-mem",
			MemberOpenID:     "member-openid-mem",
			Timestamp:        int64(1700000000),
			UnionGroupOpenID: "ug-member-1",
		}, "GROUP_MEMBER_ADD")

		if got, ok := unionOf(waitPayload(t, f)); !ok || got != "ug-member-1" {
			t.Errorf("union_group_openid = %q(存在=%v), want ug-member-1", got, ok)
		}
	})

	t.Run("不带字段", func(t *testing.T) {
		p, f := newArchProcessors()
		p.ProcessGroupMember(&dto.GroupMemberEvent{
			EventID:      "e-mem-2",
			GroupOpenID:  "group-openid-mem2",
			MemberOpenID: "member-openid-mem2",
			Timestamp:    int64(1700000001),
		}, "GROUP_MEMBER_ADD")

		if got, ok := unionOf(waitPayload(t, f)); ok {
			t.Errorf("字段为空时不应上报 union_group_openid，实际 %q", got)
		}
	})
}

func TestGroupJoinRequestUnionGroupOpenID(t *testing.T) {
	archLoadConfig(t, "legacy")

	t.Run("带字段", func(t *testing.T) {
		p, f := newArchProcessors()
		p.ProcessGroupJoinRequest(&dto.GroupJoinRequestEvent{
			JoinRequestID:    "jr-1",
			GroupOpenID:      "group-openid-jr",
			MemberOpenID:     "member-openid-jr",
			ApplyAt:          "2026-11-16T10:00:00+08:00",
			UnionGroupOpenID: "ug-join-1",
		})

		if got, ok := unionOf(waitPayload(t, f)); !ok || got != "ug-join-1" {
			t.Errorf("union_group_openid = %q(存在=%v), want ug-join-1", got, ok)
		}
	})

	t.Run("不带字段", func(t *testing.T) {
		p, f := newArchProcessors()
		p.ProcessGroupJoinRequest(&dto.GroupJoinRequestEvent{
			JoinRequestID: "jr-2",
			GroupOpenID:   "group-openid-jr2",
			MemberOpenID:  "member-openid-jr2",
			ApplyAt:       "2026-11-16T10:00:01+08:00",
		})

		if got, ok := unionOf(waitPayload(t, f)); ok {
			t.Errorf("字段为空时不应上报 union_group_openid，实际 %q", got)
		}
	})
}

func TestGroupMessageUnionGroupOpenID(t *testing.T) {
	archLoadConfig(t, "legacy")

	newMsg := func(groupID, union string) *dto.WSGroupATMessageData {
		return &dto.WSGroupATMessageData{
			ID:               "m-union-1",
			Content:          "hi",
			GroupID:          groupID,
			UnionGroupOpenID: union,
			Author:           &dto.User{ID: "user-openid-union"},
		}
	}

	t.Run("带字段", func(t *testing.T) {
		p, f := newArchProcessors()
		if err := p.ProcessGroupMessage(newMsg("group-openid-msg", "ug-msg-1")); err != nil {
			t.Fatalf("ProcessGroupMessage 失败: %v", err)
		}
		if got, ok := unionOf(waitPayload(t, f)); !ok || got != "ug-msg-1" {
			t.Errorf("union_group_openid = %q(存在=%v), want ug-msg-1", got, ok)
		}
	})

	t.Run("不带字段", func(t *testing.T) {
		p, f := newArchProcessors()
		if err := p.ProcessGroupMessage(newMsg("group-openid-msg2", "")); err != nil {
			t.Fatalf("ProcessGroupMessage 失败: %v", err)
		}
		if got, ok := unionOf(waitPayload(t, f)); ok {
			t.Errorf("字段为空时不应上报 union_group_openid，实际 %q", got)
		}
	})
}

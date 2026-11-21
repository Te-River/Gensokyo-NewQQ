package Processor

import (
	"strings"
	"testing"

	"github.com/tencent-connect/botgo/dto"
)

// 群聊：nickname 与 card 同源，card_and_nick 配置最高优先
func TestResolveNicknameAndCardGroup(t *testing.T) {
	tests := []struct {
		name        string
		author      *dto.User
		cardAndNick string
		wantNick    string
		wantCard    string
	}{
		{
			name:     "有群名片时群名片优先于用户名",
			author:   &dto.User{MemberNickname: "群名片A", Username: "昵称B"},
			wantNick: "群名片A",
			wantCard: "群名片A",
		},
		{
			name:     "无群名片时回退用户名且不设置card",
			author:   &dto.User{Username: "昵称B"},
			wantNick: "昵称B",
			wantCard: "",
		},
		{
			name:        "card_and_nick配置覆盖群名片与用户名",
			author:      &dto.User{MemberNickname: "群名片A", Username: "昵称B"},
			cardAndNick: "统一",
			wantNick:    "统一",
			wantCard:    "统一",
		},
		{
			name:     "群名片与用户名都为空",
			author:   &dto.User{},
			wantNick: "",
			wantCard: "",
		},
		{
			name:     "author为nil时不panic",
			author:   nil,
			wantNick: "",
			wantCard: "",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			gotNick, gotCard := resolveNicknameAndCard(tt.author, tt.cardAndNick)
			if gotNick != tt.wantNick || gotCard != tt.wantCard {
				t.Fatalf("resolveNicknameAndCard() = (%q, %q), want (%q, %q)", gotNick, gotCard, tt.wantNick, tt.wantCard)
			}
		})
	}
}

// 私聊：调用方传空 cardAndNick 且忽略 card，只校验 nickname = member_nickname || username
func TestResolveNicknameAndCardPrivate(t *testing.T) {
	tests := []struct {
		name     string
		author   *dto.User
		wantNick string
	}{
		{name: "无群名片时取用户名", author: &dto.User{Username: "昵称B"}, wantNick: "昵称B"},
		{name: "有群名片时取群名片", author: &dto.User{MemberNickname: "X", Username: "昵称B"}, wantNick: "X"},
		{name: "都为空时为空串", author: &dto.User{}, wantNick: ""},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			gotNick, _ := resolveNicknameAndCard(tt.author, "")
			if gotNick != tt.wantNick {
				t.Fatalf("nickname = %q, want %q", gotNick, tt.wantNick)
			}
		})
	}
}

// stubFilterHooks 注入敏感词替换规则与开关状态，测试结束后还原
func stubFilterHooks(t *testing.T, enabled bool, replace func(string) string) {
	t.Helper()
	oldWordIN, oldEnabled := checkWordIN, changeWordEnabled
	changeWordEnabled = func() bool { return enabled }
	checkWordIN = replace
	t.Cleanup(func() {
		checkWordIN, changeWordEnabled = oldWordIN, oldEnabled
	})
}

// maskSensitive 模拟 acnode.CheckWordIN 的入站替换规则
func maskSensitive(s string) string {
	return strings.ReplaceAll(s, "敏感", "**")
}

// 群消息：username 与 member_nickname 都含敏感词 → 两者都被替换，nickname/card 取过滤后的值
func TestFilterAuthorFieldsGroupBothFields(t *testing.T) {
	stubFilterHooks(t, true, maskSensitive)
	author := &dto.User{Username: "敏感用户", MemberNickname: "敏感名片"}

	FilterAuthorFields(author)

	if author.Username != "**用户" || author.MemberNickname != "**名片" {
		t.Fatalf("过滤后字段 = (%q, %q), want (%q, %q)", author.Username, author.MemberNickname, "**用户", "**名片")
	}
	gotNick, gotCard := resolveNicknameAndCard(author, "")
	if gotNick != "**名片" || gotCard != "**名片" {
		t.Fatalf("nickname/card = (%q, %q), want (%q, %q)", gotNick, gotCard, "**名片", "**名片")
	}
}

// 群消息：只有 member_nickname 含敏感词 → nickname/card 被替换，username 保持原样
func TestFilterAuthorFieldsGroupOnlyMemberNickname(t *testing.T) {
	stubFilterHooks(t, true, maskSensitive)
	author := &dto.User{Username: "普通用户", MemberNickname: "敏感名片"}

	FilterAuthorFields(author)

	if author.Username != "普通用户" {
		t.Fatalf("username = %q, want %q（不含敏感词不应改动）", author.Username, "普通用户")
	}
	gotNick, gotCard := resolveNicknameAndCard(author, "")
	if gotNick != "**名片" || gotCard != "**名片" {
		t.Fatalf("nickname/card = (%q, %q), want (%q, %q)", gotNick, gotCard, "**名片", "**名片")
	}
}

// 私聊：member_nickname 含敏感词 → sender.nickname 被替换（私聊不上报 card）
func TestFilterAuthorFieldsPrivateMemberNickname(t *testing.T) {
	stubFilterHooks(t, true, maskSensitive)
	author := &dto.User{Username: "普通用户", MemberNickname: "敏感名片"}

	FilterAuthorFields(author)

	// 私聊忽略 card 返回值（resolveNicknameAndCard 不区分场景，调用方不写 sender.card）
	gotNick, _ := resolveNicknameAndCard(author, "")
	if gotNick != "**名片" {
		t.Fatalf("nickname = %q, want %q", gotNick, "**名片")
	}
}

// 过滤关闭（配置未启用）→ username 与 member_nickname 都不变
func TestFilterAuthorFieldsDisabled(t *testing.T) {
	stubFilterHooks(t, false, maskSensitive)
	author := &dto.User{Username: "敏感用户", MemberNickname: "敏感名片"}

	FilterAuthorFields(author)

	if author.Username != "敏感用户" || author.MemberNickname != "敏感名片" {
		t.Fatalf("过滤关闭后字段 = (%q, %q), want 原值", author.Username, author.MemberNickname)
	}
}

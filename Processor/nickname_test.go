package Processor

import (
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

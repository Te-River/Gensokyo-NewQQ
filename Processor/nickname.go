package Processor

import (
	"github.com/hoshinonyaruko/gensokyo/acnode"
	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/tencent-connect/botgo/dto"
)

// checkWordIN / changeWordEnabled 抽成变量，便于测试注入替换规则与开关状态
var (
	checkWordIN       = acnode.CheckWordIN
	changeWordEnabled = config.GetEnableChangeWord
)

// FilterAuthorFields 对入站 author 的 username 与 member_nickname 应用同一敏感词替换。
// 必须在 resolveNicknameAndCard 之前调用，否则 sender.nickname / sender.card 会取到未过滤的群名片。
// 敏感词过滤未启用（enable_change_word）时不改动任何字段。
func FilterAuthorFields(author *dto.User) {
	if author == nil || !changeWordEnabled() {
		return
	}
	if author.Username != "" {
		author.Username = checkWordIN(author.Username)
	}
	if author.MemberNickname != "" {
		author.MemberNickname = checkWordIN(author.MemberNickname)
	}
}

// resolveNicknameAndCard 统一解析入站上报的 sender.nickname / sender.card：
//
//	nickname: card_and_nick 配置（非空） > member_nickname > username
//	card:     card_and_nick 配置（非空） > member_nickname > 空（不上报 card）
//
// 私聊没有群名片概念：调用方传入空 cardAndNick，并忽略返回的 card。
func resolveNicknameAndCard(author *dto.User, cardAndNick string) (nickname, card string) {
	if cardAndNick != "" {
		return cardAndNick, cardAndNick
	}
	if author == nil {
		return "", ""
	}
	if author.MemberNickname != "" {
		return author.MemberNickname, author.MemberNickname
	}
	return author.Username, ""
}

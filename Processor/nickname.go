package Processor

import "github.com/tencent-connect/botgo/dto"

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

package handlers

// 回执序列化口径测试：OneBot 客户端的 action 回执必须与 Rust 端逐字节一致——
// 带签名的 URL 中 & 原样输出，不转义成 \u0026。

import (
	"strings"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/tencent-connect/botgo/dto"
)

func TestGetUserInfoReceiptKeepsAmpersand(t *testing.T) {
	client := &contractTestClient{}
	const avatar = "https://example.com/a?x=1&y=2"
	mock := &userInfoMockAPI{info: &dto.UserInfo{Avatar: avatar}}

	raw, err := GetUserInfo(client, nil, mock, callapi.ActionMessage{
		Action: "get_user_info",
		Params: callapi.ParamsContent{UserID: strings.Repeat("c", 32)},
	})
	if err != nil {
		t.Fatalf("GetUserInfo 返回错误: %v", err)
	}
	if !strings.Contains(raw, avatar) {
		t.Errorf("回执未原样保留 &：%s", raw)
	}
	if strings.Contains(raw, `\u0026`) {
		t.Errorf("回执把 & 转义成 \\u0026：%s", raw)
	}
}

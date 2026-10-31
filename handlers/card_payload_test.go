package handlers

import (
	"context"
	"encoding/json"
	"strings"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/dto/keyboard"
	"github.com/tencent-connect/botgo/openapi"
)

// ---------- 卡片单点校验（P-G2） ----------

// cardMockAPI 捕获 PostGroupMessage / PostC2CMessage，其余方法走嵌入接口兜底。
type cardMockAPI struct {
	openapi.OpenAPI

	groupCalls int
	lastGroup  dto.APIMessage
	c2cCalls   int
	lastC2C    dto.APIMessage
	sendErr    error
}

func (m *cardMockAPI) PostGroupMessage(_ context.Context, _ string, msg dto.APIMessage) (*dto.GroupMessageResponse, error) {
	m.groupCalls++
	m.lastGroup = msg
	if m.sendErr != nil {
		return nil, m.sendErr
	}
	return &dto.GroupMessageResponse{Message: &dto.Message{ID: "card-group-msg-id"}}, nil
}

func (m *cardMockAPI) PostC2CMessage(_ context.Context, _ string, msg dto.APIMessage) (*dto.C2CMessageResponse, error) {
	m.c2cCalls++
	m.lastC2C = msg
	if m.sendErr != nil {
		return nil, m.sendErr
	}
	return &dto.C2CMessageResponse{Message: &dto.Message{ID: "card-c2c-msg-id"}}, nil
}

// cardRecorderClient 记录 handler 交给客户端的回执 payload。
type cardRecorderClient struct {
	response map[string]interface{}
}

func (c *cardRecorderClient) SendMessage(message map[string]interface{}) error {
	c.response = message
	return nil
}

// retcodeOf 解析 handler 返回的 JSON 回执里的 retcode。
func retcodeOf(t *testing.T, raw string) int {
	t.Helper()
	var resp struct {
		RetCode int `json:"retcode"`
	}
	if err := json.Unmarshal([]byte(raw), &resp); err != nil {
		t.Fatalf("回执不是合法 JSON: %q (%v)", raw, err)
	}
	return resp.RetCode
}

// 四字段齐全的合法取值：单测里只让被测字段非法，避免撞上「必填」拦截。
const (
	cardPicOK = "https://example.com/p.png"
	cardURLOK = "https://example.com"
)

func TestBuildTuwenCardTitleBoundaries(t *testing.T) {
	// 50 个汉字是 150 字节：通过即证明按字符而非按字节计数
	if _, err := buildTuwenCard(strings.Repeat("字", cardTitleMaxRunes), "描述", cardPicOK, cardURLOK); err != nil {
		t.Fatalf("title 50 字符应通过: %v", err)
	}
	_, err := buildTuwenCard(strings.Repeat("字", cardTitleMaxRunes+1), "描述", cardPicOK, cardURLOK)
	if err == nil {
		t.Fatalf("title 51 字符应被拒绝")
	}
	if !strings.Contains(err.Error(), "title") {
		t.Fatalf("错误信息应指明字段 title, got %v", err)
	}
}

func TestBuildTuwenCardDescriptionBoundaries(t *testing.T) {
	if _, err := buildTuwenCard("标题", strings.Repeat("述", cardDescMaxRunes), cardPicOK, cardURLOK); err != nil {
		t.Fatalf("description 200 字符应通过: %v", err)
	}
	_, err := buildTuwenCard("标题", strings.Repeat("述", cardDescMaxRunes+1), cardPicOK, cardURLOK)
	if err == nil {
		t.Fatalf("description 201 字符应被拒绝")
	}
	if !strings.Contains(err.Error(), "description") {
		t.Fatalf("错误信息应指明字段 description, got %v", err)
	}
}

func TestBuildTuwenCardRejectsNonHTTPS(t *testing.T) {
	cases := []struct {
		name string
		pic  string
		url  string
	}{
		{"http pic", "http://example.com/a.png", cardURLOK},
		{"http url", cardPicOK, "http://example.com"},
		{"本地路径 pic", "C:/tmp/a.png", cardURLOK},
		{"相对路径 url", cardPicOK, "./a.html"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			_, err := buildTuwenCard("标题", "描述", tc.pic, tc.url)
			if err == nil {
				t.Fatalf("非 https 链接应被拒绝: pic=%q url=%q", tc.pic, tc.url)
			}
			if !strings.Contains(err.Error(), "https") {
				t.Fatalf("错误信息应提示 https, got %v", err)
			}
		})
	}
}

func TestBuildTuwenCardRejectsOverlongURL(t *testing.T) {
	longURL := "https://example.com/" + strings.Repeat("a", cardURLMaxLen)
	if _, err := buildTuwenCard("标题", "描述", cardPicOK, longURL); err == nil {
		t.Fatalf("超 1024 的 url 应被拒绝")
	}
}

func TestBuildTuwenCardMissingTitle(t *testing.T) {
	if _, err := buildTuwenCard("   ", "描述", cardPicOK, cardURLOK); err == nil {
		t.Fatalf("缺 title 应被拒绝")
	}
}

// 官方要求 content 四字段齐全：desc / url / pic_url 缺一即拒绝（F1）。
func TestBuildTuwenCardRequiresAllFields(t *testing.T) {
	cases := []struct {
		name string
		desc string
		pic  string
		url  string
		hook string
	}{
		{"缺 description", "", cardPicOK, cardURLOK, "description"},
		{"缺 pic_url", "描述", "", cardURLOK, "pic_url"},
		{"缺 url", "描述", cardPicOK, "", "url"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			_, err := buildTuwenCard("标题", tc.desc, tc.pic, tc.url)
			if err == nil {
				t.Fatalf("%s 应被拒绝", tc.name)
			}
			if !strings.Contains(err.Error(), tc.hook) {
				t.Fatalf("错误信息应指明字段 %s, got %v", tc.hook, err)
			}
		})
	}

	// 空白字符等同缺失（trim 后判空）
	if _, err := buildTuwenCard("标题", "   ", cardPicOK, cardURLOK); err == nil {
		t.Fatalf("description 全空白应被拒绝")
	}
}

// url / pic_url 长度按字节计（与 Rust 侧同口径）：400 个汉字 = 1200 字节 > 1024。
func TestBuildTuwenCardURLByteBoundary(t *testing.T) {
	exact := "https://e.com/" + strings.Repeat("a", cardURLMaxLen-len("https://e.com/"))
	if len(exact) != cardURLMaxLen {
		t.Fatalf("构造的 url 应正好 %d 字节, got %d", cardURLMaxLen, len(exact))
	}
	if _, err := buildTuwenCard("标题", "描述", cardPicOK, exact); err != nil {
		t.Fatalf("1024 字节的 url 应通过: %v", err)
	}

	multibyte := "https://e.com/" + strings.Repeat("中", 400) // 400 字符 / 1200 字节
	if _, err := buildTuwenCard("标题", "描述", cardPicOK, multibyte); err == nil {
		t.Fatalf("按字节超 1024 的 url 应被拒绝（不能按字符计长）")
	}
}

// [CQ:card] 参数别名 desc/description、pic/pic_url 均可用。
func TestBuildCardFromJSONAliases(t *testing.T) {
	raw := `{"title":"标题","description":"描述","pic_url":"https://example.com/p.png","url":"https://example.com"}`
	card, err := buildCardFromJSON(raw)
	if err != nil {
		t.Fatalf("别名参数应放行: %v", err)
	}
	if card.Content.Description != "描述" || card.Content.PicURL != "https://example.com/p.png" {
		t.Fatalf("别名未生效: %+v", card.Content)
	}
	if card.Type != "tuwen" {
		t.Fatalf("card.type 应固定为 tuwen, got %q", card.Type)
	}
}

// 群聊纯卡片：msg_type=8 且 content 为空。
func TestSendGroupMsgPureCardMsgType8(t *testing.T) {
	mock := &cardMockAPI{}
	client := &cardRecorderClient{}
	msg := callapi.ActionMessage{
		Action: "send_group_msg",
		Params: callapi.ParamsContent{
			GroupID: openID32('g'),
			UserID:  openID32('u'),
			Message: "[CQ:card,title=标题,desc=描述,pic=https://example.com/p.png,url=https://example.com]",
		},
		Echo: "card-group-pure",
	}
	if _, err := HandleSendGroupMsg(client, nil, mock, msg); err != nil {
		t.Fatalf("HandleSendGroupMsg 返回错误: %v", err)
	}
	if mock.groupCalls != 1 {
		t.Fatalf("纯卡片应发送一次 PostGroupMessage, got %d", mock.groupCalls)
	}
	mtc, ok := mock.lastGroup.(*dto.MessageToCreate)
	if !ok {
		t.Fatalf("应发送 MessageToCreate, got %T", mock.lastGroup)
	}
	if mtc.MsgType != 8 {
		t.Fatalf("MsgType = %d, want 8", mtc.MsgType)
	}
	if mtc.Content != "" {
		t.Fatalf("msg_type=8 时 content 必须为空, got %q", mtc.Content)
	}
	if mtc.Card == nil || mtc.Card.Type != "tuwen" || mtc.Card.Content == nil {
		t.Fatalf("卡片载荷异常: %+v", mtc.Card)
	}
}

// 单聊纯卡片：msg_type=8 可达，载荷与群聊同构。
func TestSendPrivateMsgPureCardMsgType8(t *testing.T) {
	mock := &cardMockAPI{}
	client := &cardRecorderClient{}
	msg := callapi.ActionMessage{
		Action: "send_private_msg",
		Params: callapi.ParamsContent{
			UserID:  openID32('u'),
			Message: "[CQ:card,title=标题,desc=描述,pic=https://example.com/p.png,url=https://example.com]",
		},
		Echo: "card-private-pure",
	}
	if _, err := HandleSendPrivateMsg(client, nil, mock, msg); err != nil {
		t.Fatalf("HandleSendPrivateMsg 返回错误: %v", err)
	}
	if mock.c2cCalls != 1 {
		t.Fatalf("单聊纯卡片应发送一次 PostC2CMessage, got %d", mock.c2cCalls)
	}
	mtc, ok := mock.lastC2C.(*dto.MessageToCreate)
	if !ok {
		t.Fatalf("应发送 MessageToCreate, got %T", mock.lastC2C)
	}
	if mtc.MsgType != 8 || mtc.Content != "" {
		t.Fatalf("单聊卡片应 msg_type=8 且 content 为空, got MsgType=%d Content=%q", mtc.MsgType, mtc.Content)
	}
	if mtc.Card == nil || mtc.Card.Content == nil || mtc.Card.Content.Title != "标题" {
		t.Fatalf("单聊卡片载荷异常: %+v", mtc.Card)
	}
}

// 缺 title：群聊与单聊都回 retcode=1400。
func TestCardMissingTitleReturns1400(t *testing.T) {
	cases := []struct {
		name string
		run  func(*cardMockAPI, *cardRecorderClient) (string, error)
	}{
		{"group", func(mock *cardMockAPI, client *cardRecorderClient) (string, error) {
			return HandleSendGroupMsg(client, nil, mock, callapi.ActionMessage{
				Action: "send_group_msg",
				Params: callapi.ParamsContent{
					GroupID: openID32('g'),
					UserID:  openID32('u'),
					Message: "[CQ:card,desc=描述,pic=https://example.com/p.png,url=https://example.com]",
				},
				Echo: "card-group-no-title",
			})
		}},
		{"private", func(mock *cardMockAPI, client *cardRecorderClient) (string, error) {
			return HandleSendPrivateMsg(client, nil, mock, callapi.ActionMessage{
				Action: "send_private_msg",
				Params: callapi.ParamsContent{
					UserID:  openID32('u'),
					Message: "[CQ:card,desc=描述,pic=https://example.com/p.png,url=https://example.com]",
				},
				Echo: "card-private-no-title",
			})
		}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			mock := &cardMockAPI{}
			client := &cardRecorderClient{}
			raw, err := tc.run(mock, client)
			if err != nil {
				t.Fatalf("handler 返回错误: %v", err)
			}
			if got := retcodeOf(t, raw); got != 1400 {
				t.Fatalf("缺 title 应回 1400, got %d (%s)", got, raw)
			}
			if mock.groupCalls+mock.c2cCalls != 0 {
				t.Fatalf("校验失败不应发送消息")
			}
		})
	}
}

// 缺 description / url / pic_url：群聊与单聊都回 1400（四字段必填，F1）。
func TestCardMissingRequiredFieldReturns1400(t *testing.T) {
	cases := []struct {
		name string
		text string
	}{
		{"缺 description", "[CQ:card,title=标题,pic=" + cardPicOK + ",url=" + cardURLOK + "]"},
		{"缺 url", "[CQ:card,title=标题,desc=描述,pic=" + cardPicOK + "]"},
		{"缺 pic_url", "[CQ:card,title=标题,desc=描述,url=" + cardURLOK + "]"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			for _, channel := range []string{"group", "private"} {
				mock := &cardMockAPI{}
				client := &cardRecorderClient{}
				msg := callapi.ActionMessage{
					Action: "send_group_msg",
					Params: callapi.ParamsContent{
						GroupID: openID32('g'),
						UserID:  openID32('u'),
						Message: tc.text,
					},
					Echo: "card-missing-" + channel,
				}
				var raw string
				var err error
				if channel == "group" {
					raw, err = HandleSendGroupMsg(client, nil, mock, msg)
				} else {
					msg.Action = "send_private_msg"
					msg.Params.GroupID = nil
					raw, err = HandleSendPrivateMsg(client, nil, mock, msg)
				}
				if err != nil {
					t.Fatalf("%s 返回错误: %v", channel, err)
				}
				if got := retcodeOf(t, raw); got != 1400 {
					t.Fatalf("%s 应回 1400, got %d (%s)", channel, got, raw)
				}
				if mock.groupCalls+mock.c2cCalls != 0 {
					t.Fatalf("%s 校验失败不应发送消息", channel)
				}
			}
		})
	}
}

// 卡片 + @全员 同现：卡片胜出会清空 content，标签发不出去但额度已扣，
// 故在占额度之前直接回 1400（F2）。
func TestSendGroupMsgCardWithAtEveryoneReturns1400(t *testing.T) {
	store := useMemAtEveryone(t)
	mock := &cardMockAPI{}
	client := &cardRecorderClient{}
	msg := callapi.ActionMessage{
		Action: "send_group_msg",
		Params: callapi.ParamsContent{
			GroupID: openID32('g'),
			UserID:  openID32('u'),
			Message: "[CQ:at,qq=all] [CQ:card,title=标题,desc=描述,pic=" + cardPicOK + ",url=" + cardURLOK + "]",
		},
		Echo: "card-at-all",
	}
	raw, err := HandleSendGroupMsg(client, nil, mock, msg)
	if err != nil {
		t.Fatalf("HandleSendGroupMsg 返回错误: %v", err)
	}
	if got := retcodeOf(t, raw); got != 1400 {
		t.Fatalf("卡片 + @全员 应回 1400, got %d (%s)", got, raw)
	}
	if !strings.Contains(raw, "全员") || !strings.Contains(raw, "卡片") {
		t.Fatalf("错误信息应说明卡片与 @全员 冲突, got %s", raw)
	}
	if mock.groupCalls != 0 {
		t.Fatalf("组合被拒时不应发送消息, got %d", mock.groupCalls)
	}
	if len(store.data) != 0 {
		t.Fatalf("组合被拒时不得占用 @全员 额度, got %v", store.data)
	}
}

// markdown 与卡片同现：markdown 优先，卡片（含非法卡片）直接丢弃且不报错。
func TestSendGroupMsgMarkdownBeatsCard(t *testing.T) {
	mock := &cardMockAPI{}
	client := &cardRecorderClient{}
	msg := callapi.ActionMessage{
		Action: "send_group_msg",
		Params: callapi.ParamsContent{
			GroupID: openID32('g'),
			UserID:  openID32('u'),
			Message: []interface{}{
				mdSegment("### 公告"),
				textSegment("[CQ:card,desc=缺标题,pic=https://example.com/p.png]"),
			},
		},
		Echo: "card-md-wins",
	}
	raw, err := HandleSendGroupMsg(client, nil, mock, msg)
	if err != nil {
		t.Fatalf("HandleSendGroupMsg 返回错误: %v", err)
	}
	if mock.groupCalls != 1 {
		t.Fatalf("应发送一次 PostGroupMessage, got %d", mock.groupCalls)
	}
	mtc, ok := mock.lastGroup.(*dto.MessageToCreate)
	if !ok {
		t.Fatalf("应发送 MessageToCreate, got %T", mock.lastGroup)
	}
	if mtc.MsgType != 2 {
		t.Fatalf("markdown 优先应发 MsgType=2, got %d (回执 %s)", mtc.MsgType, raw)
	}
}

// ---------- P-G1 尾巴：keyboard 里的 qqbot-media:// 不得当本地路径 ----------

func TestResolveKeyboardImagesKeepsQQBotMedia(t *testing.T) {
	ref := "qqbot-media://image/v1/token-abc"
	kb := &keyboard.MessageKeyboard{Content: &keyboard.CustomKeyboard{
		Rows: []*keyboard.Row{{
			Buttons: []*keyboard.Button{{
				RenderData: &keyboard.RenderData{
					Label:        "![img](" + ref + ")",
					VisitedLabel: "![img](" + ref + ")",
				},
			}},
		}},
	}}
	// apiv2 传 nil：一旦误判为本地路径会走进 safeLocalPath/读文件分支，
	// 这里只要标签被原样保留就说明走的是透传分支。
	ResolveKeyboardImages(kb, nil)
	got := kb.Content.Rows[0].Buttons[0].RenderData
	if got.Label != "![img]("+ref+")" || got.VisitedLabel != "![img]("+ref+")" {
		t.Fatalf("qqbot-media 引用应原样透传, got label=%q visited=%q", got.Label, got.VisitedLabel)
	}
}

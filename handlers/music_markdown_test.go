package handlers

import (
	"testing"

	"github.com/tencent-connect/botgo/dto"
	"github.com/tidwall/gjson"
)

// [CQ:music] 模板废弃后不再产出 markdown：空 content 的 msg_type=2 是无效载荷（F5）。
func TestParseQQMusicReturnsNilMarkdown(t *testing.T) {
	orig := qqMusicSongInfo
	qqMusicSongInfo = func(string) (gjson.Result, error) {
		return gjson.Parse(`{"track_info":{"mid":"mid-abc"}}`), nil
	}
	t.Cleanup(func() { qqMusicSongInfo = orig })

	md, kb, err := parseQQMuiscMDData("song-1")
	if err != nil {
		t.Fatalf("解析失败: %v", err)
	}
	if md != nil {
		t.Fatalf("不应再返回 markdown（空载荷无效）: %+v", md)
	}
	if kb == nil || kb.Content == nil || len(kb.Content.Rows) == 0 {
		t.Fatalf("应返回带分享按钮的键盘: %+v", kb)
	}
}

// 调用方兜底：msg_type 不再是 2、content 非空、键盘保留（分享链接由按钮承载）。
func TestGenerateGroupMessageQQMusicFallsBackToText(t *testing.T) {
	orig := qqMusicSongInfo
	qqMusicSongInfo = func(string) (gjson.Result, error) {
		return gjson.Parse(`{"track_info":{"mid":"mid-abc"}}`), nil
	}
	t.Cleanup(func() { qqMusicSongInfo = orig })

	got := generateGroupMessage("msg-id", "event-id", map[string][]string{"qqmusic": {"song-1"}}, "", 0, &cardMockAPI{}, openID32('g'))
	mtc, ok := got.(*dto.MessageToCreate)
	if !ok {
		t.Fatalf("应返回 MessageToCreate, got %T", got)
	}
	if mtc.Markdown != nil {
		t.Fatalf("不应再带空 markdown: %+v", mtc.Markdown)
	}
	if mtc.MsgType == 2 || mtc.Content == "" {
		t.Fatalf("应退化为文本消息, got MsgType=%d Content=%q", mtc.MsgType, mtc.Content)
	}
	if mtc.Keyboard == nil {
		t.Fatalf("键盘应保留")
	}
}

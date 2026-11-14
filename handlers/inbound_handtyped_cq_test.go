package handlers

// 入站 array 上报：用户手打的 CQ 码原样留在 text 段，不做类型化解析。
//
// 依据（P72/P73 与 Rust 侧 array_value 对齐的口径）：ConvertToSegmentedMessage
// （message_parser.go:2182）对正文只做整段切片 + 入站 <@OpenID> 正则替换，
// 完全不解析用户手打的 CQ；array_value 开关由调用方 Processor 决定是否走该函数
// （如 ProcessGroupNormalMessage.go:176-179），函数本身没有开关参数——
// 它产出段数组，就是 array_value=true 时的上报形态。
// Rust 仓已有对应用例，本文件补上 Go 侧同一结论，防重构时被无声改坏。
//
// 文件名按字母序先于 zzzz_inbound_at_order_test.go：本文件不注入 config 单例
// （无需 remove_at/convert_other_at 等开关），保持 zzz 系列"最后加载完整模板"的前提。

import (
	"testing"

	"github.com/tencent-connect/botgo/dto"
)

// 手打 CQ 原文字面值：断言要求段载荷与之逐字节相等
const (
	handtypedCqPoke  = "[CQ:poke,qq=10086]"
	handtypedCqJSON  = `[CQ:json,data={"app":"com.tencent.miniapp"}]`
	handtypedCqNode  = "[CQ:node,id=123456]"
	handtypedCqFace  = "[CQ:face,id=1]"
	handtypedCqImage = "[CQ:image,file=handtyped.png]"
)

// handtypedCqMsg 构造全量群消息（GROUP_MESSAGE_CREATE）入站数据。
// 两条路径各用一份：ConvertToSegmentedMessage 会就地改写 msg.Content。
func handtypedCqMsg(content string) *dto.WSGroupMessageData {
	return &dto.WSGroupMessageData{
		Content: content,
		Author:  &dto.User{ID: "USER_OPENID"},
	}
}

// TestInboundHandtypedCqStaysInText 手打 poke/json/node/face 在 array 上报里
// 整段留在唯一 text 段：段数 >1 或段类型不是 text，即说明该码被类型化解析。
func TestInboundHandtypedCqStaysInText(t *testing.T) {
	cases := []struct {
		name    string
		content string
	}{
		{"poke", "看这个 " + handtypedCqPoke + " 一下"},
		{"json", "卡片 " + handtypedCqJSON + " 尾部"},
		{"node", "节点 " + handtypedCqNode + " 尾部"},
		{"face", "表情 " + handtypedCqFace + " 尾部"},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			segs := ConvertToSegmentedMessage(handtypedCqMsg(c.content))
			if len(segs) != 1 {
				t.Fatalf("手打 CQ 不应产生额外段（不得生成对应类型化段）: got %+v", segs)
			}
			if segType, _ := segs[0]["type"].(string); segType != "text" {
				t.Fatalf("手打 CQ 应整段留在 text 段: got 段类型 %q, segs=%+v", segType, segs)
			}
			data, _ := segs[0]["data"].(map[string]interface{})
			text, _ := data["text"].(string)
			if text != c.content {
				t.Fatalf("text 段载荷必须与原文逐字节一致:\n  got:  %q\n  want: %q", text, c.content)
			}

			// raw_message 由 string 路径产出（Processor/ProcessGroupNormalMessage.go:205
			// RawMessage: messageText），其文本也不得解析手打 CQ，两路径必须表达同一序列。
			raw := RevertTransformedText(handtypedCqMsg(c.content), "group", nil, nil, 100, 200, false)
			if raw != c.content {
				t.Errorf("raw_message 文本被改写:\n  got:  %q\n  want: %q", raw, c.content)
			}
			atAssertConsistency(t, raw, segs)
		})
	}
}

// TestInboundHandtypedCqImageKeepsTextAndRealAttachment 手打 [CQ:image] 留在 text，
// 真实 attachments 图片仍产 image 段（两件事互不干扰）。
func TestInboundHandtypedCqImageKeepsTextAndRealAttachment(t *testing.T) {
	content := "看图 " + handtypedCqImage + " 结尾"
	newMsg := func() *dto.WSGroupMessageData {
		msg := handtypedCqMsg(content)
		msg.Attachments = []*dto.MessageAttachment{
			{FileName: "pic.png", ContentType: "image/png", URL: "https://x.com/p.png"},
		}
		return msg
	}

	segs := ConvertToSegmentedMessage(newMsg())
	// 段序：text（手打 CQ 原文）→ image（真实附件），手打码不额外产 image 段
	if len(segs) != 2 {
		t.Fatalf("应为 1 个 text 段 + 1 个 image 段: got %+v", segs)
	}
	if segType, _ := segs[0]["type"].(string); segType != "text" {
		t.Fatalf("手打 [CQ:image] 应留在 text 段: got 段类型 %q, segs=%+v", segType, segs)
	}
	textData, _ := segs[0]["data"].(map[string]interface{})
	if text, _ := textData["text"].(string); text != content {
		t.Fatalf("text 段载荷必须保留手打 CQ 原文:\n  got:  %q\n  want: %q", text, content)
	}
	if segType, _ := segs[1]["type"].(string); segType != "image" {
		t.Fatalf("真实附件应产 image 段: got %+v", segs[1])
	}
	imageData, _ := segs[1]["data"].(map[string]interface{})
	if imageData["file"] != "pic.image" {
		t.Errorf("附件 image.file: got %v, want pic.image", imageData["file"])
	}
	if imageData["url"] != "https://x.com/p.png" {
		t.Errorf("附件 image.url: got %v, want https://x.com/p.png", imageData["url"])
	}

	// string 路径把手打 CQ 原文保留在前、真实附件追加为 image CQ（subType=0 仅字符串路径携带）
	wantRaw := content + "[CQ:image,file=pic.image,subType=0,url=https://x.com/p.png]"
	if raw := RevertTransformedText(newMsg(), "group", nil, nil, 100, 200, false); raw != wantRaw {
		t.Errorf("raw_message:\n  got:  %q\n  want: %q", raw, wantRaw)
	}
}

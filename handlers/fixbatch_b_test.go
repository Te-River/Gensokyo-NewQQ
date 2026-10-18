package handlers

// fixbatch B 批次回归保护：nil 语音附件不 panic、message_type=103 空 msg_elements 不产出空 reply 段。

import (
	"testing"

	"github.com/tencent-connect/botgo/dto"
)

// TestFixBatchB_NilVoiceAttachment nil 附件（attachments:[null]）返回 false 而非 panic
func TestFixBatchB_NilVoiceAttachment(t *testing.T) {
	if extra, ok := voiceAttachmentExtra(nil); ok {
		t.Fatalf("nil 附件不应产出 record 单元: %+v", extra)
	}
	extras := collectMessageEventExtras(&dto.Message{Attachments: []*dto.MessageAttachment{nil}})
	if len(extras) != 0 {
		t.Fatalf("nil 附件不应产出任何单元: %+v", extras)
	}
}

// TestFixBatchB_EmptyMsgElementsNoReplySeg 103 且 msg_elements 为空时不产出空 reply 段
func TestFixBatchB_EmptyMsgElementsNoReplySeg(t *testing.T) {
	for _, seg := range messageEventSegments(&dto.Message{MessageType: 103}) {
		if seg["type"] == "reply" {
			t.Fatalf("空 msg_elements 不应产出 reply 段: %+v", seg)
		}
	}
	for _, extra := range collectElementExtras([]*dto.MsgElement{{MessageType: 103}}) {
		if extra.kind == "reply" {
			t.Fatalf("嵌套空 msg_elements 不应产出 reply 单元: %+v", extra)
		}
	}
	// 非 103 但携带 msg_elements 的既有分支保留：仍产出 reply 段
	segs := messageEventSegments(&dto.Message{MsgElements: []*dto.MsgElement{{MsgIdx: "REFIDX_x"}}})
	if len(segs) != 1 || segs[0]["type"] != "reply" {
		t.Fatalf("非 103 的 msg_elements 应保留 reply 段: %+v", segs)
	}
	if data, _ := segs[0]["data"].(map[string]interface{}); data["msg_idx"] != "REFIDX_x" {
		t.Fatalf("reply 段 msg_idx 错误: %+v", segs[0])
	}
}

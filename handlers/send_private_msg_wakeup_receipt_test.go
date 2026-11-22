package handlers

// 回归（v016）：send_private_msg_wakeup 上游失败时只推 c2c_wakeup_resp notice、
// 不回 action 回执，客户端只能等到超时。失败分支必须「notice + 一条 1500 回执」。

import (
	"context"
	"errors"
	"strings"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

// wakeupFailMock 上游 PostC2CMessage 一律失败（模拟 code:40034122）。
type wakeupFailMock struct {
	openapi.OpenAPI
	err      error
	c2cCalls int
}

func (m *wakeupFailMock) PostC2CMessage(_ context.Context, _ string, _ dto.APIMessage) (*dto.C2CMessageResponse, error) {
	m.c2cCalls++
	return nil, m.err
}

// wakeupRecClient 记录 handler 通过 client 发出的全部 payload（notice + action 回执）。
type wakeupRecClient struct {
	msgs []map[string]interface{}
}

func (c *wakeupRecClient) SendMessage(message map[string]interface{}) error {
	c.msgs = append(c.msgs, message)
	return nil
}

// splitWakeupPayloads 按有无 retcode 字段拆出 action 回执与 notice。
func splitWakeupPayloads(msgs []map[string]interface{}) (receipts, notices []map[string]interface{}) {
	for _, m := range msgs {
		if _, ok := m["retcode"]; ok {
			receipts = append(receipts, m)
			continue
		}
		notices = append(notices, m)
	}
	return receipts, notices
}

// TestWakeupUpstreamFailureRepliesAndNotifies 上游失败：notice 仍推一次，且恰好一条 1500/failed 回执。
func TestWakeupUpstreamFailureRepliesAndNotifies(t *testing.T) {
	mock := &wakeupFailMock{err: errors.New("code:400, text:上游 API 错误: code=40034122 召回消息已达区间上限")}
	client := &wakeupRecClient{}
	msg := callapi.ActionMessage{
		Action: "send_private_msg_wakeup",
		Params: callapi.ParamsContent{UserID: openID32('u'), Message: "召回"},
		Echo:   "p3",
	}

	raw, err := HandleSendPrivateMsgWakeup(client, nil, mock, msg)
	if err != nil {
		t.Fatalf("HandleSendPrivateMsgWakeup 返回错误: %v", err)
	}
	if mock.c2cCalls != 1 {
		t.Errorf("非超时错误不应重试, PostC2CMessage 调用 %d 次", mock.c2cCalls)
	}

	receipts, notices := splitWakeupPayloads(client.msgs)
	if len(receipts) != 1 {
		t.Fatalf("应恰好收到一条 action 回执, got %d (%v)", len(receipts), client.msgs)
	}
	if len(notices) != 1 {
		t.Fatalf("notice 仍应推送一次, got %d", len(notices))
	}
	if notices[0]["notice_type"] != "c2c_wakeup_resp" || notices[0]["status"] != "failed" {
		t.Errorf("notice 口径被改动: %v", notices[0])
	}

	receipt := receipts[0]
	if receipt["retcode"] != float64(sendFailRetCode) || receipt["status"] != "failed" {
		t.Errorf("回执 retcode/status = %v/%v, want 1500/failed", receipt["retcode"], receipt["status"])
	}
	if receipt["echo"] != "p3" {
		t.Errorf("回执 echo = %v, want p3", receipt["echo"])
	}
	if receipt["data"] != nil {
		t.Errorf("回执 data 应为 null, got %v", receipt["data"])
	}
	if text, _ := receipt["message"].(string); !strings.Contains(text, "40034122") {
		t.Errorf("回执 message 应带真实错误: %q", text)
	}

	got := decodeActionReceipt(t, raw)
	if got["retcode"] != float64(sendFailRetCode) || got["status"] != "failed" || got["data"] != nil {
		t.Errorf("返回体应与回执一致(1500/failed/null): %v", got)
	}
}

// TestWakeupSuccessReceiptUnchanged 成功路径保持既有口径：字面量回执 + 一条 notice，不多发回执。
func TestWakeupSuccessReceiptUnchanged(t *testing.T) {
	mock := &mockMDSendOpenAPI{}
	client := &wakeupRecClient{}
	msg := callapi.ActionMessage{
		Action: "send_private_msg_wakeup",
		Params: callapi.ParamsContent{UserID: openID32('u'), Message: "召回"},
		Echo:   "p4",
	}

	raw, err := HandleSendPrivateMsgWakeup(client, nil, mock, msg)
	if err != nil {
		t.Fatalf("HandleSendPrivateMsgWakeup 返回错误: %v", err)
	}
	if raw != `{"status": "ok", "retcode": 0}` {
		t.Fatalf("成功回执口径被改动: %q", raw)
	}

	receipts, notices := splitWakeupPayloads(client.msgs)
	if len(receipts) != 0 {
		t.Errorf("成功路径不应额外发 action 回执: %v", receipts)
	}
	if len(notices) != 1 {
		t.Errorf("成功 notice 仍应推送一次, got %d", len(notices))
	}
}

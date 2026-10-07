package Processor

// 第二轮修复复验（tester 独立网）：P4 —— ev.Raw 经广播链到达 BroadcastMessageToAll，
// 全通道失败时 downtime 兜底可触发。
//
// 文件名 zzzzz 前缀：在 zzz/zzzz 的 config 注入测试之后运行。

import (
	"context"
	"testing"
	"time"

	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/structs"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

// round2DowntimeAPI 记录 downtime 兜底的 PostGroupMessage。
type round2DowntimeAPI struct {
	openapi.OpenAPI
	ch chan *dto.MessageToCreate
}

func (m *round2DowntimeAPI) PostGroupMessage(_ context.Context, _ string, msg dto.APIMessage) (*dto.GroupMessageResponse, error) {
	if gm, ok := msg.(*dto.MessageToCreate); ok {
		select {
		case m.ch <- gm:
		default:
		}
	}
	return &dto.GroupMessageResponse{Message: &dto.Message{ID: "dt-1"}}, nil
}

// TestP4RawReachesDowntimeFallback 钉死 P4：new 入站的 ev.Raw 经
// Publisher→RawSender→BroadcastInboundWithRaw 到达 BroadcastMessageToAll；
// 无 WS 客户端（全通道失败）时 downtime 兜底向 QQ 发送维护提示。
func TestP4RawReachesDowntimeFallback(t *testing.T) {
	archLoadConfig(t, "new")
	api := &round2DowntimeAPI{ch: make(chan *dto.MessageToCreate, 1)}
	p := &Processors{Apiv2: api, Settings: &structs.Settings{}}
	SetInbound(p)

	data := &dto.WSGroupATMessageData{ID: "m1", Content: "hi", GroupID: "g1", Author: &dto.User{ID: "u1"}}
	payload := map[string]interface{}{"post_type": "message", "message_type": "group", "raw_message": "hi"}
	p.dispatchInbound(data, payload, func() { t.Fatal("new 不应走 legacy 广播") })

	select {
	case gm := <-api.ch:
		if gm.Content != config.GetDowntimeMessage() {
			t.Fatalf("downtime 兜底内容异常: got %q want %q", gm.Content, config.GetDowntimeMessage())
		}
	case <-time.After(2 * time.Second):
		t.Fatal("ev.Raw 未到达 BroadcastMessageToAll：downtime 兜底未触发")
	}
}

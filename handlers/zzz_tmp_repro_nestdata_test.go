package handlers

// 临时复现测试（验后删）：用户真实环境 payload（嵌套 data.data markdown 段，无文本段）
// × legacy/shadow/new 三模式，记录 PostGroupMessage 调用与 Markdown.Content 实际值。
// config 单例在测试进程内不可安全初始化（见 cq_threepath_matrix_test.go 头注释），
// 三模式经分发等价链驱动：
//   - legacy: parseMessageContent 默认分发（instance==nil 回退 legacy）→ HandleSendGroupMsg 全集成
//   - shadow: 对外产物 = legacy 产物（runCQParseShadow 只 diff 不改写），另捕获 diff 日志
//   - new:    parseMessageContentCQParse 分支函数直驱（message_parser.go:662 同名分支）

import (
	"encoding/base64"
	"encoding/json"
	"strings"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/tencent-connect/botgo/dto"
)

// userPayloadSegments 反序列化用户原始 JSON payload 的 message 数组（逐字节保真）。
func userPayloadSegments(t *testing.T) []interface{} {
	t.Helper()
	const raw = `[
	  {"data":{"data":{"markdown":{"content":"### AT 检测结果\n> ❌ 消息未 @ 机器人"}}},"type":"markdown"}
	]`
	var segs []interface{}
	if err := json.Unmarshal([]byte(raw), &segs); err != nil {
		t.Fatalf("用户 payload 反序列化失败: %v", err)
	}
	return segs
}

func TestTmpReproNestedDataMarkdown(t *testing.T) {
	segs := userPayloadSegments(t)
	msg := callapi.ActionMessage{
		Action: "send_group_msg",
		Params: callapi.ParamsContent{
			GroupID: openID32('g'),
			UserID:  openID32('u'),
			Message: segs,
		},
		Echo: "tmp-repro-nested-data",
	}

	// ---- legacy：全集成 HandleSendGroupMsg ----
	mock := &mockMDSendOpenAPI{}
	client := &mdGateTestClient{}
	if _, err := HandleSendGroupMsg(client, nil, mock, msg); err != nil {
		t.Fatalf("legacy HandleSendGroupMsg err: %v", err)
	}
	_, legacyItems := parseMessageContentLegacy(msg.Params, msg, client, nil, nil)
	legacyMDContent := ""
	if mock.groupCalls > 0 {
		if mtc, ok := mock.lastGroup.(*dto.MessageToCreate); ok && mtc.Markdown != nil {
			legacyMDContent = mtc.Markdown.Content
		}
	}
	t.Logf("[legacy] PostGroupMessage 调用=%d md.Content=%q", mock.groupCalls, legacyMDContent)
	t.Logf("[legacy] foundItems[markdown]=%v", legacyItems["markdown"])

	// ---- shadow：产物 = legacy，另捕获 diff 日志 ----
	shadowLog := captureStdout(t, func() {
		legacyText, legacyItems2 := parseMessageContentLegacy(msg.Params, msg, nil, nil, nil)
		runCQParseShadow(msg.Params, msg, nil, nil, nil, legacyText, legacyItems2)
	})
	t.Logf("[shadow] diff 日志: %s", shadowLog)

	// ---- new：分支函数直驱 + handler 消费复现 ----
	newText, newItems, newPend := parseMessageContentCQParse(msg.Params, msg, nil, nil, nil)
	t.Logf("[new] text=%q pend=%v", newText, newPend)
	t.Logf("[new] foundItems 全量=%v", newItems)

	// gate 复现（send_group_msg.go:513）
	gateOpen := newText != "" || len(newPend) > 0 || len(newItems["markdown"]) > 0 || len(newItems["keyboard"]) > 0
	t.Logf("[new] 发送门槛(:513) 通过=%v", gateOpen)

	newMDContent := ""
	if mdItems := newItems["markdown"]; len(mdItems) > 0 {
		md, _ := parseMarkdownFromMessage(mdItems[0])
		if md != nil {
			newMDContent = md.Content
		}
	}
	t.Logf("[new] parseMarkdownFromMessage md.Content=%q", newMDContent)

	// ---- 三模式行为表 ----
	t.Logf("==== 三模式行为表 ====")
	t.Logf("legacy: PostGroupMessage=%d 次, md.Content=%q", mock.groupCalls, legacyMDContent)
	t.Logf("shadow: 产物=legacy, diff 日志含 [cqparse-shadow]=%v", strings.Contains(shadowLog, "[cqparse-shadow]"))
	t.Logf("new:    门槛通过=%v, foundItems[markdown] 长度=%d, md.Content=%q",
		gateOpen, len(newItems["markdown"]), newMDContent)

	// 逐字节对比 legacy vs new 的 markdown payload
	legacyMD := legacyItems["markdown"]
	newMD := newItems["markdown"]
	if len(legacyMD) == 0 {
		t.Fatalf("legacy 应产出 markdown payload: %v", legacyItems)
	}
	if len(newMD) == 0 {
		t.Errorf("REPRO CONFIRMED: new 模式丢失 markdown payload（legacy=%d 项, new=0 项）", len(legacyMD))
	} else if legacyMD[0] != newMD[0] {
		ld, _ := base64.StdEncoding.DecodeString(legacyMD[0])
		nd, _ := base64.StdEncoding.DecodeString(newMD[0])
		t.Errorf("REPRO CONFIRMED(wrap): payload 不一致\nlegacy b64=%s\nnew    b64=%s\nlegacy decoded=%s\nnew    decoded=%s",
			legacyMD[0], newMD[0], ld, nd)
	} else {
		t.Logf("payload 逐字节一致: %s", legacyMD[0])
	}
}

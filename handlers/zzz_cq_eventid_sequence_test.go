package handlers_test

// 同一条消息内多个 member 动作型 CQ 码按序出现时的 event_id 序列回归网（外部测试包 handlers_test）。
//
// 背景：cqparse.ExecutePending 把同一个 eventID *string 指针依次交给每个动作码，
// 逐码 last-write-wins 写穿（member add 命中写入 / member remove 显式清空），
// 返回的 outs[i].EventID 只是「每码当时的快照」。因此汇总函数若按
// 「最后一个非空快照」取值，会作废 remove 的显式清空，让 new 链带着
// 已作废的 event_id 发出退群回复（legacy 语义：remove 清空即转主动推送）。
//
// 关键：动作码要走到 cqparse 的 pending 路径（进而调用 pickCQEventID），
// 必须 cq_parse_mode=new；模板默认 cq_parse_mode=legacy 时动作码由
// cqcode.go 的正则管道处理，那条路径直接写穿指针、天然无此缺陷。
// 故 new 用例取 arch_mode=new + cq_parse_mode=new，
// legacy 基线取模板默认 arch_mode=legacy + cq_parse_mode=legacy。
// 期望值取自 legacy 实跑结果，而非测试臆造。

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/hoshinonyaruko/gensokyo/adapter/qq"
	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/echo"
	"github.com/hoshinonyaruko/gensokyo/handlers"
	"github.com/hoshinonyaruko/gensokyo/internal/application/outbound"
	"github.com/hoshinonyaruko/gensokyo/template"
)

// seqGroup*：本组用例专用会话群/用户（不复用 zzz_minor_parity_test.go 的常量，
// 保证 echo 的 event_id 内存缓存在本文件内互不干扰）。
const (
	seqGroupA = "88888888888888888888888888888888"
	seqUserA  = "99999999999999999999999999999999"
	seqGroupB = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
	seqUserB  = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
	seqGroupC = "cccccccccccccccccccccccccccccccc"
	seqUserC  = "dddddddddddddddddddddddddddddddd"
	// seqEventID 为 add 命中缓存时写入的 event_id。
	seqEventID = "EV-SEQ"
)

// seqLoadConfig 注入 arch_mode + lazy_message_id=true（2000 主动推送场景需要）；
// mode=="new" 时额外置 cq_parse_mode=new，使动作型 CQ 码走 cqparse pending 路径。
func seqLoadConfig(t *testing.T, mode string) {
	t.Helper()
	yml := strings.Replace(template.ConfigTemplate, "arch_mode : new", "arch_mode : "+mode, 1)
	yml = strings.Replace(yml, "lazy_message_id : false", "lazy_message_id : true", 1)
	if mode == "new" {
		yml = strings.Replace(yml, "cq_parse_mode : legacy", "cq_parse_mode : new", 1)
	}
	path := filepath.Join(t.TempDir(), "config.yml")
	if err := os.WriteFile(path, []byte(yml), 0600); err != nil {
		t.Fatalf("写入测试配置失败: %v", err)
	}
	if _, err := config.LoadConfig(path, false); err != nil {
		t.Fatalf("加载测试配置失败: %v", err)
	}
}

// seqSend 在已由调用方 seqLoadConfig 装载配置的前提下，跑一次 send_group_msg，
// messageID 由调用方预置为 "2000"（主动推送）。
// 注意：调用方必须先 seqLoadConfig 再用 config.GetAppIDStr() 预置 echo 缓存，
// 否则缓存 key 会按旧配置的 appid 生成，与执行期查表 key 不匹配（add 静默未命中）。
func seqSend(t *testing.T, mode, groupID, userID, content, echoTag string) *parityOpenAPI {
	t.Helper()
	seqLoadConfig(t, mode)
	mock := &parityOpenAPI{}
	if mode == "new" {
		handlers.SetOutboundService(outbound.NewService(qq.NewQQSender(nil, mock), outbound.RetryPolicy{}))
		defer handlers.SetOutboundService(nil)
	}
	client := &adapterTestClient{}
	msg := callapi.ActionMessage{
		Action: "send_group_msg",
		Params: callapi.ParamsContent{GroupID: groupID, UserID: userID, Message: content},
		Echo:   echoTag,
	}
	if _, err := handlers.HandleSendGroupMsg(client, nil, mock, msg); err != nil {
		t.Fatalf("HandleSendGroupMsg 返回错误: %v", err)
	}
	return mock
}

// TestSeqNewAddThenRemoveClearsEventID 钉死主修复：同一条消息里 add（缓存命中，写入 event_id）
// 后紧跟 remove（显式清空）→ 最终 event_id 必须为空。messageID=="2000" 时
// 也不得回填缓存（remove 的清空是显式决定），即退群回复转为主动推送。
// 同一输入跑 legacy 取得基线，证明期望值来自 legacy 语义。
func TestSeqNewAddThenRemoveClearsEventID(t *testing.T) {
	// new 链：add 命中缓存 → remove 显式清空
	seqLoadConfig(t, "new")
	echo.AddEvnetIDv2(config.GetAppIDStr(), seqGroupA, seqEventID)
	echo.AddLazyMessageIdv2(seqGroupA, seqUserA, "2000", time.Now())
	mock := seqSend(t, "new", seqGroupA, seqUserA,
		"bye [CQ:member,type=add,user_id="+seqUserA+"] [CQ:member,type=remove,user_id="+seqUserA+"]",
		"seq-1-new")
	if got := paritySentEventID(t, mock); got != "" {
		t.Fatalf("add 命中后紧跟 remove：remove 的显式清空应作废 add 写入的 event_id: got %q want %q", got, "")
	}

	// legacy 基线：同样输入、同样预置缓存与 2000（cq_parse_mode 保持模板默认 legacy）
	seqLoadConfig(t, "legacy")
	echo.AddEvnetIDv2(config.GetAppIDStr(), seqGroupA, seqEventID)
	echo.AddLazyMessageIdv2(seqGroupA, seqUserA, "2000", time.Now())
	legacyMock := seqSend(t, "legacy", seqGroupA, seqUserA,
		"bye [CQ:member,type=add,user_id="+seqUserA+"] [CQ:member,type=remove,user_id="+seqUserA+"]",
		"seq-1-legacy")
	if got := paritySentEventID(t, legacyMock); got != "" {
		t.Fatalf("legacy 同序列最终 event_id 基线异常: got %q want %q", got, "")
	}
}

// TestSeqNewRemoveThenAddKeepsEventID 序列方向相反：remove（显式清空）后紧跟
// add（缓存命中，写入 event_id）→ 后码覆盖前码，最终 event_id 为 add 写入的值。
// 该用例防的是「汇总时只保留首个非空 / 只认清空」的反向过度修正。
func TestSeqNewRemoveThenAddKeepsEventID(t *testing.T) {
	seqLoadConfig(t, "new")
	echo.AddEvnetIDv2(config.GetAppIDStr(), seqGroupB, seqEventID)
	echo.AddLazyMessageIdv2(seqGroupB, seqUserB, "2000", time.Now())
	mock := seqSend(t, "new", seqGroupB, seqUserB,
		"hi [CQ:member,type=remove,user_id="+seqUserB+"] [CQ:member,type=add,user_id="+seqUserB+"]",
		"seq-2-new")
	if got := paritySentEventID(t, mock); got != seqEventID {
		t.Fatalf("remove 后紧跟 add（命中）：后码应覆盖前码: got %q want %q", got, seqEventID)
	}
}

// TestSeqNewAddMissThenRemoveStaysEmpty 序列中 add 未命中缓存时不得产生空覆写：
// add 未命中只保留原值（EventIDSet=false），最终仍由 remove 决定为空，
// 即 add 未命中不得把 remove 的清空反转回非空。
func TestSeqNewAddMissThenRemoveStaysEmpty(t *testing.T) {
	// 故意不调用 echo.AddEvnetIDv2 → add 未命中。
	seqLoadConfig(t, "new")
	echo.AddLazyMessageIdv2(seqGroupC, seqUserC, "2000", time.Now())
	mock := seqSend(t, "new", seqGroupC, seqUserC,
		"bye [CQ:member,type=add,user_id="+seqUserC+"] [CQ:member,type=remove,user_id="+seqUserC+"]",
		"seq-3-new")
	if got := paritySentEventID(t, mock); got != "" {
		t.Fatalf("add 未命中后紧跟 remove：最终 event_id 应为空: got %q want %q", got, "")
	}
}

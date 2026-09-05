package handlers

// 本文件是 tester 独立复核用的对拍网（对应验证清单 3/4）：
//   1) 三路径对拍（new 模式）：同一逻辑动作以 字符串 CQ 码 / 消息段数组 / TRSS map
//      三种输入走 cqparse.Parse，断言 foundItems / pendings / 最终文本等价；
//   2) 三模式行为矩阵：md+kb 相邻输入（C1 场景）在 legacy（现状钉死，C1 仍在）、
//      new（正确拆分不吞噬）、shadow（diff 日志实证）下的行为差异。
// config 单例在测试进程内不可安全初始化（GetCQParseMode instance==nil 回退 legacy，
// 与生产默认一致），故 new/shadow 行为经同名分支函数 parseMessageContentCQParse /
// runCQParseShadow 直驱——与 message_parser.go:661 分发链逐分支等价。

import (
	"bytes"
	"encoding/base64"
	"encoding/json"
	"io"
	"os"
	"reflect"
	"strings"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/handlers/cqparse"
)

// trssJSON 将 TRSS 实际形态（JSON 原生数值/布尔）反序列化为 map 输入。
func trssJSON(t *testing.T, s string) map[string]interface{} {
	t.Helper()
	var m map[string]interface{}
	if err := json.Unmarshal([]byte(s), &m); err != nil {
		t.Fatalf("非法 TRSS JSON: %q, err=%v", s, err)
	}
	return m
}

// ---------- 三路径对拍（new 模式） ----------

// normPending 剥离 Raw 后比较（Raw 在字符串路径为原文、段路径为规范重渲染，
// 两路径 Raw 本就允许不同；Action/Params/Scope/DefaultGroupID 必须逐字段一致）。
type normPending struct {
	Action         string
	Params         map[string]string
	Scope          cqparse.Scope
	DefaultGroupID string
}

func normPendings(ps []cqparse.PendingAction) []normPending {
	out := make([]normPending, 0, len(ps))
	for _, p := range ps {
		out = append(out, normPending{Action: p.Action, Params: p.Params, Scope: p.Scope, DefaultGroupID: p.DefaultGroupID})
	}
	return out
}

// assertThreePathEq 对三输入分别 Parse，断言 (文本, foundItems, pendings) 三方等价。
func assertThreePathEq(t *testing.T, str, seg, mp cqparse.Input, d *cqparse.Deps) (
	strText, segText, mapText string,
) {
	t.Helper()
	strRes, strItems, strPend, err := cqparse.Parse(str, d)
	if err != nil {
		t.Fatalf("字符串路径 Parse 失败: %v", err)
	}
	segRes, segItems, segPend, err := cqparse.Parse(seg, d)
	if err != nil {
		t.Fatalf("段数组路径 Parse 失败: %v", err)
	}
	mapRes, mapItems, mapPend, err := cqparse.Parse(mp, d)
	if err != nil {
		t.Fatalf("TRSS map 路径 Parse 失败: %v", err)
	}

	if strRes != segRes || strRes != mapRes {
		t.Errorf("三路径文本不等价:\n  str:  %q\n  seg:  %q\n  map:  %q", strRes, segRes, mapRes)
	}
	if !reflect.DeepEqual(strItems, segItems) || !reflect.DeepEqual(strItems, mapItems) {
		t.Errorf("三路径 foundItems 不等价:\n  str:  %v\n  seg:  %v\n  map:  %v", strItems, segItems, mapItems)
	}
	npStr, npSeg, npMap := normPendings(strPend), normPendings(segPend), normPendings(mapPend)
	if !reflect.DeepEqual(npStr, npSeg) || !reflect.DeepEqual(npStr, npMap) {
		t.Errorf("三路径 pendings 不等价:\n  str:  %+v\n  seg:  %+v\n  map:  %+v", npStr, npSeg, npMap)
	}
	return strRes, segRes, mapRes
}

// TestCQParseThreePathKick_SetGroupUserIDs 对拍 set_group kick 带 user_ids。
func TestCQParseThreePathKick_SetGroupUserIDs(t *testing.T) {
	str := regStrIn(`[CQ:set_group,action=kick,group_id=100,user_ids=111&#44;222&#44;333,add_blacklist=true]`)
	seg := regSegIn([]map[string]interface{}{
		{"type": "set_group", "data": map[string]interface{}{
			"action": "kick", "group_id": "100",
			"user_ids": []interface{}{"111", "222", "333"}, "add_blacklist": true,
		}},
	}, regGroupID)
	// TRSS 实际形态：JSON 反序列化后数值/布尔为原生类型
	mp := regMapIn(trssJSON(t, `{"type":"set_group","data":{"action":"kick","group_id":100,"user_ids":[111,222,333],"add_blacklist":true}}`), regGroupID)

	_, _, _ = assertThreePathEq(t, str, seg, mp, regDeps(nil, nil))

	// 钉死 kick 参数语义：user_ids 逗号合并（C3）、add_blacklist 布尔归一（M8）
	str2, _, pend, err := cqparse.Parse(str, regDeps(nil, nil))
	if err != nil {
		t.Fatalf("Parse 失败: %v", err)
	}
	if str2 != "" {
		t.Errorf("动作码应整码移除: got %q", str2)
	}
	p := regPending(t, pend, "set_group")
	if p["action"] != "kick" || p["group_id"] != "100" || p["user_ids"] != "111,222,333" || p["add_blacklist"] != "true" {
		t.Errorf("kick 参数: got %+v", p)
	}
}

// TestCQParseThreePathGroupInfo_MultiField 对拍 group_info 多 field。
func TestCQParseThreePathGroupInfo_MultiField(t *testing.T) {
	const name, memo = "苍之group", "备忘录"
	const count = 42
	newFetcher := func() *fakeGroupInfoFetcher {
		return &fakeGroupInfoFetcher{info: &cqparse.GroupInfoData{Name: name, Memo: memo, MemberCount: count}}
	}

	allJSON, err := json.Marshal(map[string]string{"name": name, "memo": memo, "member_count": "42"})
	if err != nil {
		t.Fatalf("all JSON 构造失败: %v", err)
	}
	wantText := name + "42" + string(allJSON)

	// 字符串路径：三码同群 → 一次取数（N-G7）
	gi := newFetcher()
	strText, strItems, strPend, err := cqparse.Parse(
		regStrIn(`[CQ:group_info,field=name][CQ:group_info,field=member_count][CQ:group_info,field=all]`), regDeps(gi, nil))
	if err != nil {
		t.Fatalf("字符串路径 Parse 失败: %v", err)
	}
	if strText != wantText {
		t.Errorf("字符串路径展开文本:\n  got  %q\n  want %q", strText, wantText)
	}
	if gi.calls != 1 {
		t.Errorf("同群三码应一次取数: got %d", gi.calls)
	}
	if len(strPend) != 0 || len(strItems) != 0 {
		t.Errorf("group_info 不应产 pending/foundItems: pend=%v items=%v", strPend, strItems)
	}

	// 段数组路径：同构三段
	gi2 := newFetcher()
	segText, segItems, segPend, err := cqparse.Parse(regSegIn([]map[string]interface{}{
		{"type": "group_info", "data": map[string]interface{}{"field": "name"}},
		{"type": "group_info", "data": map[string]interface{}{"field": "member_count"}},
		{"type": "group_info", "data": map[string]interface{}{"field": "all"}},
	}, regGroupID), regDeps(gi2, nil))
	if err != nil {
		t.Fatalf("段数组路径 Parse 失败: %v", err)
	}
	if segText != wantText || gi2.calls != 1 || len(segPend) != 0 || len(segItems) != 0 {
		t.Errorf("段数组路径与字符串路径不等价: text=%q calls=%d pend=%v items=%v", segText, gi2.calls, segPend, segItems)
	}

	// TRSS map 路径：单 map 单码（field=all）
	gi3 := newFetcher()
	mapText, mapItems, mapPend, err := cqparse.Parse(
		regMapIn(trssJSON(t, `{"type":"group_info","data":{"field":"all"}}`), regGroupID), regDeps(gi3, nil))
	if err != nil {
		t.Fatalf("TRSS 路径 Parse 失败: %v", err)
	}
	if mapText != string(allJSON) || gi3.calls != 1 || len(mapPend) != 0 || len(mapItems) != 0 {
		t.Errorf("TRSS 路径 field=all 展开: text=%q calls=%d pend=%v items=%v", mapText, gi3.calls, mapPend, mapItems)
	}

	// memo 字段三路径等价兜底对拍
	strM := regStrIn(`[CQ:group_info,field=memo]`)
	segM := regSegIn([]map[string]interface{}{{"type": "group_info", "data": map[string]interface{}{"field": "memo"}}}, regGroupID)
	mapM := regMapIn(trssJSON(t, `{"type":"group_info","data":{"field":"memo"}}`), regGroupID)
	m1, m2, m3 := assertThreePathEq(t, strM, segM, mapM, regDeps(newFetcher(), nil))
	if m1 != memo || m2 != memo || m3 != memo {
		t.Errorf("memo 三路径展开: str=%q seg=%q map=%q want %q", m1, m2, m3, memo)
	}
}

// TestCQParseThreePathVoice_RecordKeys C-fix 对拍：voice 与 record 同义，
// 三输入形态（字符串 CQ 码 / 消息段数组 / TRSS map）foundItems 键统一为
// *_record（legacy 与下游 send_group_msg/send_private_msg 只认 record 系键），
// 不再产出 url_voice/base64_voice 等错位键。
func TestCQParseThreePathVoice_RecordKeys(t *testing.T) {
	str := regStrIn(`[CQ:voice,file=https://x.com/a.mp3]`)
	seg := regSegIn([]map[string]interface{}{
		{"type": "voice", "data": map[string]interface{}{"file": "https://x.com/a.mp3"}},
	}, regGroupID)
	mp := regMapIn(trssJSON(t, `{"type":"voice","data":{"file":"https://x.com/a.mp3"}}`), regGroupID)

	strText, segText, mapText := assertThreePathEq(t, str, seg, mp, regDeps(nil, nil))
	if strText != "" || segText != "" || mapText != "" {
		t.Errorf("voice 码应从正文移除: str=%q seg=%q map=%q", strText, segText, mapText)
	}

	// 键名归一断言：产出 url_records、无 *_voice 键
	_, items, _, err := cqparse.Parse(str, regDeps(nil, nil))
	if err != nil {
		t.Fatalf("Parse 失败: %v", err)
	}
	regFound(t, items, "url_records", "x.com/a.mp3")
	regNoKeys(t, items, "url_voice", "url_voices", "base64_voice", "local_voice", "unknown_voice")
}

// TestCQParseThreePathRemove_PendingOnly 对拍 remove 撤回码（Parse 只产出 pending）。
func TestCQParseThreePathRemove_PendingOnly(t *testing.T) {
	rec := &runActionRecorder{}
	str := regStrIn(`[CQ:remove,user_id=111,msg_id=555]`)
	seg := regSegIn([]map[string]interface{}{
		{"type": "remove", "data": map[string]interface{}{"user_id": "111", "msg_id": "555"}},
	}, regGroupID)
	mp := regMapIn(trssJSON(t, `{"type":"remove","data":{"user_id":111,"msg_id":555}}`), regGroupID)

	_, _, _ = assertThreePathEq(t, str, seg, mp, regDeps(nil, rec))

	strText, _, pend, err := cqparse.Parse(str, regDeps(nil, rec))
	if err != nil {
		t.Fatalf("Parse 失败: %v", err)
	}
	if strText != "" {
		t.Errorf("remove 码应移除: got %q", strText)
	}
	p := regPending(t, pend, "remove")
	if p["user_id"] != "111" || p["msg_id"] != "555" {
		t.Errorf("remove 参数: got %+v", p)
	}
	if rec.calls != 0 {
		t.Errorf("Parse 阶段不得执行动作: RunAction 被调用 %d 次", rec.calls)
	}
}

// ---------- 三模式行为矩阵（C1 场景：md+kb 相邻） ----------

// captureStdout 捕获 fn 期间写入 os.Stdout 的内容（mylog printConsole 走 fmt.Println）。
func captureStdout(t *testing.T, fn func()) string {
	t.Helper()
	old := os.Stdout
	r, w, err := os.Pipe()
	if err != nil {
		t.Fatalf("os.Pipe 失败: %v", err)
	}
	os.Stdout = w
	done := make(chan string, 1)
	go func() {
		var buf bytes.Buffer
		_, _ = io.Copy(&buf, r)
		done <- buf.String()
	}()
	fn()
	_ = w.Close()
	os.Stdout = old
	return <-done
}

// TestCQParseThreeModeMatrix_MarkdownKeyboardAdjacent 实测三模式行为矩阵。
// 输入为 §9 O-A（C1）相邻形态：markdown(JSON) + 正文 + keyboard(JSON)。
func TestCQParseThreeModeMatrix_MarkdownKeyboardAdjacent(t *testing.T) {
	params := callapi.ParamsContent{
		GroupID: "g0123456789abcdef0123456789abcde",
		UserID:  "u0123456789abcdef0123456789abcde",
		Message: `[CQ:markdown,data={"content":"hi"}]普通文本[CQ:keyboard,data={"id":"k1"}]`,
	}
	msg := callapi.ActionMessage{Action: "send_group_msg", Params: params}
	b64MD := base64.StdEncoding.EncodeToString([]byte(`{"content":"hi"}`))

	// legacy：parseMessageContent 在 config 未初始化时与生产默认一致走 legacy 分支
	legacyText, legacyItems, legacyPend := parseMessageContent(params, msg, nil, nil, nil)
	t.Logf("legacy 产物: text=%q items=%v pend=%v", legacyText, legacyItems, legacyPend)
	legacyTextBefore := legacyText
	legacyItemsBefore := map[string][]string{}
	for k, v := range legacyItems {
		legacyItemsBefore[k] = append([]string(nil), v...)
	}

	// new：同名分支函数直驱（与 message_parser.go:662 分支等价）
	newText, newItems, newPend := parseMessageContentCQParse(params, msg, nil, nil, nil)

	// shadow：同名分支函数直驱（与 message_parser.go:664-667 分支等价），捕获 diff 日志
	shadowLog := captureStdout(t, func() {
		runCQParseShadow(params, msg, nil, nil, nil, legacyText, legacyItems)
	})
	t.Logf("shadow 日志: %s", shadowLog)

	// --- new：正确拆分不吞噬 ---
	if newText != "普通文本" {
		t.Errorf("new 模式文本应正确拆分: got %q, want 普通文本", newText)
	}
	regFound(t, newItems, "markdown", b64MD)
	regFound(t, newItems, "keyboard", `{"id":"k1"}`)
	if len(newPend) != 0 {
		t.Errorf("md/kb 非动作码不应产 pending: %v", newPend)
	}

	// --- legacy：C1 仍在（贪心吞噬：正文与 keyboard 码被并入 markdown data 载荷） ---
	if legacyText == newText {
		t.Errorf("legacy 模式 C1 应仍在: legacy 与 new 文本意外一致(%q)", legacyText)
	}
	mdGot, ok := legacyItems["markdown"]
	if !ok || len(mdGot) != 1 {
		t.Fatalf("legacy markdown 产物现状: got %v", legacyItems["markdown"])
	}
	mdRaw, decErr := base64.StdEncoding.DecodeString(mdGot[0])
	if decErr != nil || !strings.Contains(string(mdRaw), "普通文本") || !strings.Contains(string(mdRaw), "[CQ:keyboard") {
		t.Errorf("legacy C1 应将正文与 keyboard 吞入 markdown 载荷: decoded=%q err=%v", string(mdRaw), decErr)
	}
	if _, ok := legacyItems["keyboard"]; ok {
		t.Errorf("legacy keyboard 产物应因 C1 缺失: got %v", legacyItems["keyboard"])
	}

	// --- shadow：diff 日志实证，且产物仍取 legacy（动作副作用只由 legacy 执行） ---
	if !strings.Contains(shadowLog, "[cqparse-shadow]") {
		t.Errorf("shadow 模式应产出 [cqparse-shadow] diff 日志: got %q", shadowLog)
	}
	if !strings.Contains(shadowLog, "文本差异") && !strings.Contains(shadowLog, "foundItems 差异") && !strings.Contains(shadowLog, "动作码差异") {
		t.Errorf("shadow diff 日志应指明差异维度: got %q", shadowLog)
	}
	// shadow 模式的对外产物 = legacy 产物（runCQParseShadow 只读不改写）：
	// 验证 legacy 文本与 foundItems 在 shadow 调用前后逐字节不变
	if legacyText != legacyTextBefore {
		t.Errorf("shadow 调用后 legacy 文本应保持原样: got %q, want %q", legacyText, legacyTextBefore)
	}
	if !reflect.DeepEqual(legacyItems, legacyItemsBefore) {
		t.Errorf("shadow 调用后 legacy foundItems 应保持原样: got %v, want %v", legacyItems, legacyItemsBefore)
	}
}

// TestCQParseModeDispatch_DefaultIsLegacy 钉死切换链默认值：config 未初始化时
// parseMessageContent 走 legacy 分支（第 1 项产物与 legacy 分支函数逐字节一致）。
func TestCQParseModeDispatch_DefaultIsLegacy(t *testing.T) {
	if got := config.GetCQParseMode(); got != "legacy" {
		t.Fatalf("config 未初始化时 GetCQParseMode 应回退 legacy: got %q", got)
	}
	params := callapi.ParamsContent{
		GroupID: "g0123456789abcdef0123456789abcde",
		UserID:  "u0123456789abcdef0123456789abcde",
		Message: "看这张图[CQ:image,file=https://x.com/p.png]好",
	}
	msg := callapi.ActionMessage{Action: "send_group_msg", Params: params}
	viaDispatch, itemsDispatch, _ := parseMessageContent(params, msg, nil, nil, nil)
	viaLegacy, itemsLegacy := parseMessageContentLegacy(params, msg, nil, nil, nil)
	if viaDispatch != viaLegacy || !reflect.DeepEqual(itemsDispatch, itemsLegacy) {
		t.Errorf("默认分发与 legacy 分支不一致: dispatch=(%q,%v) legacy=(%q,%v)",
			viaDispatch, itemsDispatch, viaLegacy, itemsLegacy)
	}
}

package handlers

// 本文件是 CQ 码统一解析架构重构（cqparse）的"特征回归网"第二部分：
// 对 handlers/cqparse 新 API（接口冻结见
// .git/opencode-team/.../cq-unified-parse/05-architect-cqparse-design.md §3）
// 按 §9 清单（O*/N*/I* 三系）写目标行为测试。
//
// 构建标签 cqparse_pending 已移除：cqparse 包落地并转正，本文件参与常规编译，
// 作为切换验收基线（状态保持类证明等价、修复类证明根治）。
//
// 断言标记约定：
//   - 「状态保持」：新旧行为一致（现状正确），迁移后必须原样通过；
//   - 「期望=迁移后行为」：断言的是审计确认的 bug 修复后的目标行为（C1/C3/M*/m*），
//     与当前 legacy 代码不符属预期，转正前由 shadow diff 验证。

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"strings"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/handlers/cqparse"
	"github.com/tencent-connect/botgo/dto"
)

// ---------- 假实现与辅助 ----------

// fakeGroupInfoFetcher 按设计 §3 Deps.GroupInfo（GroupInfoFetcher）构造的最小假实现。
// 方法签名对齐 get_group_info.go:51 的 apiv2.GroupInfo 同源取数（ctx, groupOpenID）；
// 实现落地时按其头部注释授权调整为本包 cqparse.GroupInfoData 投影
// （cqparse 零 botgo import 红线，经 Deps 接口注入）。
type fakeGroupInfoFetcher struct {
	calls  int
	lastID string
	info   *cqparse.GroupInfoData
	err    error
}

func (f *fakeGroupInfoFetcher) GroupInfo(ctx context.Context, groupOpenID string) (*cqparse.GroupInfoData, error) {
	f.calls++
	f.lastID = groupOpenID
	return f.info, f.err
}

// runActionRecorder 记录 Parse 是否违规执行了动作（Parse 必须只产出 Pending 不执行）
type runActionRecorder struct {
	calls    int
	actions  []string
	realIDs  []string
	eventIDs []string
}

func (r *runActionRecorder) run(p cqparse.PendingAction, eventID *string) cqparse.ExecOutcome {
	r.calls++
	r.actions = append(r.actions, p.Action)
	out := cqparse.ExecOutcome{RealGroupID: p.Params["group_id"]}
	if eventID != nil {
		r.eventIDs = append(r.eventIDs, *eventID)
	}
	return out
}

// regDeps 构造注入假实现的 Deps；AvatarURL 按 qq 生成可区分 URL
func regDeps(gi *fakeGroupInfoFetcher, ra *runActionRecorder) *cqparse.Deps {
	return &cqparse.Deps{
		GroupInfo: gi,
		AvatarURL: func(qq, groupID string, hasGroup bool) (string, error) {
			return "https://avatar.test/" + qq + "/" + groupID, nil
		},
		RunAction: ra.run,
	}
}

// regGroupID 测试群上下文：32 位原生 OpenID 形态（resolveGroupOpenID 直通）。
// 省略 group_id 的 group_info 路径统一经 resolveGroupOpenID 反查（Minor 修复：
// 去重键归一为 OpenID），虚拟 ID 形态在测试空 idmap 库中会反查失败走 fallback，
// 故群上下文统一用 32 位直通形态，与生产入站已转换 OpenID 的实际形态一致。
const regGroupID = "g0123456789abcdef0123456789abcde"

// regStrIn 构造群聊字符串输入
func regStrIn(s string) cqparse.Input {
	return cqparse.Input{Kind: cqparse.InputString, String: s, GroupID: regGroupID, HasGroup: true, UserID: "user-1"}
}

// regStrPrivate 构造私聊字符串输入
func regStrPrivate(s string) cqparse.Input {
	return cqparse.Input{Kind: cqparse.InputString, String: s, HasGroup: false, UserID: "user-1"}
}

// regSegIn 构造消息段数组输入（groupID 为空串时视为私聊）
func regSegIn(segs []map[string]interface{}, groupID string) cqparse.Input {
	return cqparse.Input{Kind: cqparse.InputSegments, Segments: segs, GroupID: groupID, HasGroup: groupID != "", UserID: "user-1"}
}

// regMapIn 构造 TRSS map 输入（按 §3 包装为单元素段列表）
func regMapIn(m map[string]interface{}, groupID string) cqparse.Input {
	return cqparse.Input{Kind: cqparse.InputMap, Segments: []map[string]interface{}{m}, GroupID: groupID, HasGroup: groupID != "", UserID: "user-1"}
}

// regParse 必须成功解析，失败即 Fatal
func regParse(t *testing.T, in cqparse.Input, d *cqparse.Deps) (string, map[string][]string, []cqparse.PendingAction) {
	t.Helper()
	text, items, pendings, err := cqparse.Parse(in, d)
	if err != nil {
		t.Fatalf("cqparse.Parse 失败: %v", err)
	}
	return text, items, pendings
}

// regFound 断言 foundItems[key] 与期望逐项一致（含顺序）
func regFound(t *testing.T, items map[string][]string, key string, want ...string) {
	t.Helper()
	got, ok := items[key]
	if !ok {
		t.Fatalf("缺少期望 key: %s, keys=%v", key, items)
	}
	if len(got) != len(want) {
		t.Fatalf("key %s 长度: got %d, want %d, items=%v", key, len(got), len(want), got)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Errorf("key %s[%d]: got %q, want %q", key, i, got[i], want[i])
		}
	}
}

// regNoKeys 断言这些 key 不存在
func regNoKeys(t *testing.T, items map[string][]string, keys ...string) {
	t.Helper()
	for _, key := range keys {
		if _, ok := items[key]; ok {
			t.Errorf("不应出现 key: %s", key)
		}
	}
}

// regNoRaw 断言原文码不出现在结果文本中
func regNoRaw(t *testing.T, text, raw string) {
	t.Helper()
	if strings.Contains(text, raw) {
		t.Errorf("文本不应含码原文 %q: got %q", raw, text)
	}
}

// regPending 断言恰好一条指定 action 的 pending 并返回其 Params
func regPending(t *testing.T, pendings []cqparse.PendingAction, action string) map[string]string {
	t.Helper()
	if len(pendings) != 1 {
		t.Fatalf("pending 数量: got %d, want 1, pendings=%+v", len(pendings), pendings)
	}
	if pendings[0].Action != action {
		t.Fatalf("pending action: got %q, want %q", pendings[0].Action, action)
	}
	return pendings[0].Params
}

// regJSONMap 解析 JSON 字符串为 map 断言用
func regJSONMap(t *testing.T, s string) map[string]string {
	t.Helper()
	var m map[string]string
	if err := json.Unmarshal([]byte(s), &m); err != nil {
		t.Fatalf("非法 JSON: %q, err=%v", s, err)
	}
	return m
}

// ---------- 字符串路径（tokenizer/escape，§9 O/N-A 系） ----------

// TestCQParseRegress_OA_MarkdownKeyboardAdjacent O-A（C1 修复）— 期望=迁移后行为
func TestCQParseRegress_OA_MarkdownKeyboardAdjacent(t *testing.T) {
	in := regStrIn(`[CQ:markdown,data={"content":"hi"}]普通文本[CQ:keyboard,data={"id":"k1"}]`)
	text, items, _ := regParse(t, in, regDeps(nil, nil))

	wantMD := base64.StdEncoding.EncodeToString([]byte(`{"content":"hi"}`))
	if text != "普通文本" {
		t.Errorf("文本: got %q, want 普通文本", text)
	}
	regFound(t, items, "markdown", wantMD)
	regFound(t, items, "keyboard", `{"id":"k1"}`)
}

// TestCQParseRegress_OA2_JSONWithBracket O-A2（括号配平）— 期望=迁移后行为
func TestCQParseRegress_OA2_JSONWithBracket(t *testing.T) {
	in := regStrIn(`[CQ:markdown,data={"a":[1,2]}]`)
	text, items, _ := regParse(t, in, regDeps(nil, nil))

	wantMD := base64.StdEncoding.EncodeToString([]byte(`{"a":[1,2]}`))
	if text != "" {
		t.Errorf("文本: got %q, want 空", text)
	}
	regFound(t, items, "markdown", wantMD)
}

// TestCQParseRegress_NA3_UnclosedCode N-A3（状态保持：未闭合码整段留正文）
func TestCQParseRegress_NA3_UnclosedCode(t *testing.T) {
	in := regStrIn(`[CQ:markdown,data={"a":`)
	text, items, _ := regParse(t, in, regDeps(nil, nil))

	if text != `[CQ:markdown,data={"a":` {
		t.Errorf("未闭合码应整段留正文: got %q", text)
	}
	regNoKeys(t, items, "markdown")
}

// TestCQParseRegress_OC_EscapeDecode O-C1/C2/C3
func TestCQParseRegress_OC_EscapeDecode(t *testing.T) {
	t.Run("O-C1状态保持:&#44;/&amp;/&#93;解码", func(t *testing.T) {
		in := regStrIn(`[CQ:set_group,action=add_request,group_id=1,user_id=2,flag=f,reason=a&#44;b&amp;c&#93;d]`)
		_, _, pendings := regParse(t, in, regDeps(nil, nil))
		params := regPending(t, pendings, "set_group")
		if params["reason"] != "a,b&c]d" {
			t.Errorf("reason 解码: got %q, want a,b&c]d", params["reason"])
		}
	})

	t.Run("O-C2迁移后行为:&#91;解码(m1)", func(t *testing.T) {
		in := regStrIn(`[CQ:set_group,action=add_request,group_id=1,user_id=2,flag=f,reason=x&#91;y]`)
		_, _, pendings := regParse(t, in, regDeps(nil, nil))
		params := regPending(t, pendings, "set_group")
		if params["reason"] != "x[y" {
			t.Errorf("reason 解码: got %q, want x[y", params["reason"])
		}
	})

	t.Run("O-C3状态保持:重复key后者覆盖", func(t *testing.T) {
		in := regStrIn(`[CQ:set_group,action=ban,group_id=1,user_id=1,user_id=2]`)
		_, _, pendings := regParse(t, in, regDeps(nil, nil))
		params := regPending(t, pendings, "set_group")
		if params["user_id"] != "2" {
			t.Errorf("重复 key 应后者覆盖: got %q, want 2", params["user_id"])
		}
	})
}

// TestCQParseRegress_NA4_BatchUserIDs N-A4（C3 修复）— 期望=迁移后行为
func TestCQParseRegress_NA4_BatchUserIDs(t *testing.T) {
	in := regStrIn(`[CQ:set_group,action=kick,group_id=1,user_ids=1,2,3]`)
	text, _, pendings := regParse(t, in, regDeps(nil, nil))

	params := regPending(t, pendings, "set_group")
	if params["user_ids"] != "1,2,3" {
		t.Errorf("尾随无=段应并入前值: got %q, want 1,2,3", params["user_ids"])
	}
	regNoRaw(t, text, "[CQ:set_group")
}

// TestCQParseRegress_NA5_CardboardNotCard N-A5（m3 修复）— 期望=迁移后行为
func TestCQParseRegress_NA5_CardboardNotCard(t *testing.T) {
	in := regStrIn(`[CQ:cardboard,xx=1]`)
	text, items, _ := regParse(t, in, regDeps(nil, nil))

	if text != `[CQ:cardboard,xx=1]` {
		t.Errorf("cardboard 应原样保留: got %q", text)
	}
	regNoKeys(t, items, "card")
}

// TestCQParseRegress_NA67_StreamSyntax N-A6（m2 修复，= 语法，期望=迁移后行为）
// 与 N-A7（冒号语法，状态保持）
func TestCQParseRegress_NA67_StreamSyntax(t *testing.T) {
	t.Run("N-A6迁移后行为:等号语法识别", func(t *testing.T) {
		in := regStrIn(`[CQ:stream,type=start,qq=123]`)
		text, items, _ := regParse(t, in, regDeps(nil, nil))
		if text != "" {
			t.Errorf("stream 码应移除: got %q", text)
		}
		m := regJSONMap(t, items["stream"][0])
		if m["type"] != "start" || m["qq"] != "123" {
			t.Errorf("stream 参数: got %v", m)
		}
	})

	t.Run("N-A7状态保持:冒号语法仍识别", func(t *testing.T) {
		in := regStrIn(`[CQ:stream,type:start,qq:123]`)
		_, items, _ := regParse(t, in, regDeps(nil, nil))
		m := regJSONMap(t, items["stream"][0])
		if m["type"] != "start" || m["qq"] != "123" {
			t.Errorf("stream 参数: got %v", m)
		}
	})
}

// TestCQParseRegress_NA8_AtVariants N-A8（状态保持：at 变体原样保留）
func TestCQParseRegress_NA8_AtVariants(t *testing.T) {
	for _, raw := range []string{"[CQ:at]", "[CQ:at,qq=]", "[cq:at,qq=123]"} {
		t.Run(raw, func(t *testing.T) {
			in := regStrIn(raw + "正文")
			text, items, _ := regParse(t, in, regDeps(nil, nil))
			if text != raw+"正文" {
				t.Errorf("at 变体应原样保留: got %q, want %q", text, raw+"正文")
			}
			if len(items) != 0 {
				t.Errorf("at 变体不应产 foundItems: %v", items)
			}
		})
	}
}

// TestCQParseRegress_NA910_MediaExtraParams N-A9/N-A10（M2 修复）— 期望=迁移后行为
func TestCQParseRegress_NA910_MediaExtraParams(t *testing.T) {
	t.Run("N-A9附加参数不污染URL", func(t *testing.T) {
		in := regStrIn(`[CQ:image,file=https://x.com/a.png,subType=0,url=https://y.com/b.png]`)
		text, items, _ := regParse(t, in, regDeps(nil, nil))
		if text != "" {
			t.Errorf("媒体码应移除: got %q", text)
		}
		regFound(t, items, "url_images", "x.com/a.png")
		regNoKeys(t, items, "unknown_image")
	})

	t.Run("N-A10 go-cqhttp风格文件名入unknown_image", func(t *testing.T) {
		in := regStrIn(`[CQ:image,file=ABC.image,subType=0,url=https://x.com/p.png]`)
		text, items, _ := regParse(t, in, regDeps(nil, nil))
		if text != "" {
			t.Errorf("媒体码应移除不泄漏: got %q", text)
		}
		regFound(t, items, "unknown_image", "ABC.image")
	})
}

// TestCQParseRegress_NA1112_FileValues N-A11/N-A12（m4 修复）— 期望=迁移后行为
func TestCQParseRegress_NA1112_FileValues(t *testing.T) {
	t.Run("N-A11值含逗号不截断", func(t *testing.T) {
		in := regStrIn(`[CQ:file,file=https://x.com/a,b.txt]`)
		text, items, _ := regParse(t, in, regDeps(nil, nil))
		if text != "" {
			t.Errorf("file 码应移除: got %q", text)
		}
		regFound(t, items, "url_files", "x.com/a,b.txt")
	})

	t.Run("N-A12未知前缀入unknown_file不泄漏", func(t *testing.T) {
		in := regStrIn(`[CQ:file,file=weird://x]`)
		text, items, _ := regParse(t, in, regDeps(nil, nil))
		if text != "" {
			t.Errorf("file 码应移除不泄漏: got %q", text)
		}
		regFound(t, items, "unknown_file", "weird://x")
	})
}

// TestCQParseRegress_OE_SegmentStringEquivalence O-E/N-E3（状态保持：三路批量等价）
func TestCQParseRegress_OE_SegmentStringEquivalence(t *testing.T) {
	d := regDeps(nil, nil)

	seg := regSegIn([]map[string]interface{}{
		{"type": "set_group", "data": map[string]interface{}{
			"action":   "kick",
			"group_id": "1",
			"user_ids": []interface{}{"111", "222", "333"},
		}},
	}, "1")
	str := regStrIn(`[CQ:set_group,action=kick,group_id=1,user_ids=111&#44;222&#44;333]`)

	_, _, segPendings := regParse(t, seg, d)
	_, _, strPendings := regParse(t, str, d)

	segParams := regPending(t, segPendings, "set_group")
	strParams := regPending(t, strPendings, "set_group")
	if segParams["user_ids"] != "111,222,333" || strParams["user_ids"] != "111,222,333" {
		t.Errorf("三路批量应等价: seg=%q str=%q", segParams["user_ids"], strParams["user_ids"])
	}
	if segParams["action"] != strParams["action"] || segParams["group_id"] != strParams["group_id"] {
		t.Errorf("段/字符串路径 Params 不一致:\n  seg=%v\n  str=%v", segParams, strParams)
	}
}

// TestCQParseRegress_OM4_WholeBanInvalidEnable O-M4（M4 修复）— 期望=迁移后行为
func TestCQParseRegress_OM4_WholeBanInvalidEnable(t *testing.T) {
	in := regStrIn(`[CQ:set_group,action=whole_ban,group_id=1,enable=abc]`)
	text, _, pendings := regParse(t, in, regDeps(nil, nil))

	regNoRaw(t, text, "[CQ:set_group")
	params := regPending(t, pendings, "set_group")
	if params["enable"] != "abc" {
		t.Errorf("enable 参数应完整上交执行器: got %q", params["enable"])
	}
	// 执行器契约：enable 非法 → 日志 + 跳过执行，码不回填正文（M4）
}

// TestCQParseRegress_OI_AlphanumericReply O-I（M6 修复）— 期望=迁移后行为
func TestCQParseRegress_OI_AlphanumericReply(t *testing.T) {
	in := regStrIn(`[CQ:reply,id=BAC3N5VPRKPRQ1GJ]正文`)
	text, items, _ := regParse(t, in, regDeps(nil, nil))

	if text != "正文" {
		t.Errorf("reply 码应移除: got %q, want 正文", text)
	}
	regFound(t, items, "reply_msg_id", "BAC3N5VPRKPRQ1GJ")
}

// TestCQParseRegress_NA1314_RemovePending N-A13/N-A14（M3）— 期望=迁移后行为
func TestCQParseRegress_NA1314_RemovePending(t *testing.T) {
	t.Run("N-A13码必移除且pending完整上交", func(t *testing.T) {
		in := regStrIn(`[CQ:remove,user_id=42,msg_id=7]`)
		text, _, pendings := regParse(t, in, regDeps(nil, nil))

		regNoRaw(t, text, "[CQ:remove")
		params := regPending(t, pendings, "remove")
		if params["user_id"] != "42" || params["msg_id"] != "7" {
			t.Errorf("remove 参数: got %v", params)
		}
		// 执行器契约：撤回成败不影响码移除（M3：失败路径 return match 泄漏已废除）
	})

	t.Run("N-A14缺msg_id时Params无该key触发自动查最新分支", func(t *testing.T) {
		in := regStrIn(`[CQ:remove,user_id=42]`)
		_, _, pendings := regParse(t, in, regDeps(nil, nil))

		params := regPending(t, pendings, "remove")
		if _, ok := params["msg_id"]; ok {
			t.Errorf("缺省 msg_id 不应产生空值 key（执行器据 key 缺失走自动查最新分支）: %v", params)
		}
		if params["user_id"] != "42" {
			t.Errorf("user_id: got %q", params["user_id"])
		}
	})
}

// TestCQParseRegress_NA15_MarkdownBase64 N-A15（状态保持）
func TestCQParseRegress_NA15_MarkdownBase64(t *testing.T) {
	b64MD := base64.StdEncoding.EncodeToString([]byte(`{"content":"hi"}`))
	in := regStrIn("[CQ:markdown,data=base64://" + b64MD + "]正文")
	text, items, _ := regParse(t, in, regDeps(nil, nil))

	if text != "正文" {
		t.Errorf("文本: got %q, want 正文", text)
	}
	regFound(t, items, "markdown", b64MD)
}

// TestCQParseRegress_NA16_ActiveKeysAndMDOrder N-A16
func TestCQParseRegress_NA16_ActiveKeysAndMDOrder(t *testing.T) {
	t.Run("N-A16a状态保持:active三键归位", func(t *testing.T) {
		in := regStrIn(`[CQ:active,type=push,sub_type=1]主动`)
		text, items, _ := regParse(t, in, regDeps(nil, nil))
		if text != "主动" {
			t.Errorf("文本: got %q", text)
		}
		regFound(t, items, "active", "true")
		regFound(t, items, "active_type", "push")
		regFound(t, items, "active_sub_type", "1")
	})

	t.Run("N-A16b迁移后行为:md内嵌media串不被误提取(m8)", func(t *testing.T) {
		in := regStrIn(`[CQ:markdown,data={"content":"[CQ:image,file=https://x.com/a.png]"}]正文`)
		text, items, _ := regParse(t, in, regDeps(nil, nil))

		wantMD := base64.StdEncoding.EncodeToString([]byte(`{"content":"[CQ:image,file=https://x.com/a.png]"}`))
		if text != "正文" {
			t.Errorf("文本: got %q", text)
		}
		regFound(t, items, "markdown", wantMD)
		regNoKeys(t, items, "url_images")
	})
}

// ---------- 三输入等价矩阵（§9 N-E 系） ----------

// TestCQParseRegress_NE1_AtNumericSegment N-E1（M8 修复）— 期望=迁移后行为
func TestCQParseRegress_NE1_AtNumericSegment(t *testing.T) {
	in := regSegIn([]map[string]interface{}{
		{"type": "at", "data": map[string]interface{}{"qq": float64(123)}},
	}, "1")
	text, items, _ := regParse(t, in, regDeps(nil, nil))

	if text != "[CQ:at,qq=123]" {
		t.Errorf("数字 qq 应 coerceString 渲染: got %q, want [CQ:at,qq=123]", text)
	}
	if len(items) != 0 {
		t.Errorf("at 段不应产 foundItems: %v", items)
	}
}

// TestCQParseRegress_NE2_MemberNumericParams N-E2（M8 修复）— 期望=迁移后行为
func TestCQParseRegress_NE2_MemberNumericParams(t *testing.T) {
	in := regSegIn([]map[string]interface{}{
		{"type": "member", "data": map[string]interface{}{
			"type": "add", "group_id": float64(999), "user_id": float64(42),
		}},
	}, "1")
	_, _, pendings := regParse(t, in, regDeps(nil, nil))

	params := regPending(t, pendings, "member")
	if params["type"] != "add" || params["group_id"] != "999" || params["user_id"] != "42" {
		t.Errorf("数字 group_id/user_id 应正确 coerce: got %v", params)
	}
}

// TestCQParseRegress_NE4_NoGroupEquivalence N-E4（M7 修复）— 期望=迁移后行为
// GroupID 缺省（零值）与显式空串两种 Input 必须产生完全相同的解析结果
// （均视为私聊）；JSON 层 nil/"" → HasGroup=false 的归一由接入层钉死。
func TestCQParseRegress_NE4_NoGroupEquivalence(t *testing.T) {
	segments := []map[string]interface{}{
		{"type": "text", "data": map[string]interface{}{"text": "私聊消息"}},
		{"type": "at", "data": map[string]interface{}{"qq": "123"}},
	}
	inZero := cqparse.Input{Kind: cqparse.InputSegments, Segments: segments, HasGroup: false}
	inEmpty := cqparse.Input{Kind: cqparse.InputSegments, Segments: segments, GroupID: "", HasGroup: false}

	d := regDeps(nil, nil)
	text1, items1, pendings1 := regParse(t, inZero, d)
	text2, items2, pendings2 := regParse(t, inEmpty, d)

	if text1 != text2 || len(pendings1) != len(pendings2) || len(items1) != len(items2) {
		t.Errorf("GroupID 零值与空串结果应一致:\n  zero:  %q %v %v\n  empty: %q %v %v",
			text1, items1, pendings1, text2, items2, pendings2)
	}
}

// TestCQParseRegress_NE5_TRSSInvalidMarkdown N-E5（m6 修复）— 期望=迁移后行为
func TestCQParseRegress_NE5_TRSSInvalidMarkdown(t *testing.T) {
	in := regMapIn(map[string]interface{}{
		"type": "markdown",
		"data": map[string]interface{}{"data": "not-a-json"},
	}, "1")
	text, items, _ := regParse(t, in, regDeps(nil, nil))

	if text != "" {
		t.Errorf("非法 JSON markdown 应跳过: got %q", text)
	}
	regNoKeys(t, items, "markdown")
}

// TestCQParseRegress_NE6_WakeupNumericSegment N-E6（coerceString）— 期望=迁移后行为
func TestCQParseRegress_NE6_WakeupNumericSegment(t *testing.T) {
	in := regSegIn([]map[string]interface{}{
		{"type": "wakeup", "data": map[string]interface{}{"userid": float64(12345)}},
	}, "")
	text, items, _ := regParse(t, in, regDeps(nil, nil))

	if text != "" {
		t.Errorf("wakeup 码不应留痕: got %q", text)
	}
	regFound(t, items, "wakeup", "12345")
}

// ---------- avatar / group_info（§9 O-G / N-G 系） ----------

// TestCQParseRegress_OG_TwoAvatarsDistinct O-G（M5 修复）— 期望=迁移后行为
func TestCQParseRegress_OG_TwoAvatarsDistinct(t *testing.T) {
	in := regStrIn(`[CQ:avatar,qq=111]和[CQ:avatar,qq=222]`)
	text, items, _ := regParse(t, in, regDeps(nil, nil))

	if text != "和" {
		t.Errorf("avatar 码应移除且产物不回写正文: got %q, want 和", text)
	}
	if strings.Contains(text, "[CQ:image") {
		t.Errorf("avatar 产物不应以 CQ 码形式回写正文: %q", text)
	}
	regFound(t, items, "url_images",
		"https://avatar.test/111/"+regGroupID,
		"https://avatar.test/222/"+regGroupID,
	)
}

// TestCQParseRegress_NG_GroupInfo N-G1..N-G8（group_info 第一个内容扩展 handler）
// 取数经 Deps.GroupInfo 注入，计数验证 N-G7 的"同群去重一次取数"。
func TestCQParseRegress_NG_GroupInfo(t *testing.T) {
	newGI := func(err error) *fakeGroupInfoFetcher {
		return &fakeGroupInfoFetcher{
			info: &cqparse.GroupInfoData{Name: "测试群", Memo: "群公告", MemberCount: 42},
			err:  err,
		}
	}

	t.Run("N-G1省略group_id回退当前群field=name", func(t *testing.T) {
		gi := newGI(nil)
		in := regStrIn(`前[CQ:group_info,field=name]后`)
		text, items, _ := regParse(t, in, regDeps(gi, nil))

		if text != "前测试群后" {
			t.Errorf("文本应替换为群名: got %q", text)
		}
		if gi.calls != 1 {
			t.Errorf("GroupInfo 调用次数: got %d, want 1", gi.calls)
		}
		regNoKeys(t, items, "url_images")
	})

	t.Run("N-G2 field=member_count", func(t *testing.T) {
		gi := newGI(nil)
		in := regStrIn(`[CQ:group_info,field=member_count]`)
		text, _, _ := regParse(t, in, regDeps(gi, nil))
		if text != "42" {
			t.Errorf("member_count 应替换为数字: got %q, want 42", text)
		}
	})

	t.Run("N-G3 field=all三字段JSON", func(t *testing.T) {
		gi := newGI(nil)
		in := regStrIn(`[CQ:group_info,field=all]`)
		text, _, _ := regParse(t, in, regDeps(gi, nil))

		m := regJSONMap(t, text) // 合法 JSON 且含三字段值（键名由实现定义）
		found := 0
		for _, v := range m {
			if v == "测试群" || v == "群公告" || v == "42" {
				found++
			}
		}
		if found < 3 {
			t.Errorf("field=all 应含三字段值: got %v", m)
		}
	})

	t.Run("N-G4 field=bogus原文保留", func(t *testing.T) {
		gi := newGI(nil)
		in := regStrIn(`[CQ:group_info,field=bogus]`)
		text, _, _ := regParse(t, in, regDeps(gi, nil))
		if text != `[CQ:group_info,field=bogus]` {
			t.Errorf("参数错误应保留原文: got %q", text)
		}
	})

	t.Run("N-G5 32位OpenID直通", func(t *testing.T) {
		gi := newGI(nil)
		openID32 := "g0123456789abcdef0123456789abcde"
		if len(openID32) != 32 {
			t.Fatalf("测试前提: 32 位 OpenID 构造错误")
		}
		in := regStrIn(`[CQ:group_info,field=name,group_id=` + openID32 + `]`)
		text, _, _ := regParse(t, in, regDeps(gi, nil))

		if text != "测试群" {
			t.Errorf("32 位 OpenID 应直通取数: got %q", text)
		}
		if gi.lastID != openID32 {
			t.Errorf("fetcher 应收到原始 group_id: got %q, want %q", gi.lastID, openID32)
		}
	})

	t.Run("N-G5b虚拟ID不panic且码被替换", func(t *testing.T) {
		// 虚拟 ID 依赖 idmap 反查（cqparse 允许依赖 idmap）；空库反查失败时
		// 按 N-G6 失败分级落 fallback——此处仅钉不变量：不 panic、码不出现在正文
		gi := newGI(nil)
		in := regStrIn(`[CQ:group_info,field=name,group_id=999]`)
		text, _, _ := regParse(t, in, regDeps(gi, nil))
		regNoRaw(t, text, "[CQ:group_info")
	})

	t.Run("N-G6 API失败fallback默认空串", func(t *testing.T) {
		gi := newGI(errors.New("官方错误码 11253"))
		in := regStrIn(`[CQ:group_info,field=name]`)
		text, _, _ := regParse(t, in, regDeps(gi, nil))

		if text != "" {
			t.Errorf("失败应替换为默认 fallback 空串: got %q", text)
		}
		if gi.calls != 1 {
			t.Errorf("GroupInfo 调用次数: got %d, want 1", gi.calls)
		}
	})

	t.Run("N-G7同群三码一次取数", func(t *testing.T) {
		gi := newGI(nil)
		in := regStrIn(`a[CQ:group_info,field=name]b[CQ:group_info,field=member_count]c[CQ:group_info,field=name]`)
		text, _, _ := regParse(t, in, regDeps(gi, nil))

		if text != "a测试群b42c测试群" {
			t.Errorf("三码替换: got %q, want a测试群b42c测试群", text)
		}
		if gi.calls != 1 {
			t.Errorf("同群多码应合并为一次 GroupInfo 调用(30 QPM 保护): got %d, want 1", gi.calls)
		}
	})

	t.Run("N-G8 fallback含转义逗号正确解码", func(t *testing.T) {
		gi := newGI(errors.New("官方错误"))
		in := regStrIn(`[CQ:group_info,field=name,fallback=暂无&#44;群名]`)
		text, _, _ := regParse(t, in, regDeps(gi, nil))

		if text != "暂无,群名" {
			t.Errorf("fallback 应解码转义后使用: got %q, want 暂无,群名", text)
		}
	})
}

// ---------- 终审修复轮（2026-09-05）：C-fix voice 键归一 / M-fix last-wins / Minor 裸码 ----------

// TestCQParseRegress_VoiceKeyNormalizedRecord C-fix：[CQ:voice] 三输入形态
// （字符串 CQ 码 / 消息段数组 / TRSS map）统一产出 *_record 键——与 legacy
// case "voice","record" 及全部下游消费者（send_group_msg/send_private_msg）对齐，
// 不再产出 url_voice/base64_voice 等错位键导致语音静默丢失。
func TestCQParseRegress_VoiceKeyNormalizedRecord(t *testing.T) {
	t.Run("字符串路径https voice→url_records", func(t *testing.T) {
		in := regStrIn(`[CQ:voice,file=https://x.com/a.mp3]语音`)
		text, items, _ := regParse(t, in, regDeps(nil, nil))
		if text != "语音" {
			t.Errorf("voice 码应移除: got %q", text)
		}
		regFound(t, items, "url_records", "x.com/a.mp3")
		regNoKeys(t, items, "url_voice", "url_voices", "base64_voice", "local_voice", "unknown_voice")
	})

	t.Run("段数组路径base64 voice→base64_record", func(t *testing.T) {
		in := regSegIn([]map[string]interface{}{
			{"type": "voice", "data": map[string]interface{}{"file": "base64://dm9pY2U="}},
		}, regGroupID)
		text, items, _ := regParse(t, in, regDeps(nil, nil))
		if text != "" {
			t.Errorf("voice 段应移除: got %q", text)
		}
		regFound(t, items, "base64_record", "dm9pY2U=")
		regNoKeys(t, items, "base64_voice")
	})

	t.Run("TRSS路径http voice→url_record", func(t *testing.T) {
		in := regMapIn(trssJSON(t, `{"type":"voice","data":{"file":"http://x.com/b.mp3"}}`), regGroupID)
		text, items, _ := regParse(t, in, regDeps(nil, nil))
		if text != "" {
			t.Errorf("voice 段应移除: got %q", text)
		}
		regFound(t, items, "url_record", "x.com/b.mp3")
		regNoKeys(t, items, "url_voice")
	})
}

// TestCQParseRegress_MemberLastWinsRealGroupID M-fix：多条 [CQ:member] 码的
// realGroupID 取最后一个非空值（last-wins，对齐 legacy ProcessOutboundCQCodes
// 逐码覆写语义），不再 first-wins 静默改变跨群路由目标。
func TestCQParseRegress_MemberLastWinsRealGroupID(t *testing.T) {
	in := regStrIn(`[CQ:member,type=add,group_id=111,user_id=42][CQ:member,type=add,group_id=222,user_id=42]`)
	text, _, pendings := regParse(t, in, regDeps(nil, nil))
	if text != "" {
		t.Errorf("member 码应移除: got %q", text)
	}
	if len(pendings) != 2 {
		t.Fatalf("应有两条 member pending: got %d", len(pendings))
	}

	// 执行产物 → pickLastRealGroupID last-wins（RunAction 用透传桩，聚焦路由语义）
	outs := cqparse.ExecutePending(pendings, &cqparse.Deps{
		RunAction: func(p cqparse.PendingAction, _ *string) cqparse.ExecOutcome {
			return cqparse.ExecOutcome{RealGroupID: p.Params["group_id"]}
		},
	}, nil)
	if len(outs) != 2 || outs[0].RealGroupID != "111" || outs[1].RealGroupID != "222" {
		t.Fatalf("执行产物应保留顺序与值: %+v", outs)
	}
	if got := pickLastRealGroupID(outs); got != "222" {
		t.Errorf("多 member 码 realGroupID 应 last-wins: got %q, want 222", got)
	}
}

// TestCQParseRegress_BareMediaCodesKept Minor：裸媒体码/裸动作码（无任何有效
// 参数）与 legacy 字面保留语义一致——不静默移除、不产 foundItems、不产 pending。
func TestCQParseRegress_BareMediaCodesKept(t *testing.T) {
	for _, raw := range []string{
		`[CQ:image]`, `[CQ:record]`, `[CQ:voice]`, `[CQ:video]`, `[CQ:file]`,
		`[CQ:image,file=]`, `[CQ:member]`,
	} {
		t.Run(raw, func(t *testing.T) {
			in := regStrIn(raw + "正文")
			text, items, pendings := regParse(t, in, regDeps(nil, nil))
			if text != raw+"正文" {
				t.Errorf("裸码应保留字面: got %q, want %q", text, raw+"正文")
			}
			if len(items) != 0 {
				t.Errorf("裸码不应产 foundItems: %v", items)
			}
			if len(pendings) != 0 {
				t.Errorf("裸码不应产 pending: %v", pendings)
			}
		})
	}
}

// TestCQParseRegress_NG_GroupInfoDedupKeyNormalized Minor：省略 group_id 与
// 显式 group_id（同一群）两种写法共享同一去重键（OpenID），合并为一次取数。
func TestCQParseRegress_NG_GroupInfoDedupKeyNormalized(t *testing.T) {
	gi := &fakeGroupInfoFetcher{info: &cqparse.GroupInfoData{Name: "测试群", Memo: "群公告", MemberCount: 42}}
	// regGroupID（32 位 OpenID）为当前会话群；显式再指定同一群
	in := regStrIn(`[CQ:group_info,field=name][CQ:group_info,field=member_count,group_id=` + regGroupID + `]`)
	text, _, _ := regParse(t, in, regDeps(gi, nil))

	if text != "测试群42" {
		t.Errorf("两码应依次展开: got %q", text)
	}
	if gi.calls != 1 {
		t.Errorf("省略/显式同群两写法应合并为一次取数: got %d", gi.calls)
	}
	if gi.lastID != regGroupID {
		t.Errorf("去重键应为反查后的 OpenID: got %q, want %q", gi.lastID, regGroupID)
	}
}

// ---------- 用户嵌套 data.data 段形态（2026-09 线上案例修复） ----------

// TestCQParseRegress_NestedDataWrapper_Markdown md-only 嵌套 data.data 双层 map：
// legacy 段路径取 segment["data"]["data"] → marshal+base64；cqparse normalize 层
// 对齐解包后 payload 逐字节一致、md.Content 正确。修复前 new 模式
// coerceString(map)="" → resolveMarkdown 段路径静默丢弃 → :513 门不开 → 空消息。
func TestCQParseRegress_NestedDataWrapper_Markdown(t *testing.T) {
	content := "### AT 检测结果\n> ❌ 消息未 @ 机器人"
	in := regSegIn([]map[string]interface{}{
		{"type": "markdown", "data": map[string]interface{}{
			"data": map[string]interface{}{
				"markdown": map[string]interface{}{"content": content},
			},
		}},
	}, regGroupID)
	text, items, _ := regParse(t, in, regDeps(nil, nil))

	if text != "" {
		t.Errorf("md 段不应留痕: got %q", text)
	}
	// legacy 同款 payload：b64(marshal(map))，json.Marshal 键序确定 → 逐字节可对拍
	wantPayload, err := json.Marshal(map[string]interface{}{
		"markdown": map[string]interface{}{"content": content},
	})
	if err != nil {
		t.Fatalf("构造期望 payload 失败: %v", err)
	}
	regFound(t, items, "markdown", base64.StdEncoding.EncodeToString(wantPayload))

	md, _ := parseMarkdownFromMessage(items["markdown"][0])
	if md == nil || md.Content != content {
		t.Errorf("md.Content: got %+v, want %q", md, content)
	}
}

// TestCQParseRegress_NestedDataWrapper_Keyboard keyboard 嵌套 data.data 双层 map：
// legacy 段路径取 segment["data"]["data"] → marshal 原样 JSON（不 base64），
// cqparse 对齐后 payload 一致、kb.ID 正确。
func TestCQParseRegress_NestedDataWrapper_Keyboard(t *testing.T) {
	in := regSegIn([]map[string]interface{}{
		{"type": "keyboard", "data": map[string]interface{}{
			"data": map[string]interface{}{"id": "tpl_1"},
		}},
	}, regGroupID)
	text, items, _ := regParse(t, in, regDeps(nil, nil))

	if text != "" {
		t.Errorf("kb 段不应留痕: got %q", text)
	}
	regFound(t, items, "keyboard", `{"id":"tpl_1"}`)

	kb, err := parseKeyboardData([]byte(items["keyboard"][0]))
	if err != nil || kb == nil || kb.ID != "tpl_1" {
		t.Errorf("kb.ID: kb=%+v err=%v, want id=tpl_1", kb, err)
	}
}

// TestCQParseRegress_NestedDataWrapper_InnerStringAndBase64 非回归钉死：
// 内层为 JSON 字符串 / base64:// 前缀时既有语义不变（与 legacy 字符串内层分支一致）。
func TestCQParseRegress_NestedDataWrapper_InnerStringAndBase64(t *testing.T) {
	t.Run("markdown内层JSON字符串", func(t *testing.T) {
		in := regSegIn([]map[string]interface{}{
			{"type": "markdown", "data": map[string]interface{}{
				"data": `{"markdown":{"content":"hi"}}`,
			}},
		}, regGroupID)
		text, items, _ := regParse(t, in, regDeps(nil, nil))
		if text != "" {
			t.Errorf("md 段不应留痕: got %q", text)
		}
		regFound(t, items, "markdown", base64.StdEncoding.EncodeToString([]byte(`{"markdown":{"content":"hi"}}`)))
	})

	t.Run("markdown内层base64前缀", func(t *testing.T) {
		b64MD := base64.StdEncoding.EncodeToString([]byte(`{"content":"hi"}`))
		in := regSegIn([]map[string]interface{}{
			{"type": "markdown", "data": map[string]interface{}{
				"data": "base64://" + b64MD,
			}},
		}, regGroupID)
		text, items, _ := regParse(t, in, regDeps(nil, nil))
		if text != "" {
			t.Errorf("md 段不应留痕: got %q", text)
		}
		regFound(t, items, "markdown", b64MD)
	})

	t.Run("keyboard内层base64前缀", func(t *testing.T) {
		b64KB := base64.StdEncoding.EncodeToString([]byte(`{"id":"tpl_1"}`))
		in := regSegIn([]map[string]interface{}{
			{"type": "keyboard", "data": map[string]interface{}{
				"data": "base64://" + b64KB,
			}},
		}, regGroupID)
		text, items, _ := regParse(t, in, regDeps(nil, nil))
		if text != "" {
			t.Errorf("kb 段不应留痕: got %q", text)
		}
		regFound(t, items, "keyboard", `{"id":"tpl_1"}`)
	})
}

// ---------- 私聊/转发拦截（§9 N-M1） ----------

// TestCQParseRegress_NM1_PrivateActionIntercepted N-M1（M1 修复）— 期望=迁移后行为
// 私聊 scope 的动作码：不执行（RunAction 零调用）、不泄漏（码从正文移除）、不产 pending。
// 转发 scope 的拦截在接入层（send_group_forward_msg 收到 pendings 即丢弃），Input 层无转发标志。
func TestCQParseRegress_NM1_PrivateActionIntercepted(t *testing.T) {
	ra := &runActionRecorder{}
	for _, raw := range []string{
		`[CQ:remove,user_id=1]`,
		`[CQ:set_group,action=ban,group_id=1,user_id=2]`,
	} {
		in := regStrPrivate(raw + "正文")
		text, _, pendings := regParse(t, in, regDeps(nil, ra))

		if text != "正文" {
			t.Errorf("私聊动作码应移除不泄漏: input=%q got %q", raw, text)
		}
		if len(pendings) != 0 {
			t.Errorf("私聊不应产 pending: input=%q pendings=%v", raw, pendings)
		}
	}
	if ra.calls != 0 {
		t.Errorf("Parse 不得执行动作(RunAction 零调用): got %d", ra.calls)
	}
}

// ---------- 入站小修（§9 I 系 + lead 拍板 Q1/Q2） ----------

// TestCQParseRegress_Q1_MemberEmptyParamsOmitted Q1（lead 拍板：空参省略）
// — 期望=迁移后行为：段路径 member 空 group_id 不再产 group_id= 空 key，
// 执行器侧按 key 缺失回退 defaultGroupID（ExecOutcome 契约）。
func TestCQParseRegress_Q1_MemberEmptyParamsOmitted(t *testing.T) {
	in := regSegIn([]map[string]interface{}{
		{"type": "member", "data": map[string]interface{}{
			"type": "add", "group_id": "", "user_id": "42",
		}},
	}, regGroupID)
	text, _, pendings := regParse(t, in, regDeps(nil, nil))

	if strings.Contains(text, "group_id=,") {
		t.Errorf("空参应省略而非产 type=,group_id=,user_id= 形态: %q", text)
	}
	params := regPending(t, pendings, "member")
	if _, ok := params["group_id"]; ok {
		t.Errorf("空 group_id 应省略 key: %v", params)
	}
	if params["type"] != "add" || params["user_id"] != "42" {
		t.Errorf("非空参数应保留: %v", params)
	}
}

// TestCQParseRegress_Q2_TrimSpace Q2（lead 拍板采纳 TrimSpace）
func TestCQParseRegress_Q2_TrimSpace(t *testing.T) {
	t.Run("迁移后行为:媒体码参数TrimSpace", func(t *testing.T) {
		in := regStrIn(`[CQ:image, file=https://x.com/a.png ]`)
		text, items, _ := regParse(t, in, regDeps(nil, nil))
		if text != "" {
			t.Errorf("媒体码应识别并移除: got %q", text)
		}
		regFound(t, items, "url_images", "x.com/a.png")
	})

	t.Run("状态保持:动作码容忍空格", func(t *testing.T) {
		in := regStrIn(`[CQ:set_group, action=ban , group_id=1 , user_id=2 ]`)
		_, _, pendings := regParse(t, in, regDeps(nil, nil))
		params := regPending(t, pendings, "set_group")
		if params["action"] != "ban" || params["group_id"] != "1" || params["user_id"] != "2" {
			t.Errorf("动作码 TrimSpace 后参数应正确: got %v", params)
		}
	})
}

// TestCQParseRegress_IH1_ArrayStripBotAt I-H1（入站小修）— 期望=迁移后行为
// array=true 全量群消息 `<@bot> hello`：@bot 段剥离 + Trim，仅剩 text "hello"
// （对齐 AGENTS.md 与字符串路径语义；修 ConvertToSegmentedMessage 缺 isFullGroupMsg=true）。
func TestCQParseRegress_IH1_ArrayStripBotAt(t *testing.T) {
	botOpenID := "0123456789ABCDEF0123456789ABCDEF"
	RememberSelfAtID(botOpenID)

	msg := &dto.WSGroupMessageData{Content: "<@" + botOpenID + "> hello"}
	segs := ConvertToSegmentedMessage(msg)

	if len(segs) != 1 {
		t.Fatalf("应仅剩 1 个 text 段: got %+v", segs)
	}
	if segs[0]["type"] != "text" {
		t.Errorf("段类型: got %v, want text", segs[0]["type"])
	}
	if text, _ := segs[0]["data"].(map[string]interface{})["text"].(string); text != "hello" {
		t.Errorf("@bot 剥离后文本: got %q, want hello", text)
	}
}

// TestCQParseRegress_IM2_ArrayAttachmentContentType I-M2（入站小修）— 期望=迁移后行为
// array 模式非 image 附件（视频/语音）不再误标 image 段（ContentType 过滤）。
func TestCQParseRegress_IM2_ArrayAttachmentContentType(t *testing.T) {
	msg := &dto.WSGroupMessageData{
		Content: "看视频",
		Attachments: []*dto.MessageAttachment{
			{FileName: "video.mp4", ContentType: "video/mp4", URL: "https://x.com/v.mp4"},
			{FileName: "pic.png", ContentType: "image/png", URL: "https://x.com/p.png"},
		},
	}
	segs := ConvertToSegmentedMessage(msg)

	for _, s := range segs {
		if s["type"] == "image" {
			data, _ := s["data"].(map[string]interface{})
			if file, _ := data["file"].(string); strings.HasSuffix(file, ".mp4") || strings.Contains(file, "video") {
				t.Errorf("视频附件不应误标 image 段: %+v", s)
			}
		}
	}
	// image 前缀附件仍产 image 段
	hasImage := false
	for _, s := range segs {
		if s["type"] == "image" {
			hasImage = true
		}
	}
	if !hasImage {
		t.Errorf("ContentType=image/png 附件应产 image 段: %+v", segs)
	}
}

package cqparse

import (
	"context"
	"encoding/json"
	"fmt"
	"strconv"

	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/mylog"
)

// group_info 内容展开 handler（第一个内容扩展码，架构设计 §7/N-G 系）。
// - field 四枚举：name / memo / member_count / all；
// - group_id 省略时回退当前会话群（Q1：空 group_id 走缺省回退）；
// - group_id 支持虚拟 ID 与 32 位原生 OpenID 双格式，统一反查后以 OpenID 为去重键；
// - 同消息同目标群多码合并为一次 GroupInfo 调用（30 QPM 保护）；
// - 失败分级：参数错误→保留原文；ID 反查失败/官方错误/网络失败→替换 fallback（默认空串）。
type groupInfoHandler struct{}

func (groupInfoHandler) Kind() Kind   { return KindContent }
func (groupInfoHandler) Scope() Scope { return ScopeGroup | ScopePrivate | ScopeForward }

func init() {
	RegisterBatch("group_info", groupInfoHandler{})
}

var validGroupInfoFields = map[string]bool{
	"name":         true,
	"memo":         true,
	"member_count": true,
	"all":          true,
}

// groupInfoFetchState 记录同一目标群的取数状态，实现多码去重。
type groupInfoFetchState struct {
	done bool
	data *GroupInfoData
	err  error
}

func (groupInfoHandler) ResolveBatch(ctx *ResolveCtx, toks []Token) []Outcome {
	outs := make([]Outcome, len(toks))
	states := map[string]*groupInfoFetchState{}

	for i, tok := range toks {
		field := tok.Params["field"]
		if !validGroupInfoFields[field] {
			// 参数错误：保留原文（诚实暴露无效用法；段路径 Raw 为规范重渲染文本）
			outs[i] = Outcome{
				Replacement: tok.Raw,
				Warn:        "[CQ:group_info] 无效 field=" + field + ",未展开,保留码文本",
			}
			continue
		}

		gid := tok.Params["group_id"]
		if gid == "" {
			if !ctx.Input.HasGroup || ctx.Input.GroupID == "" {
				// 私聊且未指定群：诚实回退 fallback
				outs[i] = Outcome{
					Replacement: tok.Params["fallback"],
					Warn:        "[CQ:group_info] 当前会话无群上下文,替换为 fallback",
				}
				continue
			}
			// 修 Minor：省略 group_id 也统一先反查 OpenID 再入 states——
			// 与显式路径共享同一去重键，同群"省略/显式"两种写法合并为一次取数
			gid = ctx.Input.GroupID
		}

		resolved, err := resolveGroupOpenID(gid)
		if err != nil {
			outs[i] = Outcome{
				Replacement: tok.Params["fallback"],
				Warn:        fmt.Sprintf("[CQ:group_info] group_id 反查失败 group=%s field=%s: %v,替换为 fallback", gid, field, err),
			}
			continue
		}

		st := states[resolved]
		if st == nil {
			st = &groupInfoFetchState{}
			states[resolved] = st
		}
		if !st.done {
			st.done = true
			if ctx.Deps == nil || ctx.Deps.GroupInfo == nil {
				st.err = errDepsNotInjected
			} else {
				st.data, st.err = ctx.Deps.GroupInfo.GroupInfo(context.TODO(), resolved)
			}
		}
		if st.err != nil {
			// 官方错误码/网络失败：替换 fallback（默认空串）+ 日志（code+含义+field+group）
			outs[i] = Outcome{
				Replacement: tok.Params["fallback"],
				Warn:        fmt.Sprintf("[CQ:group_info] 取数失败 err=%v field=%s group=%s,替换为 fallback", st.err, field, resolved),
			}
			continue
		}
		outs[i] = Outcome{Replacement: groupInfoFieldValue(st.data, field)}
	}
	return outs
}

// resolveGroupOpenID 群 ID 双格式解析：32 位原生 OpenID 直通，否则 idmap 反查。
func resolveGroupOpenID(groupID string) (string, error) {
	if len(groupID) == 32 {
		return groupID, nil
	}
	realID, err := idmap.RetrieveRowByIDv2(groupID)
	if err != nil || realID == "" {
		return "", fmt.Errorf("反查 group_openid 失败: %w", err)
	}
	return realID, nil
}

// groupInfoFieldValue 按字段展开；all 为诚实三字段 JSON。
func groupInfoFieldValue(info *GroupInfoData, field string) string {
	switch field {
	case "name":
		return info.Name
	case "memo":
		return info.Memo
	case "member_count":
		return strconv.Itoa(info.MemberCount)
	case "all":
		b, err := json.Marshal(map[string]string{
			"name":         info.Name,
			"memo":         info.Memo,
			"member_count": strconv.Itoa(info.MemberCount),
		})
		if err != nil {
			mylog.Printf("[CQ:group_info] field=all JSON 化失败: %v", err)
			return ""
		}
		return string(b)
	}
	return ""
}

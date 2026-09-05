package cqparse

// 动作类 handler：member / remove / set_group（架构设计 §7）。
// 只产出 PendingAction 描述，不在 Parse 内执行；
// 执行器见 handlers/cqparse_exec.go（迁移 cq*Action 系列）。
// 作用域：仅群聊；私聊/转发的拦截由 walker 按 Scope 统一处理（修 M1）。

type actionHandler struct{ action string }

func (actionHandler) Kind() Kind   { return KindAction }
func (actionHandler) Scope() Scope { return ScopeGroup }

func init() {
	for _, a := range []string{"member", "remove", "set_group"} {
		Register(a, actionHandler{action: a})
	}
}

func (h actionHandler) Resolve(ctx *ResolveCtx, tok Token) Outcome {
	if h.action == "set_group" && !knownSetGroupActions[tok.Params["action"]] {
		// 未知 action 与今日一致保留原文
		return Outcome{Replacement: tok.Raw}
	}
	if h.action == "member" && len(tok.Params) == 0 {
		// 修 Minor：裸 [CQ:member]（无任何参数）legacy 正则不匹配保留字面，
		// new 对齐保留原文且不产 pending（空 pending 会无意义覆写跨群路由 realGroupID）
		return Outcome{Replacement: tok.Raw}
	}
	// 修 M8：段路径数值 group_id/user_id 已在 Normalize 阶段 coerce；
	// 修 C3：字符串路径批量 user_ids 尾随段已并入前值。
	return Outcome{
		Replacement: "",
		Pending: &PendingAction{
			Action:         tok.Action,
			Params:         tok.Params,
			Raw:            tok.Raw,
			Scope:          ScopeGroup,
			DefaultGroupID: ctx.Input.GroupID,
		},
	}
}

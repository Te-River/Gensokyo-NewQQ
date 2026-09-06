package cqparse

import (
	"strings"

	"github.com/hoshinonyaruko/gensokyo/mylog"
)

// Walker：Phase1 Resolve（纯解析）+ Phase3 Splice（单遍前向拼接）。
// 禁止二次扫描：Splice 是最后一次文本操作，Replacement/Found 值不回炉重扫。

// Parse 是对外主入口：三输入归一 → 纯解析 → 拼接。
// 动作码只产出 PendingAction，由调用方在原时序点 ExecutePending。
func Parse(in Input, d *Deps) (string, map[string][]string, []PendingAction, error) {
	doc, err := Normalize(in)
	if err != nil {
		return "", nil, nil, err
	}
	outcomes, found, pendings := resolve(&doc, &in, d)
	return splice(&doc, outcomes), found, pendings, nil
}

// resolve 按 Doc.Tokens 顺序分发 handler，收集 foundItems（文档顺序）与 pendings。
func resolve(doc *Doc, in *Input, d *Deps) ([]Outcome, map[string][]string, []PendingAction) {
	ctx := &ResolveCtx{Input: in, Deps: d}
	outcomes := make([]Outcome, len(doc.Tokens))
	viaBatch := make([]bool, len(doc.Tokens))

	// Phase1a：批量 handler（group_info 同群去重一次取数）
	batchIdx := map[string][]int{}
	for i, tok := range doc.Tokens {
		if tok.Kind == KindText || tok.Kind == KindPassthrough {
			continue
		}
		if _, ok := batchHandlers[tok.Action]; ok {
			batchIdx[tok.Action] = append(batchIdx[tok.Action], i)
		}
	}
	for action, idxs := range batchIdx {
		toks := make([]Token, len(idxs))
		for n, i := range idxs {
			toks[n] = doc.Tokens[i]
		}
		outs := batchHandlers[action].ResolveBatch(ctx, toks)
		for n, i := range idxs {
			outcomes[i] = outs[n]
			viaBatch[i] = true
		}
	}

	scope := ScopePrivate
	if in.HasGroup {
		scope = ScopeGroup
	}

	// Phase1b：单 Token handler / 作用域拦截 / passthrough
	for i, tok := range doc.Tokens {
		switch tok.Kind {
		case KindText:
			continue
		case KindPassthrough:
			// at/face/未知码：原文回填，正文与今日完全一致
			outcomes[i] = Outcome{Replacement: tok.Raw}
			continue
		}
		if viaBatch[i] {
			continue
		}
		h, ok := handlers[tok.Action]
		if !ok {
			outcomes[i] = Outcome{Replacement: tok.Raw}
			continue
		}
		if h.Scope()&scope == 0 {
			// 动作码在私聊/转发：不执行、不泄漏、不产 pending（修 M1）
			outcomes[i] = Outcome{
				Replacement: "",
				Warn:        "[cqparse] " + tok.Action + " 动作码不在当前会话作用域,已拦截不执行: " + tok.Raw,
			}
			continue
		}
		outcomes[i] = h.Resolve(ctx, tok)
	}

	// 收集 foundItems 与 pendings（文档顺序，修 m8 顺序副作用）
	found := make(map[string][]string)
	var pendings []PendingAction
	for i := range doc.Tokens {
		o := outcomes[i]
		for _, fi := range o.Found {
			found[fi.Key] = append(found[fi.Key], fi.Value)
		}
		if o.Pending != nil {
			pendings = append(pendings, *o.Pending)
		}
		if o.Warn != "" {
			mylog.Printf("%s", o.Warn)
		}
	}
	return outcomes, found, pendings
}

// splice 单遍前向拼接：文本区间原样输出，码 Token 输出 Replacement。
func splice(doc *Doc, outcomes []Outcome) string {
	var sb strings.Builder
	cursor := 0
	for i, tok := range doc.Tokens {
		if tok.Kind == KindText {
			// 文本段由区间拼接覆盖，不单独输出
			continue
		}
		sb.WriteString(doc.Source[cursor:tok.Span.Start])
		sb.WriteString(outcomes[i].Replacement)
		cursor = tok.Span.End
	}
	sb.WriteString(doc.Source[cursor:])
	return sb.String()
}

package cqparse

import (
	"fmt"
	"strings"

	"github.com/hoshinonyaruko/gensokyo/mylog"
)

// 影子模式对比（架构设计 §8.2）：
// 新解析只跑纯部分（Normalize+Resolve+Splice），与 legacy 的
// messageText / foundItems / 动作参数表 diff，差异仅 mylog 上报不生效。
// 动作副作用只由 legacy 执行，新路径仅比对"检测到的 action+params"集合——绝不双 kick。

// shadowDiffMaxRunes 日志截断上限（按 rune 计，避免按字节截断切破 UTF-8 多字节字符）。
const shadowDiffMaxRunes = 200

// ShadowCompare 对比新旧解析结果；legacyText 为 legacy 产物正文
// （动作码尚未移除，对比前先剥离动作码区间对齐口径）。
func ShadowCompare(legacyText string, legacyItems map[string][]string, newText string, newItems map[string][]string, newPendings []PendingAction) {
	// 修 Minor：legacyText 只 Tokenize 一次，剥离与动作提取复用同一文档
	doc := Tokenize(legacyText)
	legacyStripped := stripActionTokensDoc(doc)
	if legacyStripped != newText {
		mylog.Printf("[cqparse-shadow] 文本差异 legacy=%s new=%s",
			truncateForLog(legacyStripped), truncateForLog(newText))
	}
	if !sameFoundItems(legacyItems, newItems) {
		mylog.Printf("[cqparse-shadow] foundItems 差异 legacy=%s new=%s",
			truncateForLog(fmt.Sprint(legacyItems)), truncateForLog(fmt.Sprint(newItems)))
	}
	legacyActions := extractActionsDoc(doc)
	if !sameActions(legacyActions, newPendings) {
		mylog.Printf("[cqparse-shadow] 动作码差异 legacy=%s new=%s",
			truncateForLog(formatActions(legacyActions)), truncateForLog(formatPendings(newPendings)))
	}
}

// stripActionTokens 将文本中的动作码区间替换为空串，得到与 new 路径可对比的正文。
func stripActionTokens(text string) string {
	return stripActionTokensDoc(Tokenize(text))
}

func stripActionTokensDoc(doc Doc) string {
	var sb strings.Builder
	cursor := 0
	for _, tok := range doc.Tokens {
		if tok.Kind == KindText {
			continue
		}
		repl := ""
		if tok.Kind != KindAction {
			repl = tok.Raw
		}
		sb.WriteString(doc.Source[cursor:tok.Span.Start])
		sb.WriteString(repl)
		cursor = tok.Span.End
	}
	sb.WriteString(doc.Source[cursor:])
	return sb.String()
}

// extractActions 用纯 tokenizer 提取文本中的动作码描述（只读，无副作用）。
func extractActions(text string) []PendingAction {
	return extractActionsDoc(Tokenize(text))
}

func extractActionsDoc(doc Doc) []PendingAction {
	var acts []PendingAction
	for _, tok := range doc.Tokens {
		if tok.Kind != KindAction {
			continue
		}
		if tok.Action == "set_group" && !knownSetGroupActions[tok.Params["action"]] {
			continue
		}
		acts = append(acts, PendingAction{Action: tok.Action, Params: tok.Params, Raw: tok.Raw})
	}
	return acts
}

// sameFoundItems 按 key+顺序逐项比较。
func sameFoundItems(a, b map[string][]string) bool {
	if len(a) != len(b) {
		return false
	}
	for k, va := range a {
		vb, ok := b[k]
		if !ok || len(va) != len(vb) {
			return false
		}
		for i := range va {
			if va[i] != vb[i] {
				return false
			}
		}
	}
	return true
}

// sameActions 比较 action+params 集合（顺序敏感，Raw 不参与）。
func sameActions(a, b []PendingAction) bool {
	if len(a) != len(b) {
		return false
	}
	for i := range a {
		if a[i].Action != b[i].Action || !sameParams(a[i].Params, b[i].Params) {
			return false
		}
	}
	return true
}

func sameParams(a, b map[string]string) bool {
	if len(a) != len(b) {
		return false
	}
	for k, va := range a {
		if vb, ok := b[k]; !ok || va != vb {
			return false
		}
	}
	return true
}

func formatActions(acts []PendingAction) string {
	if len(acts) == 0 {
		return "[]"
	}
	parts := make([]string, 0, len(acts))
	for _, a := range acts {
		parts = append(parts, a.Action+":"+fmt.Sprint(a.Params))
	}
	return "[" + strings.Join(parts, " ") + "]"
}

func formatPendings(ps []PendingAction) string {
	if len(ps) == 0 {
		return "[]"
	}
	parts := make([]string, 0, len(ps))
	for _, p := range ps {
		parts = append(parts, p.Action+":"+fmt.Sprint(p.Params))
	}
	return "[" + strings.Join(parts, " ") + "]"
}

func truncateForLog(s string) string {
	// 修 Nit：按 rune 截断（按字节会在多字节字符中间切开产生乱码）
	runes := []rune(s)
	if len(runes) > shadowDiffMaxRunes {
		return string(runes[:shadowDiffMaxRunes]) + "...(truncated)"
	}
	return s
}

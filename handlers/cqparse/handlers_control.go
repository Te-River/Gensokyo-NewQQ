package cqparse

// 控制类 handler：reply / active / wakeup（架构设计 §7）；
// 另含入站专用码 ark（出站无消费点，纯剥离，对齐 legacy ProcessCQInboundOnlyCodes）。
// 修 M6（reply id 放宽为任意非空值，字母数字官方 msg_id 正确入 reply_msg_id）。

type controlHandler struct{ action string }

func (controlHandler) Kind() Kind   { return KindControl }
func (controlHandler) Scope() Scope { return ScopeGroup | ScopePrivate | ScopeForward }

// inboundOnlyHandler 入站专用码：消费 Token 但不产出任何内容（纯剥离）。
type inboundOnlyHandler struct{}

func (inboundOnlyHandler) Kind() Kind   { return KindContent }
func (inboundOnlyHandler) Scope() Scope { return ScopeGroup | ScopePrivate | ScopeForward }

func (inboundOnlyHandler) Resolve(_ *ResolveCtx, _ Token) Outcome {
	return Outcome{Replacement: ""}
}

func init() {
	for _, a := range []string{"reply", "active", "wakeup"} {
		Register(a, controlHandler{action: a})
	}
	Register("ark", inboundOnlyHandler{})
}

func (h controlHandler) Resolve(ctx *ResolveCtx, tok Token) Outcome {
	switch h.action {
	case "reply":
		if id := tok.Params["id"]; id != "" {
			return Outcome{
				Replacement: "",
				Found:       []FoundItem{{Key: "reply_msg_id", Value: id}},
			}
		}
		// 入站专用形态 [CQ:reply,msg_idx=...]：出站无消费点，纯剥离
		// （对齐 legacy ProcessCQInboundOnlyCodes；[CQ:reply,id=..] 语义不变）
		if _, ok := tok.Params["msg_idx"]; ok {
			return Outcome{Replacement: ""}
		}
	case "active":
		found := []FoundItem{{Key: "active", Value: "true"}}
		if v := tok.Params["type"]; v != "" {
			found = append(found, FoundItem{Key: "active_type", Value: v})
		}
		if v := tok.Params["sub_type"]; v != "" {
			found = append(found, FoundItem{Key: "active_sub_type", Value: v})
		}
		return Outcome{Replacement: "", Found: found}
	case "wakeup":
		if uid := tok.Params["userid"]; uid != "" {
			return Outcome{
				Replacement: "",
				Found:       []FoundItem{{Key: "wakeup", Value: uid}},
			}
		}
	}
	// 参数缺失：字符串路径保留原文（与今日正则不匹配一致）；段路径不留痕
	if tok.Segment {
		return Outcome{Replacement: ""}
	}
	return Outcome{Replacement: tok.Raw}
}

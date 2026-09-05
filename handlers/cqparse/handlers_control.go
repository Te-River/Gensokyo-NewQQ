package cqparse

// 控制类 handler：reply / active / wakeup（架构设计 §7）。
// 修 M6（reply id 放宽为任意非空值，字母数字官方 msg_id 正确入 reply_msg_id）。

type controlHandler struct{ action string }

func (controlHandler) Kind() Kind   { return KindControl }
func (controlHandler) Scope() Scope { return ScopeGroup | ScopePrivate | ScopeForward }

func init() {
	for _, a := range []string{"reply", "active", "wakeup"} {
		Register(a, controlHandler{action: a})
	}
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

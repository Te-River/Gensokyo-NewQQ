package cqparse

import (
	"encoding/base64"
	"encoding/json"
	"strings"

	"github.com/hoshinonyaruko/gensokyo/mylog"
)

// 内容类 handler：markdown / keyboard / card / input_notify / stream / music。
// 修 C1（tokenizer 括号配平，贪婪 JSON 吞正文已根治）；
// 修 m3（action 精确匹配，cardboard 不再被当 card）；
// 修 m2（stream 兼容 = 语法，tokenizer 冒号兼容）；
// 修 m8（markdown 整段一个 Token，内嵌 media 串不再被误提取）。

type contentHandler struct{ action string }

func (contentHandler) Kind() Kind  { return KindContent }
func (contentHandler) Scope() Scope { return ScopeGroup | ScopePrivate | ScopeForward }

func init() {
	for _, a := range []string{"markdown", "keyboard", "card", "input_notify", "stream", "music"} {
		Register(a, contentHandler{action: a})
	}
}

func (h contentHandler) Resolve(ctx *ResolveCtx, tok Token) Outcome {
	switch h.action {
	case "markdown":
		return resolveMarkdown(tok)
	case "keyboard":
		return resolveKeyboard(tok)
	case "card":
		return resolveCard(tok)
	case "input_notify":
		return resolveInputNotify(tok)
	case "stream":
		return resolveStream(tok)
	case "music":
		return resolveMusic(tok)
	}
	return Outcome{Replacement: tok.Raw}
}

// resolveMarkdown base64/JSON 双形态。
// 字符串路径：非 base64:// 且非 { 形态与今日一致保留原文；
// 段路径：data 为 map 时已在 Normalize 阶段 marshal 成 JSON 字符串。
func resolveMarkdown(tok Token) Outcome {
	data := tok.Params["data"]
	if data == "" {
		if tok.Segment {
			// 段路径 data 缺失：不留痕（与今日 "data is nil" 分支一致）
			return Outcome{Replacement: ""}
		}
		return Outcome{Replacement: tok.Raw}
	}
	if strings.HasPrefix(data, "base64://") {
		return Outcome{
			Replacement: "",
			Found:       []FoundItem{{Key: "markdown", Value: strings.TrimPrefix(data, "base64://")}},
		}
	}
	if tok.Segment {
		// 段路径语义：原始 base64（无前缀）校验为 JSON 后直接使用；
		// 否则解码实体后尝试 JSON，失败则跳过不发空 md（修 m6/N-E5）
		if decoded, err := base64.StdEncoding.DecodeString(data); err == nil && json.Valid(decoded) {
			return Outcome{Replacement: "", Found: []FoundItem{{Key: "markdown", Value: data}}}
		}
		s := DecodeValue(data)
		if json.Valid([]byte(s)) {
			return Outcome{
				Replacement: "",
				Found:       []FoundItem{{Key: "markdown", Value: base64.StdEncoding.EncodeToString([]byte(s))}},
			}
		}
		return Outcome{Replacement: ""}
	}
	if strings.HasPrefix(data, "{") {
		// JSON 形态 base64 编码后存入（与今日 mdJSONPattern 一致，不再贪婪跨码）
		return Outcome{
			Replacement: "",
			Found:       []FoundItem{{Key: "markdown", Value: base64.StdEncoding.EncodeToString([]byte(data))}},
		}
	}
	return Outcome{Replacement: tok.Raw}
}

// resolveKeyboard base64/JSON 双形态（JSON 形态原样透传，不 base64）。
func resolveKeyboard(tok Token) Outcome {
	data := tok.Params["data"]
	if data == "" {
		if tok.Segment {
			return Outcome{Replacement: ""}
		}
		return Outcome{Replacement: tok.Raw}
	}
	if strings.HasPrefix(data, "base64://") {
		decoded, err := base64.StdEncoding.DecodeString(strings.TrimPrefix(data, "base64://"))
		if err != nil {
			// 与今日一致：解码失败码移除 + 日志
			mylog.Printf("[CQ:keyboard] base64 解码失败: %v", err)
			return Outcome{Replacement: ""}
		}
		return Outcome{Replacement: "", Found: []FoundItem{{Key: "keyboard", Value: string(decoded)}}}
	}
	if tok.Segment || strings.HasPrefix(data, "{") {
		// 段路径原始字符串直通；字符串路径 JSON 形态原样存入
		return Outcome{Replacement: "", Found: []FoundItem{{Key: "keyboard", Value: data}}}
	}
	return Outcome{Replacement: tok.Raw}
}

// resolveCard 参数顺序无关，JSON 编码存入。
func resolveCard(tok Token) Outcome {
	cardData := make(map[string]string, len(tok.Params))
	for k, v := range tok.Params {
		if v != "" {
			cardData[k] = v
		}
	}
	if len(cardData) == 0 {
		// 与今日一致：裸 [CQ:card] 无参数时码移除、无产物
		return Outcome{Replacement: ""}
	}
	encoded, err := json.Marshal(cardData)
	if err != nil {
		mylog.Printf("[CQ:card] 参数 JSON 化失败: %v", err)
		return Outcome{Replacement: ""}
	}
	return Outcome{Replacement: "", Found: []FoundItem{{Key: "card", Value: string(encoded)}}}
}

// resolveInputNotify 参数 JSON 化（type 必填语义按今日宽容处理）。
func resolveInputNotify(tok Token) Outcome {
	notifyData := map[string]string{}
	if v := tok.Params["type"]; v != "" {
		notifyData["type"] = v
	}
	if v := tok.Params["second"]; v != "" {
		notifyData["second"] = v
	}
	if len(notifyData) == 0 {
		if tok.Segment {
			return Outcome{Replacement: ""}
		}
		return Outcome{Replacement: tok.Raw}
	}
	encoded, err := json.Marshal(notifyData)
	if err != nil {
		return Outcome{Replacement: ""}
	}
	return Outcome{Replacement: "", Found: []FoundItem{{Key: "input_notify", Value: string(encoded)}}}
}

// resolveStream 参数 JSON 化（m2：= 与冒号语法均已在 tokenizer 归一）。
func resolveStream(tok Token) Outcome {
	streamData := map[string]string{}
	if v := tok.Params["type"]; v != "" {
		streamData["type"] = v
	}
	if v := tok.Params["qq"]; v != "" {
		streamData["qq"] = v
	}
	if len(streamData) == 0 {
		if tok.Segment {
			return Outcome{Replacement: ""}
		}
		return Outcome{Replacement: tok.Raw}
	}
	encoded, err := json.Marshal(streamData)
	if err != nil {
		return Outcome{Replacement: ""}
	}
	return Outcome{Replacement: "", Found: []FoundItem{{Key: "stream", Value: string(encoded)}}}
}

// resolveMusic 仅支持 type=qq（其余与今日一致保留原文）。
func resolveMusic(tok Token) Outcome {
	if tok.Params["type"] == "qq" && tok.Params["id"] != "" {
		return Outcome{
			Replacement: "",
			Found:       []FoundItem{{Key: "qqmusic", Value: tok.Params["id"]}},
		}
	}
	if tok.Segment {
		return Outcome{Replacement: ""}
	}
	return Outcome{Replacement: tok.Raw}
}

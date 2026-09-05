package cqparse

import (
	"encoding/json"
	"fmt"
	"strconv"
	"strings"
)

// Normalize 将三种入口消息形态归一化为 Token 文档（架构设计 §5）。
// 段数组/TRSS 直产 Token，不经字符串往返，杜绝二次转义。
func Normalize(in Input) (Doc, error) {
	switch in.Kind {
	case InputString:
		return Tokenize(in.String), nil
	case InputSegments, InputMap:
		return normalizeSegments(in.Segments), nil
	default:
		return Doc{}, fmt.Errorf("cqparse: 未知输入类型 %d", in.Kind)
	}
}

// normalizeSegments 将消息段列表直产为 Token 流。
// - text/at/member/set_group 拼接进 Source（真实 Span）；
// - 其余类型零宽 Token（正文不留痕，与今日段路径一致）；
// - 未知段类型（face/forward 等）整体跳过（与今日 Unhandled 分支一致）。
func normalizeSegments(segs []map[string]interface{}) Doc {
	var toks []Token
	var sb strings.Builder
	for _, seg := range segs {
		if seg == nil {
			continue
		}
		segType, _ := seg["type"].(string)
		if segType == "" {
			continue
		}
		data, _ := seg["data"].(map[string]interface{})
		switch segType {
		case "text":
			t, _ := data["text"].(string)
			start := sb.Len()
			sb.WriteString(t)
			toks = append(toks, Token{Kind: KindText, Action: "text", Raw: t, Span: Span{Start: start, End: sb.Len()}})
		case "at":
			// at 渲染进正文，与今日 messageText 拼接一致；M8：数字 qq coerce
			qq := coerceString(data["qq"])
			raw := "[CQ:at,qq=" + qq + "]"
			start := sb.Len()
			sb.WriteString(raw)
			toks = append(toks, Token{
				Kind: KindPassthrough, Action: "at",
				Params: map[string]string{"qq": qq},
				Raw:    raw, Span: Span{Start: start, End: sb.Len()}, Segment: true,
			})
		case "member", "set_group":
			// 动作段直产 Params（Q1：空参数省略，空 group_id 走执行器缺省回退），
			// 不再经 buildSetGroupCQCode 字符串往返
			params := actionParams(segType, data)
			raw := renderActionCQ(segType, params)
			start := sb.Len()
			sb.WriteString(raw)
			toks = append(toks, Token{
				Kind: kindForAction(segType), Action: segType,
				Params: params, Raw: raw, Span: Span{Start: start, End: sb.Len()}, Segment: true,
			})
		default:
			kind := kindForAction(segType)
			if kind == KindPassthrough {
				// 未知段类型：不产 Token 不留痕（与今日 Unhandled 分支一致）
				continue
			}
			toks = append(toks, Token{
				Kind: kind, Action: segType,
				Params: dataParams(data),
				Raw:    "", Span: Span{Start: sb.Len(), End: sb.Len()}, Segment: true,
			})
		}
	}
	return Doc{Tokens: toks, Source: sb.String()}
}

// dataParams 将段 data map 全量参数化（M8：数值字段 coerceString）。
func dataParams(data map[string]interface{}) map[string]string {
	params := make(map[string]string, len(data))
	for k, v := range data {
		params[k] = coerceString(v)
	}
	return params
}

// setGroupParamKeys 与 buildSetGroupCQCode 的固定字段序一致，保证段路径
// 与字符串路径产物语义对齐；未知 key 与今日一样忽略。
var setGroupParamKeys = []string{
	"action", "group_id", "user_id", "user_ids", "duration", "enable",
	"approve", "flag", "reason", "add_to_member_blacklist", "add_blacklist", "strategy_id",
}

// actionParams 提取动作段参数；空值省略（Q1 拍板）。
func actionParams(action string, data map[string]interface{}) map[string]string {
	params := make(map[string]string)
	if action == "member" {
		for _, k := range []string{"type", "group_id", "user_id"} {
			if v := coerceString(data[k]); v != "" {
				params[k] = v
			}
		}
		return params
	}
	for _, k := range setGroupParamKeys {
		v, ok := data[k]
		if !ok {
			continue
		}
		if arr, isArr := v.([]interface{}); isArr {
			// user_ids 数组：逐元素 coerce 后逗号拼接（C3：与字符串转义路径等价）
			parts := make([]string, 0, len(arr))
			for _, item := range arr {
				if s := coerceString(item); s != "" {
					parts = append(parts, s)
				}
			}
			if s := strings.Join(parts, ","); s != "" {
				params[k] = s
			}
			continue
		}
		if s := coerceString(v); s != "" {
			params[k] = s
		}
	}
	return params
}

// renderActionCQ 按固定字段序渲染动作码规范形（仅用于 Doc.Source 与日志，
// Splice 时动作 Token 的 Replacement 为空，不会出现在最终正文）。
func renderActionCQ(action string, params map[string]string) string {
	keys := []string{"type", "group_id", "user_id"}
	if action == "set_group" {
		keys = setGroupParamKeys
	}
	var sb strings.Builder
	sb.WriteString("[CQ:")
	sb.WriteString(action)
	for _, k := range keys {
		v, ok := params[k]
		if !ok {
			continue
		}
		sb.WriteString(",")
		sb.WriteString(k)
		sb.WriteString("=")
		sb.WriteString(EncodeValue(v))
	}
	sb.WriteString("]")
	return sb.String()
}

// coerceString 柔性取值（M8）：string 直通、数字/布尔格式化，其他类型忽略。
func coerceString(v interface{}) string {
	switch tv := v.(type) {
	case string:
		return tv
	case float64:
		return strconv.FormatFloat(tv, 'f', -1, 64)
	case bool:
		return strconv.FormatBool(tv)
	case int:
		return strconv.Itoa(tv)
	case int64:
		return strconv.FormatInt(tv, 10)
	case json.Number:
		return tv.String()
	default:
		return ""
	}
}

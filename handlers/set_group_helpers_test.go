package handlers

import (
	"testing"
)

// TestCQParseParams 验证顺序无关解析与转义反转
func TestCQParseParams(t *testing.T) {
	tests := []struct {
		name   string
		params string
		want   map[string]string
	}{
		{
			name:   "标准参数顺序无关",
			params: "action=ban,user_id=789,duration=60,group_id=123",
			want:   map[string]string{"action": "ban", "user_id": "789", "duration": "60", "group_id": "123"},
		},
		{
			name:   "参数缺失返回空串",
			params: "action=ban",
			want:   map[string]string{"action": "ban"},
		},
		{
			name:   "值含逗号被转义还原",
			params: "action=add_request,reason=广告&#44;勿扰",
			want:   map[string]string{"action": "add_request", "reason": "广告,勿扰"},
		},
		{
			name:   "值含右括号被转义还原",
			params: "action=add_request,reason=请勿&#93;打扰",
			want:   map[string]string{"action": "add_request", "reason": "请勿]打扰"},
		},
		{
			name:   "值含 & 被转义还原",
			params: "action=ban,user_id=1&amp;2",
			want:   map[string]string{"action": "ban", "user_id": "1&2"},
		},
		{
			name:   "值含等号保留（SplitN 只切第一个 =）",
			params: "action=ban,reason=a=b",
			want:   map[string]string{"action": "ban", "reason": "a=b"},
		},
		{
			name:   "无等号片段被忽略",
			params: "action=ban,broken,duration=60",
			want:   map[string]string{"action": "ban", "duration": "60"},
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := cqParseParams(tt.params)
			if len(got) != len(tt.want) {
				t.Fatalf("参数数量: got %d, want %d, got=%v", len(got), len(tt.want), got)
			}
			for k, wantV := range tt.want {
				if got[k] != wantV {
					t.Errorf("key %s: got %q, want %q", k, got[k], wantV)
				}
			}
		})
	}
}

// TestBuildSetGroupCQCode 验证消息段 data → CQ 码字符串的拼装（含非 string 值与转义）
func TestBuildSetGroupCQCode(t *testing.T) {
	tests := []struct {
		name string
		data map[string]interface{}
		want string
	}{
		{
			name: "全 string 参数顺序固定",
			data: map[string]interface{}{
				"action":   "ban",
				"group_id": "123",
				"user_id":  "456",
				"duration": "60",
			},
			want: "[CQ:set_group,action=ban,group_id=123,user_id=456,duration=60]",
		},
		{
			name: "数字 duration 不丢失（禁言而非解禁）",
			data: map[string]interface{}{
				"action":   "ban",
				"group_id": "123",
				"user_id":  "456",
				"duration": float64(60),
			},
			want: "[CQ:set_group,action=ban,group_id=123,user_id=456,duration=60]",
		},
		{
			name: "布尔 enable 不丢失",
			data: map[string]interface{}{
				"action":   "whole_ban",
				"group_id": "123",
				"enable":   true,
			},
			want: "[CQ:set_group,action=whole_ban,group_id=123,enable=true]",
		},
		{
			name: "reason 含逗号被转义",
			data: map[string]interface{}{
				"action":   "add_request",
				"group_id": "123",
				"user_id":  "456",
				"flag":     "f_1",
				"approve":  "false",
				"reason":   "广告,勿扰",
			},
			// 字段按 buildSetGroupCQCode 固定顺序：approve 在 flag 之前
			want: "[CQ:set_group,action=add_request,group_id=123,user_id=456,approve=false,flag=f_1,reason=广告&#44;勿扰]",
		},
		{
			name: "无参数返回空串",
			data: map[string]interface{}{},
			want: "",
		},
		{
			name: "空串值被跳过",
			data: map[string]interface{}{
				"action": "ban",
				"user_id": "",
			},
			want: "[CQ:set_group,action=ban]",
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := buildSetGroupCQCode(tt.data)
			if got != tt.want {
				t.Errorf("拼装结果:\n  got:  %q\n  want: %q", got, tt.want)
			}
		})
	}
}

// TestBuildSetGroupCQCodeRoundTrip 验证消息段拼装 → 真实解析路径的往返一致性（转义不丢失）
func TestBuildSetGroupCQCodeRoundTrip(t *testing.T) {
	data := map[string]interface{}{
		"action":   "add_request",
		"group_id": "123",
		"user_id":  "456",
		"flag":     "f_1",
		"approve":  "false",
		"reason":   "广告,勿扰]请自重&再见",
	}
	cq := buildSetGroupCQCode(data)
	// 模拟 ProcessOutboundCQCodes 的参数提取：去掉 [CQ:set_group, 前缀与结尾 ]
	paramsStr := cq[len("[CQ:set_group,") : len(cq)-1]
	got := cqParseParams(paramsStr)
	wantReason := "广告,勿扰]请自重&再见"
	if got["reason"] != wantReason {
		t.Errorf("往返后 reason: got %q, want %q (paramsStr=%s)", got["reason"], wantReason, paramsStr)
	}
	for _, k := range []string{"action", "group_id", "user_id", "flag", "approve"} {
		if got[k] != data[k].(string) {
			t.Errorf("往返后 %s: got %q, want %q", k, got[k], data[k])
		}
	}
}

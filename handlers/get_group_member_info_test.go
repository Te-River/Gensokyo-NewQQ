package handlers

// get_group_member_info 回执契约：
//   - 上游成功 → retcode:0/status:"ok"，data 为成员信息（body 形状不变）
//   - 反查失败 → retcode:1400/status:"failed"（参数/映射问题）
//   - 官方接口失败（如 400 code:11253 应用无接口访问权限）→ retcode:1500/status:"failed"，
//     message 带真实错误，不再用中性假数据伪装成功。

import (
	"context"
	"encoding/json"
	"strings"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/errs"
	"github.com/tencent-connect/botgo/openapi"
)

// groupMemberInfoMockAPI 记录 GroupMemberInfo 调用，其余方法走嵌入接口兜底
type groupMemberInfoMockAPI struct {
	openapi.OpenAPI
	member      *dto.GroupMember
	err         error
	calls       int
	gotGroupID  string
	gotMemberID string
}

func (m *groupMemberInfoMockAPI) GroupMemberInfo(_ context.Context, groupOpenID, memberOpenID string) (*dto.GroupMember, error) {
	m.calls++
	m.gotGroupID = groupOpenID
	m.gotMemberID = memberOpenID
	return m.member, m.err
}

func decodeGroupMemberInfoReply(t *testing.T, raw string) map[string]interface{} {
	t.Helper()
	var got map[string]interface{}
	if err := json.Unmarshal([]byte(raw), &got); err != nil {
		t.Fatalf("回执不是合法 JSON: %v (%s)", err, raw)
	}
	return got
}

// TestGetGroupMemberInfoSuccess 上游成功路径回执形状不变：retcode:0 / status:ok / data 为成员信息。
func TestGetGroupMemberInfoSuccess(t *testing.T) {
	client := &contractTestClient{}
	mock := &groupMemberInfoMockAPI{member: &dto.GroupMember{
		Username:   "测试成员",
		MemberRole: "admin",
		JoinedAt:   "2026-08-05T14:19:09+08:00",
	}}

	raw, err := GetGroupMemberInfo(client, nil, mock, callapi.ActionMessage{
		Action: "get_group_member_info",
		Params: callapi.ParamsContent{GroupID: openID32('g'), UserID: openID32('u')},
		Echo:   "member-1",
	})
	if err != nil {
		t.Fatalf("GetGroupMemberInfo 返回错误: %v", err)
	}
	got := decodeGroupMemberInfoReply(t, raw)
	if got["retcode"] != float64(0) || got["status"] != "ok" {
		t.Fatalf("成功路径回执不对: %s", raw)
	}
	if _, hasMessage := got["message"]; hasMessage {
		t.Errorf("成功路径不应新增 message 字段: %s", raw)
	}
	data, _ := got["data"].(map[string]interface{})
	if data == nil {
		t.Fatalf("data 缺失: %s", raw)
	}
	if data["nickname"] != "测试成员" || data["role"] != "admin" {
		t.Errorf("成员字段不对: %v", data)
	}
	if mock.calls != 1 {
		t.Errorf("GroupMemberInfo 调用次数: got %d, want 1", mock.calls)
	}
}

// TestGetGroupMemberInfoUpstreamFailureRepliesFailed 官方 400（应用无接口访问权限）→ 1500/failed，带真实错误。
func TestGetGroupMemberInfoUpstreamFailureRepliesFailed(t *testing.T) {
	client := &contractTestClient{}
	mock := &groupMemberInfoMockAPI{err: errs.New(11253, "应用无接口访问权限")}

	raw, err := GetGroupMemberInfo(client, nil, mock, callapi.ActionMessage{
		Action: "get_group_member_info",
		Params: callapi.ParamsContent{GroupID: openID32('g'), UserID: openID32('u')},
		Echo:   "member-fail",
	})
	if err != nil {
		t.Fatalf("GetGroupMemberInfo 返回错误: %v", err)
	}
	got := decodeGroupMemberInfoReply(t, raw)
	if got["retcode"] != float64(1500) {
		t.Errorf("retcode = %v, want 1500 (%s)", got["retcode"], raw)
	}
	if got["status"] != "failed" {
		t.Errorf("status = %v, want failed (%s)", got["status"], raw)
	}
	if data, ok := got["data"]; !ok || data != nil {
		t.Errorf("data 应为 null，实际: %v (%s)", data, raw)
	}
	msg, _ := got["message"].(string)
	if !strings.Contains(msg, "11253") || !strings.Contains(msg, "应用无接口访问权限") {
		t.Errorf("message 应含官方 code 与 message，实际: %q", msg)
	}
	if got["echo"] != "member-fail" {
		t.Errorf("echo 应原样回显: %v", got["echo"])
	}
}

// TestGetGroupMemberInfoResolveFailureRepliesParamError 参数无法反查 → 1400，且不调用上游。
func TestGetGroupMemberInfoResolveFailureRepliesParamError(t *testing.T) {
	client := &contractTestClient{}
	mock := &groupMemberInfoMockAPI{}

	raw, err := GetGroupMemberInfo(client, nil, mock, callapi.ActionMessage{
		Action: "get_group_member_info",
		Params: callapi.ParamsContent{GroupID: openID32('g'), UserID: "999999999999999999"},
		Echo:   "member-404",
	})
	if err != nil {
		t.Fatalf("GetGroupMemberInfo 返回错误: %v", err)
	}
	got := decodeGroupMemberInfoReply(t, raw)
	if got["retcode"] != float64(1400) {
		t.Errorf("retcode = %v, want 1400 (%s)", got["retcode"], raw)
	}
	if got["status"] != "failed" {
		t.Errorf("status = %v, want failed", got["status"])
	}
	if mock.calls != 0 {
		t.Errorf("反查失败不应调用官方接口，实际调用 %d 次", mock.calls)
	}
}

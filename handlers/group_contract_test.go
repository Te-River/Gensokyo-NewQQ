package handlers

// 群管理接口与 QQ 官方 v2 现文档的契约测试：
//   - bot_state / join_request_list / restrict_chat_setting 的 DTO 反序列化
//   - 入群申请事件 auto_approved 对象
//   - set_group_whole_ban 的明确失败回执（官方 POST 无全员禁言写入字段）

import (
	"context"
	"encoding/json"
	"strings"
	"testing"
	"time"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

// contractMockOpenAPI 记录 SetRestrictChatSetting 请求体，其余方法走嵌入接口兜底
type contractMockOpenAPI struct {
	openapi.OpenAPI
	setReqs []*dto.RestrictChatSetting
}

func (m *contractMockOpenAPI) SetRestrictChatSetting(_ context.Context, _ string, setting *dto.RestrictChatSetting) error {
	m.setReqs = append(m.setReqs, setting)
	return nil
}

// contractTestClient 记录回执 payload
type contractTestClient struct{ last map[string]interface{} }

func (c *contractTestClient) SendMessage(m map[string]interface{}) error { c.last = m; return nil }

// TestJoinRequestListOfficialJSON 官方 join_request_list 响应示例反序列化
func TestJoinRequestListOfficialJSON(t *testing.T) {
	raw := `{"list":[{"join_request_id":"jr-1","risk_tips":"","union_openid":"union-1","member_openid":"member-1","username":"小明","apply_at":"2026-08-05T14:19:09+08:00","apply_source":"self_apply","invited_by":"","bot":false,"verify_info":{"method":"verify_message","verify_message":"几款看看","review_qa_list":[]}}],"next_cursor":"1785767153250497"}`
	var list dto.JoinRequestList
	if err := json.Unmarshal([]byte(raw), &list); err != nil {
		t.Fatalf("官方示例应反序列化成功: %v", err)
	}
	if len(list.List) != 1 {
		t.Fatalf("list 长度: got %d, want 1", len(list.List))
	}
	got := list.List[0]
	if got.JoinRequestID != "jr-1" || got.UnionOpenID != "union-1" || got.MemberOpenID != "member-1" {
		t.Errorf("id 字段: %+v", got)
	}
	if got.ApplyAt != "2026-08-05T14:19:09+08:00" {
		t.Errorf("apply_at 应为 RFC3339 字符串: got %q", got.ApplyAt)
	}
	if got.ApplySource != "self_apply" || got.Bot {
		t.Errorf("apply_source/bot: %+v", got)
	}
	if got.VerifyInfo.Method != "verify_message" || got.VerifyInfo.String() != "几款看看" {
		t.Errorf("verify_info: %+v", got.VerifyInfo)
	}
	if list.NextCursor != "1785767153250497" {
		t.Errorf("next_cursor: got %q", list.NextCursor)
	}
}

// TestBotInGroupStateOfficialJSON 官方 bot_state 响应示例反序列化 + joined_at 解析
func TestBotInGroupStateOfficialJSON(t *testing.T) {
	raw := `{"member_openid":"bot-openid","joined_at":"2026-08-05T14:19:09+08:00","allow_proactive_msg":true,"recv_msg_setting":"only_mention","member_role":"admin"}`
	var state dto.BotInGroupState
	if err := json.Unmarshal([]byte(raw), &state); err != nil {
		t.Fatalf("官方示例应反序列化成功: %v", err)
	}
	if state.MemberOpenID != "bot-openid" || !state.AllowProactiveMsg ||
		state.RecvMsgSetting != "only_mention" || state.MemberRole != "admin" {
		t.Errorf("bot_state 字段: %+v", state)
	}
	want := time.Date(2026, 8, 5, 14, 19, 9, 0, time.FixedZone("CST", 8*3600)).Unix()
	if got := parseBotJoinedAt(state.JoinedAt); got != want {
		t.Errorf("joined_at 应转 Unix 秒: got %d, want %d", got, want)
	}
	if got := parseBotJoinedAt(""); got != 0 {
		t.Errorf("空 joined_at 应置 0: got %d", got)
	}
	if got := parseBotJoinedAt("not-a-time"); got != 0 {
		t.Errorf("非法 joined_at 应置 0: got %d", got)
	}
}

// TestGroupJoinRequestEventAutoApprovedObject 事件 auto_approved 为对象（AutoAppproved）
func TestGroupJoinRequestEventAutoApprovedObject(t *testing.T) {
	raw := `{"id":"e1","event_id":"ev1","group_openid":"g1","join_request_id":"jr1","member_openid":"m1","username":"小明","apply_at":"2026-08-05T14:19:09+08:00","apply_source":"invited","invited_by":"u1","risk_tips":"warning_tips","union_openid":"union1","bot":true,"verify_info":{"method":"admin_review_qa","verify_message":"","review_qa_list":[{"question":"q","answer":"a"}]},"auto_approved":{"strategy_id":"s1"}}`
	var ev dto.GroupJoinRequestEvent
	if err := json.Unmarshal([]byte(raw), &ev); err != nil {
		t.Fatalf("auto_approved 对象应反序列化成功: %v", err)
	}
	if ev.AutoApproved == nil || ev.AutoApproved.StrategyID != "s1" {
		t.Fatalf("auto_approved: %+v", ev.AutoApproved)
	}
	if len(ev.VerifyInfo.ReviewQAList) != 1 || ev.VerifyInfo.ReviewQAList[0].Question != "q" {
		t.Errorf("review_qa_list: %+v", ev.VerifyInfo.ReviewQAList)
	}
	if ev.RiskTips != "warning_tips" || !ev.Bot || ev.UnionOpenID != "union1" {
		t.Errorf("事件新字段: %+v", ev)
	}
}

// TestRestrictChatSettingOfficialJSON 官方 restrict_chat_setting 查询响应反序列化
func TestRestrictChatSettingOfficialJSON(t *testing.T) {
	raw := `{"global_rule":{"mode":"schedule","schedule_rules":[{"task_id":"t1","start_at":"2026-08-05T14:19:09+08:00","end_at":"2026-08-05T15:19:09+08:00","enabled":true}],"recurring_rules":[{"task_id":"t2","weekdays":[1,7],"start_time":"22:00","end_time":"06:00","enabled":true}]},"members":[{"member_openid":"m1","mute_expire_at":"2026-08-06T14:19:09+08:00","username":"小明","union_openid":"u1"}]}`
	var setting dto.RestrictChatSetting
	if err := json.Unmarshal([]byte(raw), &setting); err != nil {
		t.Fatalf("官方示例应反序列化成功: %v", err)
	}
	if setting.GlobalRule == nil || setting.GlobalRule.Mode != "schedule" {
		t.Fatalf("global_rule: %+v", setting.GlobalRule)
	}
	if len(setting.GlobalRule.ScheduleRules) != 1 || setting.GlobalRule.ScheduleRules[0].TaskID != "t1" {
		t.Errorf("schedule_rules: %+v", setting.GlobalRule.ScheduleRules)
	}
	if len(setting.GlobalRule.RecurringRules) != 1 || len(setting.GlobalRule.RecurringRules[0].Weekdays) != 2 {
		t.Errorf("recurring_rules: %+v", setting.GlobalRule.RecurringRules)
	}
	if len(setting.Members) != 1 || setting.Members[0].MuteExpireAt != "2026-08-06T14:19:09+08:00" ||
		setting.Members[0].Username != "小明" {
		t.Errorf("members: %+v", setting.Members)
	}
}

// TestApplyRestrictChatSettingBanOps 禁言/解禁按官方 op 增量语义提交
func TestApplyRestrictChatSettingBanOps(t *testing.T) {
	mock := &contractMockOpenAPI{}
	if err := applyRestrictChatSetting(mock, "g", "m", 60, nil); err != nil {
		t.Fatalf("禁言应成功: %v", err)
	}
	if len(mock.setReqs) != 1 || len(mock.setReqs[0].Members) != 1 {
		t.Fatalf("应提交 1 条成员禁言: %+v", mock.setReqs)
	}
	item := mock.setReqs[0].Members[0]
	if item.Op != "add" || item.MemberOpenID != "m" {
		t.Errorf("禁言应为 op=add: %+v", item)
	}
	if _, err := time.Parse(time.RFC3339, item.MuteExpireAt); err != nil {
		t.Errorf("mute_expire_at 应为 RFC3339: %q", item.MuteExpireAt)
	}
	body, err := json.Marshal(mock.setReqs[0])
	if err != nil {
		t.Fatalf("请求体应可序列化: %v", err)
	}
	if strings.Contains(string(body), "global_rule") {
		t.Errorf("POST 请求体不应携带 global_rule: %s", body)
	}

	if err := applyRestrictChatSetting(mock, "g", "m", 0, nil); err != nil {
		t.Fatalf("解禁应成功: %v", err)
	}
	item = mock.setReqs[1].Members[0]
	if item.Op != "del" || item.MuteExpireAt != "" {
		t.Errorf("解禁应为 op=del + 空 mute_expire_at: %+v", item)
	}
}

// TestSetGroupWholeBanReturnsFailure 官方 POST 无全员禁言写入字段：明确失败回执，不提交请求
func TestSetGroupWholeBanReturnsFailure(t *testing.T) {
	const groupOpenID = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
	if err := idmap.WriteConfigv2(groupOpenID, "type", "group"); err != nil {
		t.Fatalf("写入群类型失败: %v", err)
	}
	mock := &contractMockOpenAPI{}
	client := &contractTestClient{}
	msg := callapi.ActionMessage{
		Action: "set_group_whole_ban",
		Params: callapi.ParamsContent{GroupID: groupOpenID, Enable: true},
		Echo:   "e-whole-ban",
	}
	got, err := SetGroupWholeBan(client, nil, mock, msg)
	if err != nil {
		t.Fatalf("SetGroupWholeBan 返回错误: %v", err)
	}
	var resp struct {
		Message string `json:"message"`
		RetCode int    `json:"retcode"`
		Status  string `json:"status"`
	}
	if err := json.Unmarshal([]byte(got), &resp); err != nil {
		t.Fatalf("回执应可解析: %v (%s)", err, got)
	}
	if resp.RetCode != 100 || resp.Status != "failed" {
		t.Errorf("应返回明确失败: %+v", resp)
	}
	if !strings.Contains(resp.Message, "全员禁言") {
		t.Errorf("message 应说明原因: %q", resp.Message)
	}
	if len(mock.setReqs) != 0 {
		t.Errorf("不应提交全员禁言请求: %+v", mock.setReqs)
	}
}

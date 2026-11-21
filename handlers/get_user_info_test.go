package handlers

// get_user_info（QQ 官方 2026-11-16 新增 GET /v2/users/{user_openid}/info）契约测试：
//   - 缺 user_id → retcode 1400，且不调用上游
//   - 上游成功 → data 四字段（user_id 原样回显）
//   - 上游失败 → retcode 1500 + data null + 上游错误信息

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

// userInfoMockAPI 记录 UserInfo 调用，其余方法走嵌入接口兜底
type userInfoMockAPI struct {
	openapi.OpenAPI
	info      *dto.UserInfo
	err       error
	calls     int
	gotOpenID string
}

func (m *userInfoMockAPI) UserInfo(_ context.Context, userOpenID string) (*dto.UserInfo, error) {
	m.calls++
	m.gotOpenID = userOpenID
	return m.info, m.err
}

// decodeUserInfoReply 解析回执为 map，便于按字段断言（含 data:null 的形态）
func decodeUserInfoReply(t *testing.T, raw string) map[string]interface{} {
	t.Helper()
	var got map[string]interface{}
	if err := json.Unmarshal([]byte(raw), &got); err != nil {
		t.Fatalf("回执不是合法 JSON: %v (%s)", err, raw)
	}
	return got
}

func TestGetUserInfoMissingUserID(t *testing.T) {
	client := &contractTestClient{}
	mock := &userInfoMockAPI{}

	raw, err := GetUserInfo(client, nil, mock, callapi.ActionMessage{Action: "get_user_info"})
	if err != nil {
		t.Fatalf("GetUserInfo 返回错误: %v", err)
	}
	got := decodeUserInfoReply(t, raw)
	if got["retcode"] != float64(1400) {
		t.Errorf("retcode = %v, want 1400 (%s)", got["retcode"], raw)
	}
	if got["status"] != "failed" {
		t.Errorf("status = %v, want failed", got["status"])
	}
	if _, ok := got["data"]; !ok || got["data"] != nil {
		t.Errorf("data 应为 null: %v", got["data"])
	}
	if mock.calls != 0 {
		t.Errorf("缺参时不应调用上游，实际调用 %d 次", mock.calls)
	}
	if client.last["retcode"] != float64(1400) {
		t.Errorf("SendMessage 回执 retcode = %v, want 1400", client.last["retcode"])
	}
}

// TestGetUserInfoNumericUserID 入参 user_id 为数字时经 ParamsContent 归一为字符串
func TestGetUserInfoNumericUserID(t *testing.T) {
	var msg callapi.ActionMessage
	if err := json.Unmarshal([]byte(`{"action":"get_user_info","params":{"user_id":12345}}`), &msg); err != nil {
		t.Fatalf("解析 action 失败: %v", err)
	}
	if uid, _ := msg.Params.UserID.(string); uid != "12345" {
		t.Errorf("数字 user_id 应归一为字符串 \"12345\"，实际 %v", msg.Params.UserID)
	}
}

func TestGetUserInfoSuccess(t *testing.T) {
	client := &contractTestClient{}
	openID := strings.Repeat("a", 32) // 32 位原生 openid 直接使用，不走 idmap
	mock := &userInfoMockAPI{info: &dto.UserInfo{
		Username:          "同道中人",
		AllowProactiveMsg: true,
		Avatar:            "https://thirdqq.qlogo.cn/bqqthird?x=1",
	}}

	raw, err := GetUserInfo(client, nil, mock, callapi.ActionMessage{
		Action: "get_user_info",
		Params: callapi.ParamsContent{UserID: openID},
	})
	if err != nil {
		t.Fatalf("GetUserInfo 返回错误: %v", err)
	}
	got := decodeUserInfoReply(t, raw)
	if got["retcode"] != float64(0) || got["status"] != "ok" || got["message"] != "" {
		t.Errorf("回执头不符: %s", raw)
	}
	if mock.gotOpenID != openID {
		t.Errorf("上游 openid = %q, want %q", mock.gotOpenID, openID)
	}
	data, _ := got["data"].(map[string]interface{})
	if data == nil {
		t.Fatalf("data 缺失: %s", raw)
	}
	if data["user_id"] != openID {
		t.Errorf("data.user_id = %v, want 原样回显 %q", data["user_id"], openID)
	}
	if data["username"] != "同道中人" {
		t.Errorf("data.username = %v", data["username"])
	}
	if data["allow_proactive_msg"] != true {
		t.Errorf("data.allow_proactive_msg = %v, want true", data["allow_proactive_msg"])
	}
	if data["avatar"] != "https://thirdqq.qlogo.cn/bqqthird?x=1" {
		t.Errorf("data.avatar = %v", data["avatar"])
	}
}

func TestGetUserInfoUpstreamError(t *testing.T) {
	client := &contractTestClient{}
	mock := &userInfoMockAPI{err: errors.New("500 上游炸了")}

	raw, err := GetUserInfo(client, nil, mock, callapi.ActionMessage{
		Action: "get_user_info",
		Params: callapi.ParamsContent{UserID: strings.Repeat("b", 32)},
	})
	if err != nil {
		t.Fatalf("GetUserInfo 返回错误: %v", err)
	}
	got := decodeUserInfoReply(t, raw)
	if got["retcode"] != float64(1500) {
		t.Errorf("retcode = %v, want 1500 (%s)", got["retcode"], raw)
	}
	if got["status"] != "failed" {
		t.Errorf("status = %v, want failed", got["status"])
	}
	if _, ok := got["data"]; !ok || got["data"] != nil {
		t.Errorf("data 应为 null: %v", got["data"])
	}
	if msg, _ := got["message"].(string); !strings.Contains(msg, "500 上游炸了") {
		t.Errorf("message = %v, 应包含上游错误", got["message"])
	}
}

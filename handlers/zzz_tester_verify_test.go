package handlers

// 独立验证（tester）：不复用 implementer 用例的断言路径，直接以 mock OpenAPI
// 驱动 handler 全链，核对请求参数、响应字段与边界值。

import (
	"context"
	"encoding/json"
	"regexp"
	"runtime"
	"strings"
	"testing"
	"time"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

type tvMockAPI struct {
	openapi.OpenAPI
	botState     *dto.BotInGroupState
	listResp     *dto.JoinRequestList
	gotGroup     string
	gotCursor    string
	gotLimit     int
	setReqs      []*dto.RestrictChatSetting
	prepareCalls int
}

func (m *tvMockAPI) BotInGroupState(_ context.Context, g string) (*dto.BotInGroupState, error) {
	m.gotGroup = g
	return m.botState, nil
}

func (m *tvMockAPI) JoinRequestList(_ context.Context, g, cursor string, limit int) (*dto.JoinRequestList, error) {
	m.gotGroup, m.gotCursor, m.gotLimit = g, cursor, limit
	return m.listResp, nil
}

func (m *tvMockAPI) SetRestrictChatSetting(_ context.Context, _ string, s *dto.RestrictChatSetting) error {
	m.setReqs = append(m.setReqs, s)
	return nil
}

func (m *tvMockAPI) FileUploadPrepare(_ context.Context, _ string, _ bool, _ *dto.UploadPrepareRequest) (*dto.UploadPrepareResponse, error) {
	m.prepareCalls++
	return &dto.UploadPrepareResponse{UploadID: "u", Parts: []dto.UploadPart{{Index: 0, PresignedURL: "http://127.0.0.1:1/x"}}}, nil
}

type tvClient struct{ last map[string]interface{} }

func (c *tvClient) SendMessage(m map[string]interface{}) error { c.last = m; return nil }

func tvSeg(segs []map[string]interface{}, typ string) map[string]interface{} {
	for _, s := range segs {
		if s["type"] == typ {
			return s
		}
	}
	return nil
}

func tvSegData(t *testing.T, seg map[string]interface{}) map[string]interface{} {
	t.Helper()
	if seg == nil {
		t.Fatal("段缺失")
	}
	d, ok := seg["data"].(map[string]interface{})
	if !ok {
		t.Fatalf("段 data 类型: %#v", seg["data"])
	}
	return d
}

// TestTesterVerify_BotStateDoubleWrite 8 字段双写 + join_time 由 RFC3339 解析
func TestTesterVerify_BotStateDoubleWrite(t *testing.T) {
	const groupOpenID = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
	mock := &tvMockAPI{botState: &dto.BotInGroupState{
		MemberOpenID:      "bot-openid",
		JoinedAt:          "2026-08-05T14:19:09+08:00",
		AllowProactiveMsg: true,
		RecvMsgSetting:    "only_mention",
		MemberRole:        "admin",
	}}
	client := &tvClient{}
	got, err := GetGroupBotState(client, nil, mock, callapi.ActionMessage{
		Action: "get_group_bot_state",
		Params: callapi.ParamsContent{GroupID: groupOpenID},
		Echo:   "e1",
	})
	if err != nil {
		t.Fatalf("handler 返回错误: %v", err)
	}
	if mock.gotGroup != groupOpenID {
		t.Errorf("传给 API 的 group_openid: got %q", mock.gotGroup)
	}
	var resp struct {
		Data    map[string]interface{} `json:"data"`
		RetCode int                    `json:"retcode"`
		Status  string                 `json:"status"`
	}
	if err := json.Unmarshal([]byte(got), &resp); err != nil {
		t.Fatalf("响应解析失败: %v (%s)", err, got)
	}
	if resp.RetCode != 0 || resp.Status != "ok" {
		t.Fatalf("成功回执: %+v", resp)
	}
	wantUnix := float64(time.Date(2026, 8, 5, 14, 19, 9, 0, time.FixedZone("CST", 8*3600)).Unix())
	checks := []struct {
		key  string
		want interface{}
	}{
		{"join_time", wantUnix},
		{"can_push", true},
		{"push_msg_setting", "only_mention"},
		{"role", "admin"},
		{"joined_at", "2026-08-05T14:19:09+08:00"},
		{"allow_proactive_msg", true},
		{"recv_msg_setting", "only_mention"},
		{"member_role", "admin"},
		{"member_openid", "bot-openid"},
	}
	for _, c := range checks {
		if v, ok := resp.Data[c.key]; !ok || v != c.want {
			t.Errorf("data[%s]: got %#v (存在=%v), want %#v", c.key, v, ok, c.want)
		}
	}
	if len(resp.Data) != 10 { // 9 字段 + group_id
		t.Errorf("data 字段数: got %d, keys=%v", len(resp.Data), resp.Data)
	}
	virtual, err := idmap.StoreIDv2(groupOpenID)
	if err != nil {
		t.Fatalf("StoreIDv2: %v", err)
	}
	if resp.Data["group_id"] != float64(virtual) {
		t.Errorf("group_id 应为虚拟群号 %d: got %#v", virtual, resp.Data["group_id"])
	}
}

// TestTesterVerify_JoinRequestCursorLimit cursor 优先 / next_index 回退 / limit 上限 50
func TestTesterVerify_JoinRequestCursorLimit(t *testing.T) {
	const groupOpenID = "cccccccccccccccccccccccccccccccc"
	cases := []struct {
		name       string
		cursor     string
		nextIndex  int
		limit      int
		wantCursor string
		wantLimit  int
	}{
		{"cursor 优先于 next_index", "c1", 7, 0, "c1", 20},
		{"cursor 优先且 limit 截断", "c2", 9, 200, "c2", 50},
		{"next_index 回退", "", 7, 0, "7", 20},
		{"两者皆空", "", 0, 0, "", 20},
		{"limit 恰好 50", "", 0, 50, "", 50},
		{"limit 51 截断", "", 0, 51, "", 50},
		{"limit 负数取默认", "", 0, -3, "", 20},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			mock := &tvMockAPI{listResp: &dto.JoinRequestList{NextCursor: "1785767153250497"}}
			_, err := GetGroupJoinRequestList(&tvClient{}, nil, mock, callapi.ActionMessage{
				Action: "get_group_join_request_list",
				Params: callapi.ParamsContent{GroupID: groupOpenID, Cursor: tc.cursor, NextIndex: tc.nextIndex, Limit: tc.limit},
			})
			if err != nil {
				t.Fatalf("handler 错误: %v", err)
			}
			if mock.gotCursor != tc.wantCursor || mock.gotLimit != tc.wantLimit {
				t.Fatalf("API 参数: got (cursor=%q, limit=%d), want (%q, %d)", mock.gotCursor, mock.gotLimit, tc.wantCursor, tc.wantLimit)
			}
		})
	}
}

// TestTesterVerify_JoinRequestListAliases list 与 join_requests 内容一致 + 字段映射
func TestTesterVerify_JoinRequestListAliases(t *testing.T) {
	const groupOpenID = "dddddddddddddddddddddddddddddddd"
	const memberOpenID = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
	mock := &tvMockAPI{listResp: &dto.JoinRequestList{
		List: []dto.JoinRequest{{
			JoinRequestID: "jr-1",
			MemberOpenID:  memberOpenID,
			Username:      "小明",
			ApplyAt:       "2026-08-05T14:19:09+08:00",
			ApplySource:   "self_apply",
			VerifyInfo:    dto.VerifyInfo{Method: "verify_message", Message: "几款看看"},
			AutoApproved:  &dto.AutoApprovedInfo{StrategyID: "s1", Approved: true},
		}},
		NextCursor: "1785767153250497",
	}}
	got, err := GetGroupJoinRequestList(&tvClient{}, nil, mock, callapi.ActionMessage{
		Action: "get_group_join_request_list",
		Params: callapi.ParamsContent{GroupID: groupOpenID},
	})
	if err != nil {
		t.Fatalf("handler 错误: %v", err)
	}
	var resp struct {
		Data struct {
			List         []map[string]interface{} `json:"list"`
			JoinRequests []map[string]interface{} `json:"join_requests"`
			NextCursor   string                   `json:"next_cursor"`
			NextIndex    int                      `json:"next_index"`
		} `json:"data"`
	}
	if err := json.Unmarshal([]byte(got), &resp); err != nil {
		t.Fatalf("响应解析失败: %v (%s)", err, got)
	}
	a, _ := json.Marshal(resp.Data.List)
	b, _ := json.Marshal(resp.Data.JoinRequests)
	if string(a) != string(b) {
		t.Errorf("list 与 join_requests 内容不一致:\n list=%s\n join_requests=%s", a, b)
	}
	if len(resp.Data.List) != 1 {
		t.Fatalf("条目数: got %d", len(resp.Data.List))
	}
	item := resp.Data.List[0]
	if item["flag"] != "jr-1" || item["username"] != "小明" || item["apply_at"] != "2026-08-05T14:19:09+08:00" {
		t.Errorf("条目字段映射: %#v", item)
	}
	if item["verify_info"] != "几款看看" || item["auto_approved"] != true {
		t.Errorf("verify_info/auto_approved: %#v", item)
	}
	virtual, err := idmap.StoreIDv2(memberOpenID)
	if err != nil {
		t.Fatalf("StoreIDv2: %v", err)
	}
	if item["user_id"] != float64(virtual) {
		t.Errorf("user_id 应为虚拟号 %d: got %#v", virtual, item["user_id"])
	}
	// 1785767153250497 超出 32 位可表示范围 → 兼容字段 next_index 置 0（32 位构建 Atoi 会溢出）
	if resp.Data.NextCursor != "1785767153250497" || resp.Data.NextIndex != 0 {
		t.Errorf("分页字段: cursor=%q index=%d (want cursor=1785767153250497, index=0)", resp.Data.NextCursor, resp.Data.NextIndex)
	}
}

// TestTesterVerify_SetGroupBanHandlerOps handler 路径的 op=add/del 与请求体
func TestTesterVerify_SetGroupBanHandlerOps(t *testing.T) {
	const groupOpenID = "ffffffffffffffffffffffffffffffff"
	const memberOpenID = "11111111111111111111111111111111"
	if err := idmap.WriteConfigv2(groupOpenID, "type", "group"); err != nil {
		t.Fatalf("WriteConfigv2: %v", err)
	}
	mock := &tvMockAPI{}
	client := &tvClient{}

	before := time.Now()
	got, err := SetGroupBan(client, nil, mock, callapi.ActionMessage{
		Action: "set_group_ban",
		Params: callapi.ParamsContent{GroupID: groupOpenID, UserID: memberOpenID, Duration: 600},
	})
	if err != nil {
		t.Fatalf("禁言 handler 错误: %v", err)
	}
	if !strings.Contains(got, `"retcode":0`) {
		t.Errorf("禁言应成功: %s", got)
	}
	if len(mock.setReqs) != 1 || len(mock.setReqs[0].Members) != 1 {
		t.Fatalf("应提交 1 条成员变更: %+v", mock.setReqs)
	}
	item := mock.setReqs[0].Members[0]
	if item.Op != "add" || item.MemberOpenID != memberOpenID {
		t.Errorf("禁言条目: %+v", item)
	}
	expire, perr := time.Parse(time.RFC3339, item.MuteExpireAt)
	if perr != nil {
		t.Fatalf("mute_expire_at 非 RFC3339: %q (%v)", item.MuteExpireAt, perr)
	}
	lo, hi := before.Add(600*time.Second-time.Second), time.Now().Add(600*time.Second+time.Second)
	if expire.Before(lo) || expire.After(hi) {
		t.Errorf("mute_expire_at 应≈now+600s: got %s, 期望区间 [%s, %s]", expire, lo, hi)
	}
	body, _ := json.Marshal(mock.setReqs[0])
	if strings.Contains(string(body), "global_rule") {
		t.Errorf("POST 体不应含 global_rule: %s", body)
	}

	if _, err := SetGroupBan(client, nil, mock, callapi.ActionMessage{
		Action: "set_group_ban",
		Params: callapi.ParamsContent{GroupID: groupOpenID, UserID: memberOpenID, Duration: 0},
	}); err != nil {
		t.Fatalf("解禁 handler 错误: %v", err)
	}
	item = mock.setReqs[1].Members[0]
	if item.Op != "del" || item.MuteExpireAt != "" {
		t.Errorf("解禁条目应为 op=del + 空串: %+v", item)
	}
	body, _ = json.Marshal(mock.setReqs[1])
	if !strings.Contains(string(body), `"mute_expire_at":""`) {
		t.Errorf("解禁请求体应显式携带空 mute_expire_at: %s", body)
	}
}

// TestTesterVerify_WholeBanNoWriteCall retcode 100 且不调用 SetRestrictChatSetting
func TestTesterVerify_WholeBanNoWriteCall(t *testing.T) {
	const groupOpenID = "22222222222222222222222222222222"
	if err := idmap.WriteConfigv2(groupOpenID, "type", "group"); err != nil {
		t.Fatalf("WriteConfigv2: %v", err)
	}
	for _, enable := range []bool{true, false} {
		mock := &tvMockAPI{}
		got, err := SetGroupWholeBan(&tvClient{}, nil, mock, callapi.ActionMessage{
			Action: "set_group_whole_ban",
			Params: callapi.ParamsContent{GroupID: groupOpenID, Enable: enable},
		})
		if err != nil {
			t.Fatalf("handler 错误: %v", err)
		}
		var resp struct {
			RetCode int    `json:"retcode"`
			Status  string `json:"status"`
			Message string `json:"message"`
		}
		if err := json.Unmarshal([]byte(got), &resp); err != nil {
			t.Fatalf("响应解析失败: %v (%s)", err, got)
		}
		if resp.RetCode != 100 || resp.Status != "failed" {
			t.Errorf("enable=%v 应返回失败回执: %+v", enable, resp)
		}
		if len(mock.setReqs) != 0 {
			t.Errorf("enable=%v 不应提交写入请求: %+v", enable, mock.setReqs)
		}
	}
}

// TestTesterVerify_Inbound103ContentPreserved message_type=103 正文与引用正文均不丢
func TestTesterVerify_Inbound103ContentPreserved(t *testing.T) {
	msg := &dto.WSGroupMessageData{
		Content:     "外层正文",
		MessageType: 103,
		MsgElements: []*dto.MsgElement{{MsgIdx: "REFIDX_r", MessageType: 0, Content: "被引用正文"}},
	}
	segs := ConvertToSegmentedMessage(msg)
	var texts []string
	for _, s := range segs {
		if s["type"] == "text" {
			texts = append(texts, tvSegData(t, s)["text"].(string))
		}
	}
	joined := strings.Join(texts, "|")
	if !strings.Contains(joined, "外层正文") || !strings.Contains(joined, "被引用正文") {
		t.Errorf("103 正文丢失: texts=%q segs=%+v", joined, segs)
	}
	reply := tvSeg(segs, "reply")
	if reply == nil || tvSegData(t, reply)["msg_idx"] != "REFIDX_r" {
		t.Errorf("reply 段: %+v", reply)
	}
}

// TestTesterVerify_ArkVoiceEdgeCases ark 空字段恒定键 / voice 回退与空 asr
func TestTesterVerify_ArkVoiceEdgeCases(t *testing.T) {
	// message_type=3 但 ark_data 缺失：仍产出 ark 段，四键恒定
	segs := ConvertToSegmentedMessage(&dto.WSGroupMessageData{MessageType: 3})
	ark := tvSeg(segs, "ark")
	if ark == nil {
		t.Fatalf("message_type=3 应产出 ark 段: %+v", segs)
	}
	data := tvSegData(t, ark)
	for _, k := range []string{"ark_type", "ark_name", "prompt", "fields"} {
		if _, ok := data[k]; !ok {
			t.Errorf("ark 段缺键 %s: %#v", k, data)
		}
	}
	if data["ark_type"] != "" || data["ark_name"] != "" {
		t.Errorf("空 ark_data 应给空串: %#v", data)
	}

	// voice：voice_wav_url 缺失回退 url；asr 为空不产出键
	segs = ConvertToSegmentedMessage(&dto.WSGroupMessageData{
		Attachments: []*dto.MessageAttachment{{URL: "https://x/b.silk", ContentType: "voice"}},
	})
	rec := tvSeg(segs, "record")
	if rec == nil {
		t.Fatalf("语音附件应产出 record 段: %+v", segs)
	}
	recData := tvSegData(t, rec)
	if recData["file"] != "https://x/b.silk" {
		t.Errorf("file 回退: %#v", recData)
	}
	if _, ok := recData["asr_refer_text"]; ok {
		t.Errorf("空 asr 不应产出键: %#v", recData)
	}

	// 非 voice 附件（image/）不产出 record 段
	segs = ConvertToSegmentedMessage(&dto.WSGroupMessageData{
		Attachments: []*dto.MessageAttachment{{URL: "https://x/a.png", FileName: "a.png", ContentType: "image/png"}},
	})
	if tvSeg(segs, "record") != nil {
		t.Errorf("图片附件不应产出 record 段: %+v", segs)
	}
}

// TestTesterVerify_RecordRegexBoundaries 管道级边界：asr 空值 / URL 自带逗号
func TestTesterVerify_RecordRegexBoundaries(t *testing.T) {
	cases := []struct {
		name     string
		input    string
		wantKey  string
		wantURL  string
		wantText string
	}{
		{"asr 空值仍剥离参数", "[CQ:record,file=https://x/a.wav,asr=]", "url_records", "x/a.wav", ""},
		{"URL 自带逗号完整保留", "[CQ:record,file=https://x/a,b.wav]", "url_records", "x/a,b.wav", ""},
		{"URL 自带逗号 + asr", "[CQ:record,file=https://x/a,b.wav,asr=你好]", "url_records", "x/a,b.wav", ""},
		{"正文保留", "听[CQ:record,file=https://x/a.wav,asr=你好]完", "url_records", "x/a.wav", "听完"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			found := map[string][]string{}
			out := ProcessCQCodePipeline(tc.input, found, nil)
			got := found[tc.wantKey]
			if len(got) != 1 || got[0] != tc.wantURL {
				t.Fatalf("%s: got %v, want [%s] (found=%v)", tc.wantKey, got, tc.wantURL, found)
			}
			if out != tc.wantText {
				t.Fatalf("正文: got %q, want %q", out, tc.wantText)
			}
		})
	}
}

// TestTesterVerify_LocalRecordAsrBranchDivergence 平台字面量差异 + asr 后缀一致性核查：
// cqcode.go 两分支的 localRecordPattern 均已带 `(?:,asr=[^\]]*)?`，差异只剩斜杠数量
// （Windows `file:///` 吃掉第三个斜杠，Unix `file://` 保留前导 /）。
func TestTesterVerify_LocalRecordAsrBranchDivergence(t *testing.T) {
	// 字面量与 cqcode.go:56 / cqcode.go:60 同步
	winLocal := regexp.MustCompile(`\[CQ:record,file=file:///([^\]]+?)(?:,asr=[^\]]*)?\]`)
	unixLocal := regexp.MustCompile(`\[CQ:record,file=file://([^\]]+?)(?:,asr=[^\]]*)?\]`)

	if m := winLocal.FindStringSubmatch("[CQ:record,file=file:///tmp/a.wav,asr=你好]"); len(m) < 2 || m[1] != "tmp/a.wav" {
		t.Fatalf("Windows 分支应剥离 asr 参数: got %#v", m)
	}
	if m := unixLocal.FindStringSubmatch("[CQ:record,file=file:///tmp/a.wav,asr=你好]"); len(m) < 2 || m[1] != "/tmp/a.wav" {
		t.Fatalf("Unix 分支应剥离 asr 参数并保留前导 /: got %#v", m)
	}

	// 本平台实际生效的分支：两平台都不再被 ,asr= 污染（cqcode.go:60 修复后的回归保护）
	compilePatternsOnce.Do(initPlatformPatterns)
	want := "/tmp/a.wav"
	if runtime.GOOS == "windows" {
		want = "tmp/a.wav"
	}
	if m := localRecordPattern.FindStringSubmatch("[CQ:record,file=file:///tmp/a.wav,asr=你好]"); len(m) < 2 || m[1] != want {
		t.Fatalf("localRecordPattern(%s) 捕获错误: got %#v, want %q（cqcode.go:56/60 字面量漂移？）", runtime.GOOS, m, want)
	}
}

// TestTesterVerify_UploadThresholdWiring 5MB 边界在 tryChunkedUpload 上的接线
func TestTesterVerify_UploadThresholdWiring(t *testing.T) {
	mock := &tvMockAPI{}
	exact := make([]byte, maxDirectFileDataBytes)
	if _, err := tryChunkedUpload(context.Background(), mock, "g", true, exact, 1, "a.png"); err == nil {
		t.Fatalf("恰好 5MB 应回退 URL 直传（返回错误提示）")
	} else if !strings.Contains(err.Error(), "5MB") {
		t.Errorf("回退提示应说明 5MB 上限: %v", err)
	}
	if mock.prepareCalls != 0 {
		t.Errorf("恰好 5MB 不应发起预上传: prepareCalls=%d", mock.prepareCalls)
	}
	if needsChunkedUpload(maxDirectFileDataBytes, 1) || !needsChunkedUpload(maxDirectFileDataBytes+1, 1) {
		t.Errorf("5MB 边界判据错误")
	}
}

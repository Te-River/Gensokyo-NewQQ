package handlers

import (
	"context"
	"errors"
	"fmt"
	"os"
	"strings"
	"testing"
	"time"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

// TestMain 将测试工作目录切换到全局共享临时目录：
// idmap 的 bbolt 数据库文件（idmap-identity.db 等）按相对路径在首次使用时
// 创建于 cwd，sync.Once 保证整个测试进程只初始化一次，因此所有测试必须
// 共享同一个目录（切换后恢复会使已打开的句柄失效）。测试内不使用 t.Parallel。
func TestMain(m *testing.M) {
	tmp, err := os.MkdirTemp("", "gensokyo-handlers-test")
	if err != nil {
		fmt.Fprintln(os.Stderr, "TestMain: 创建临时目录失败:", err)
		os.Exit(1)
	}
	if err := os.Chdir(tmp); err != nil {
		fmt.Fprintln(os.Stderr, "TestMain: 切换目录失败:", err)
		os.Exit(1)
	}
	code := m.Run()
	// 尽力清理：Windows 下 bbolt 句柄仍打开时 RemoveAll 可能部分失败，忽略
	_ = os.Chdir(os.TempDir())
	_ = os.RemoveAll(tmp)
	os.Exit(code)
}

// ---------- 共享 mock ----------

// mockGroupMemberOpenAPI 覆盖成员管理相关方法，其余方法走嵌入接口兜底 panic。
type mockGroupMemberOpenAPI struct {
	openapi.OpenAPI

	// GroupMemberList 按调用次序出队的固定脚本
	listPages  []*dto.GroupMemberList
	listErrs   []error
	listCalls  int
	listCursor []string // 记录每次调用的 cursor 参数

	// BatchRemoveMembers
	kickReqs   []*dto.BatchRemoveMembersRequest
	kickGroups []string
	kickResp   *dto.BatchRemoveMembersResponse
	kickErr    error

	// UpdateMemberBlacklist
	blReqs   []*dto.MemberBlacklistRequest
	blGroups []string
	blResp   *dto.MemberBlacklistResponse
	blErr    error
}

func (m *mockGroupMemberOpenAPI) GroupMemberList(ctx context.Context, groupOpenID, cursor string, limit int) (*dto.GroupMemberList, error) {
	m.listCalls++
	m.listCursor = append(m.listCursor, cursor)
	if m.listCalls <= len(m.listErrs) && m.listErrs[m.listCalls-1] != nil {
		return nil, m.listErrs[m.listCalls-1]
	}
	idx := m.listCalls
	if idx > len(m.listPages) {
		idx = len(m.listPages)
	}
	if idx == 0 {
		return &dto.GroupMemberList{}, nil
	}
	return m.listPages[idx-1], nil
}

func (m *mockGroupMemberOpenAPI) BatchRemoveMembers(ctx context.Context, groupOpenID string, req *dto.BatchRemoveMembersRequest) (*dto.BatchRemoveMembersResponse, error) {
	m.kickReqs = append(m.kickReqs, req)
	m.kickGroups = append(m.kickGroups, groupOpenID)
	if m.kickErr != nil {
		return nil, m.kickErr
	}
	return m.kickResp, nil
}

func (m *mockGroupMemberOpenAPI) UpdateMemberBlacklist(ctx context.Context, groupOpenID string, req *dto.MemberBlacklistRequest) (*dto.MemberBlacklistResponse, error) {
	m.blReqs = append(m.blReqs, req)
	m.blGroups = append(m.blGroups, groupOpenID)
	if m.blErr != nil {
		return nil, m.blErr
	}
	return m.blResp, nil
}

// groupMemberTestClient 记录 handler 发出的响应 payload
type groupMemberTestClient struct {
	response map[string]interface{}
}

func (c *groupMemberTestClient) SendMessage(message map[string]interface{}) error {
	c.response = message
	return nil
}

// openID32 构造一个确定性 32 位原生 OpenID（resolveMemberOpenID 直通分支，不查库）
func openID32(prefix byte) string {
	id := make([]byte, 32)
	for i := range id {
		id[i] = prefix
	}
	return string(id)
}

// ---------- 纯函数 ----------

// TestDedupeTruncateUserIDs 验证合并去重保序、空项过滤、超 20 截断
func TestDedupeTruncateUserIDs(t *testing.T) {
	tests := []struct {
		name string
		in   []string
		want []string
	}{
		{
			name: "21个唯一ID截断为20",
			in:   []string{"1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16", "17", "18", "19", "20", "21"},
			want: []string{"1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16", "17", "18", "19", "20"},
		},
		{
			name: "重复ID去重保序",
			in:   []string{"3", "1", "3", "2", "1"},
			want: []string{"3", "1", "2"},
		},
		{
			name: "空项被过滤",
			in:   []string{"", "5", "", "7"},
			want: []string{"5", "7"},
		},
		{
			name: "空输入返回空切片",
			in:   []string{},
			want: []string{},
		},
		{
			name: "nil输入返回空切片",
			in:   nil,
			want: []string{},
		},
		{
			name: "恰好20不截断",
			in:   []string{"1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16", "17", "18", "19", "20"},
			want: []string{"1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16", "17", "18", "19", "20"},
		},
		{
			name: "去重后不足20不误截",
			in:   []string{strings.Repeat("a", 32), strings.Repeat("a", 32), strings.Repeat("b", 32)},
			want: []string{strings.Repeat("a", 32), strings.Repeat("b", 32)},
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := dedupeTruncateUserIDs(tt.in)
			if len(got) != len(tt.want) {
				t.Fatalf("长度: got %d, want %d, got=%v", len(got), len(tt.want), got)
			}
			for i := range tt.want {
				if got[i] != tt.want[i] {
					t.Errorf("[%d]: got %q, want %q（顺序应保序）", i, got[i], tt.want[i])
				}
			}
		})
	}
}

// TestNormalizeMemberRole 验证官方 member_role 枚举直通与未知值保底
func TestNormalizeMemberRole(t *testing.T) {
	tests := []struct {
		name string
		in   string
		want string
	}{
		{"member直通", "member", "member"},
		{"owner直通", "owner", "owner"},
		{"admin直通", "admin", "admin"},
		{"未知值保底member", "moderator", "member"},
		{"空串保底member", "", "member"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := normalizeMemberRole(tt.in); got != tt.want {
				t.Errorf("normalizeMemberRole(%q) = %q, want %q", tt.in, got, tt.want)
			}
		})
	}
}

// TestParseRFC3339ToInt32 验证 RFC3339 解析与容错（空串/非法格式置 0）
func TestParseRFC3339ToInt32(t *testing.T) {
	valid := "2024-01-02T03:04:05+08:00"
	want := time.Date(2024, 1, 2, 3, 4, 5, 0, time.FixedZone("+0800", 8*3600)).Unix()
	tests := []struct {
		name  string
		value string
		want  int32
	}{
		{"合法RFC3339转Unix秒", valid, int32(want)},
		{"空串置0", "", 0},
		{"非法格式置0", "not-a-time", 0},
		{"纯日期缺时区置0", "2024-01-02", 0},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := parseRFC3339ToInt32(tt.value, "joined_at")
			if got != tt.want {
				t.Errorf("parseRFC3339ToInt32(%q) = %d, want %d", tt.value, got, tt.want)
			}
		})
	}
}

// TestResolveOpenIDList 验证面板对象列表任一反查失败整体报错
func TestResolveOpenIDList(t *testing.T) {
	t.Run("空列表返回nil无错误", func(t *testing.T) {
		got, err := resolveOpenIDList(nil, func(s string) (string, error) { return "x", nil })
		if err != nil || got != nil {
			t.Fatalf("got %v, err %v", got, err)
		}
	})
	t.Run("全部成功按序返回", func(t *testing.T) {
		got, err := resolveOpenIDList([]string{"a", "b"}, func(s string) (string, error) { return s + "-oid", nil })
		if err != nil || len(got) != 2 || got[0] != "a-oid" || got[1] != "b-oid" {
			t.Fatalf("got %v, err %v", got, err)
		}
	})
	t.Run("任一失败整体失败", func(t *testing.T) {
		_, err := resolveOpenIDList([]string{"ok", "bad", "never"}, func(s string) (string, error) {
			if s == "bad" {
				return "", errors.New("反查失败")
			}
			return s, nil
		})
		if err == nil {
			t.Fatal("期望整体返回错误")
		}
	})
}

// ---------- fetchMembersByAPI 三态 ----------

// TestFetchMembersByAPIFirstPageFallsBack 首页失败 → 回退本地缓存路径
func TestFetchMembersByAPIFirstPageFallsBack(t *testing.T) {
	mock := &mockGroupMemberOpenAPI{listErrs: []error{errors.New("11253 无权限")}}
	groupID := "123456" // 数字形态群号

	got := fetchMembersByAPI(mock, openID32('g'), groupID)

	// 空环境下 fallback 依赖的旧库不存在 → 返回空（与直接调用 fallback 行为一致）
	wantFallback := fetchFallbackMembers(groupID)
	if len(got) != len(wantFallback) {
		t.Fatalf("回退路径结果异常: got %d 项, 直接调用 fallback %d 项", len(got), len(wantFallback))
	}
	if mock.listCalls != 1 {
		t.Errorf("首页失败后不应继续翻页, calls = %d", mock.listCalls)
	}
}

// TestFetchMembersByAPIMidPageFailureKeepsPartial 中途页失败 → 返回已收集部分
func TestFetchMembersByAPIMidPageFailureKeepsPartial(t *testing.T) {
	mock := &mockGroupMemberOpenAPI{
		listPages: []*dto.GroupMemberList{
			{
				Members: []dto.GroupMember{
					{MemberOpenID: openID32('u'), Username: "成员A", MemberRole: "owner", JoinedAt: "2024-01-02T03:04:05+08:00"},
					{MemberOpenID: openID32('v'), Username: "成员B", MemberRole: "member"},
				},
				NextCursor: "page2",
			},
		},
		listErrs: []error{nil, errors.New("第2页网络失败")},
	}

	got := fetchMembersByAPI(mock, openID32('g'), "123456")

	if len(got) != 2 {
		t.Fatalf("应保留第1页已收集的 2 名成员, got %d", len(got))
	}
	if mock.listCalls != 2 {
		t.Errorf("应尝试拉取第 2 页, calls = %d", mock.listCalls)
	}
	if got[0].Nickname != "成员A" || got[0].Card != "成员A" {
		t.Errorf("成员A 昵称/名片未映射 username: %+v", got[0])
	}
	if got[0].Role != "owner" {
		t.Errorf("成员A role 应直通 owner, got %q", got[0].Role)
	}
	if got[1].Role != "member" {
		t.Errorf("成员B role 应直通 member, got %q", got[1].Role)
	}
	if got[0].UserID == 0 || got[1].UserID == 0 {
		t.Errorf("虚拟 user_id 不应为 0: %d / %d", got[0].UserID, got[1].UserID)
	}
	if got[0].UserID == got[1].UserID {
		t.Errorf("不同 openid 应映射到不同虚拟 ID")
	}
}

// TestFetchMembersByAPIEmptyListFallsBack 官方返回空列表 → 回退本地缓存路径
func TestFetchMembersByAPIEmptyListFallsBack(t *testing.T) {
	mock := &mockGroupMemberOpenAPI{
		listPages: []*dto.GroupMemberList{{Members: []dto.GroupMember{}}},
	}

	got := fetchMembersByAPI(mock, openID32('g'), "123456")

	wantFallback := fetchFallbackMembers("123456")
	if len(got) != len(wantFallback) {
		t.Fatalf("空列表应回退本地路径: got %d 项, fallback %d 项", len(got), len(wantFallback))
	}
	if mock.listCalls != 1 {
		t.Errorf("空列表只应拉取 1 页, calls = %d", mock.listCalls)
	}
}

// TestFetchMembersByAPIFullPagination 正常两页拉全量 + 字段映射
func TestFetchMembersByAPIFullPagination(t *testing.T) {
	mock := &mockGroupMemberOpenAPI{
		listPages: []*dto.GroupMemberList{
			{
				Members: []dto.GroupMember{
					{MemberOpenID: openID32('u'), Username: "成员A", MemberRole: "admin", JoinedAt: "2024-01-02T03:04:05+08:00"},
				},
				NextCursor: "page2",
			},
			{
				Members: []dto.GroupMember{
					{MemberOpenID: openID32('w'), Username: "成员C", MemberRole: "member", JoinedAt: "not-a-time"},
				},
			},
		},
	}

	got := fetchMembersByAPI(mock, openID32('g'), "123456")

	if len(got) != 2 {
		t.Fatalf("两页共 2 名成员, got %d", len(got))
	}
	if mock.listCalls != 2 {
		t.Errorf("应拉取 2 页, calls = %d", mock.listCalls)
	}
	// 第 1 页无 cursor 传入，第 2 页带第 1 页的 next_cursor
	if mock.listCursor[0] != "" || mock.listCursor[1] != "page2" {
		t.Errorf("cursor 传递异常: %v", mock.listCursor)
	}
	if got[0].Role != "admin" {
		t.Errorf("role 应直通 admin, got %q", got[0].Role)
	}
	if got[0].Sex != "unknown" {
		t.Errorf("官方无性别字段应为中性值 unknown, got %q", got[0].Sex)
	}
	wantJoin := int32(time.Date(2024, 1, 2, 3, 4, 5, 0, time.FixedZone("+0800", 8*3600)).Unix())
	if got[0].JoinTime != wantJoin {
		t.Errorf("joined_at 应转 Unix 秒: got %d, want %d", got[0].JoinTime, wantJoin)
	}
	// 非法时间容错：不中断列表,JoinTime 置 0
	if got[1].JoinTime != 0 {
		t.Errorf("非法 joined_at 应置 0: got %d", got[1].JoinTime)
	}
	if got[1].Nickname != "成员C" {
		t.Errorf("第 2 页成员 nickname: got %q, want 成员C", got[1].Nickname)
	}
}

// ---------- set_group_kick ----------

// TestSetGroupKickResolvesAndMapsAddBlacklist 验证 openID 反查、add_blacklist 映射与跳过不中断
func TestSetGroupKickResolvesAndMapsAddBlacklist(t *testing.T) {
	mock := &mockGroupMemberOpenAPI{
		kickResp: &dto.BatchRemoveMembersResponse{RemoveMembersResult: "success"},
	}
	client := &groupMemberTestClient{}

	msg := callapiActionMessage("set_group_kick", map[string]interface{}{
		"group_id":      openID32('g'),
		"user_id":       openID32('u'),
		"user_ids":      []string{openID32('v'), "100"}, // "100" 无映射,应跳过不中断
		"add_blacklist": true,
	})

	_, err := SetGroupKick(client, nil, mock, msg)
	if err != nil {
		t.Fatalf("SetGroupKick 返回错误: %v", err)
	}
	if len(mock.kickReqs) != 1 {
		t.Fatalf("应调用一次 BatchRemoveMembers, got %d", len(mock.kickReqs))
	}
	req := mock.kickReqs[0]
	if len(req.MemberOpenIDs) != 2 || req.MemberOpenIDs[0] != openID32('u') || req.MemberOpenIDs[1] != openID32('v') {
		t.Errorf("member_openids 应只含反查成功的 2 个 openid: %v", req.MemberOpenIDs)
	}
	if !req.AddToMemberBlacklist {
		t.Errorf("add_blacklist 应映射为 add_to_member_blacklist=true")
	}
	if mock.kickGroups[0] != openID32('g') {
		t.Errorf("group_openid 应直通: got %q", mock.kickGroups[0])
	}
	if rc := responseRetCode(t, client.response); rc != 0 || client.response["status"] != "ok" {
		t.Fatalf("成功响应异常: %v", client.response)
	}
	data, _ := client.response["data"].(map[string]interface{})
	if data == nil {
		t.Fatal("data 缺失")
	}
	if data["remove_members_result"] != "success" {
		t.Errorf("remove_members_result 应透传: got %v", data["remove_members_result"])
	}
	kicked, ok := data["kicked"].([]interface{})
	if !ok || len(kicked) != 2 {
		t.Errorf("kicked 应含 2 个成功反查的虚拟 ID: %v", data["kicked"])
	}
}

// TestSetGroupKickAllResolveFail 验证全部反查失败时返回 retcode 100 且不调用 API
func TestSetGroupKickAllResolveFail(t *testing.T) {
	mock := &mockGroupMemberOpenAPI{}
	client := &groupMemberTestClient{}

	msg := callapiActionMessage("set_group_kick", map[string]interface{}{
		"group_id": openID32('g'),
		"user_ids": []string{"100", "200"},
	})

	_, err := SetGroupKick(client, nil, mock, msg)
	if err != nil {
		t.Fatalf("SetGroupKick 返回错误: %v", err)
	}
	if len(mock.kickReqs) != 0 {
		t.Errorf("全部反查失败不应调用 API")
	}
	if rc := responseRetCode(t, client.response); rc != 100 || client.response["status"] != "failed" {
		t.Errorf("期望 retcode=100/failed, got %v", client.response)
	}
}

// TestSetGroupKickEmptyMembers 验证 user_id/user_ids 均为空时 retcode 100
func TestSetGroupKickEmptyMembers(t *testing.T) {
	mock := &mockGroupMemberOpenAPI{}
	client := &groupMemberTestClient{}

	msg := callapiActionMessage("set_group_kick", map[string]interface{}{"group_id": openID32('g')})

	_, _ = SetGroupKick(client, nil, mock, msg)
	if len(mock.kickReqs) != 0 {
		t.Errorf("不应调用 API")
	}
	if responseRetCode(t, client.response) != 100 {
		t.Errorf("期望 retcode=100, got %v", client.response)
	}
}

// ---------- set_group_member_blacklist ----------

// TestSetGroupMemberBlacklistOpValidation 验证 op 非法直接失败且不查群
func TestSetGroupMemberBlacklistOpValidation(t *testing.T) {
	mock := &mockGroupMemberOpenAPI{}
	client := &groupMemberTestClient{}

	msg := callapiActionMessage("set_group_member_blacklist", map[string]interface{}{
		"group_id": openID32('g'),
		"op":       "clear",
		"user_ids": []string{openID32('u')},
	})

	_, _ = SetGroupMemberBlacklist(client, nil, mock, msg)
	if len(mock.blReqs) != 0 {
		t.Errorf("op 非法不应调用 API")
	}
	if responseRetCode(t, client.response) != 100 {
		t.Errorf("期望 retcode=100, got %v", client.response)
	}
}

// TestSetGroupMemberBlacklistAddMapping 验证 add op 请求体映射与 fail_openids 透传
func TestSetGroupMemberBlacklistAddMapping(t *testing.T) {
	mock := &mockGroupMemberOpenAPI{
		blResp: &dto.MemberBlacklistResponse{FailOpenids: []string{openID32('v')}},
	}
	client := &groupMemberTestClient{}

	msg := callapiActionMessage("set_group_member_blacklist", map[string]interface{}{
		"group_id": openID32('g'),
		"op":       "add",
		"user_id":  openID32('u'),
		"user_ids": []string{openID32('v'), "100"}, // "100" 跳过
	})

	_, err := SetGroupMemberBlacklist(client, nil, mock, msg)
	if err != nil {
		t.Fatalf("SetGroupMemberBlacklist 返回错误: %v", err)
	}
	if len(mock.blReqs) != 1 {
		t.Fatalf("应调用一次 UpdateMemberBlacklist, got %d", len(mock.blReqs))
	}
	req := mock.blReqs[0]
	if req.Op != "add" {
		t.Errorf("op 应为 add, got %q", req.Op)
	}
	if len(req.MemberOpenIDs) != 2 || req.MemberOpenIDs[0] != openID32('u') || req.MemberOpenIDs[1] != openID32('v') {
		t.Errorf("member_openids 应为反查成功的 2 个: %v", req.MemberOpenIDs)
	}
	data, _ := client.response["data"].(map[string]interface{})
	if data == nil {
		t.Fatal("data 缺失")
	}
	fails, ok := data["fail_openids"].([]interface{})
	if !ok || len(fails) != 1 || fails[0] != openID32('v') {
		t.Errorf("fail_openids 应透传官方响应: %v", data["fail_openids"])
	}
}

// TestSetGroupMemberBlacklistDelOp 验证 del op 映射
func TestSetGroupMemberBlacklistDelOp(t *testing.T) {
	mock := &mockGroupMemberOpenAPI{
		blResp: &dto.MemberBlacklistResponse{},
	}
	client := &groupMemberTestClient{}

	msg := callapiActionMessage("set_group_member_blacklist", map[string]interface{}{
		"group_id": openID32('g'),
		"op":       "del",
		"user_ids": []string{openID32('u')},
	})

	_, _ = SetGroupMemberBlacklist(client, nil, mock, msg)
	if len(mock.blReqs) != 1 || mock.blReqs[0].Op != "del" {
		t.Fatalf("op 应映射为 del: %+v", mock.blReqs)
	}
	if responseRetCode(t, client.response) != 0 {
		t.Errorf("期望 retcode=0, got %v", client.response)
	}
}

// ---------- 测试辅助 ----------

// responseRetCode 从 client mock 捕获的响应中取 retcode。
// structToMap 经 JSON 往返,数字均为 float64,需类型宽容比较。
func responseRetCode(t *testing.T, response map[string]interface{}) int {
	t.Helper()
	v, ok := response["retcode"]
	if !ok {
		t.Fatal("响应缺少 retcode")
	}
	switch n := v.(type) {
	case float64:
		return int(n)
	case int:
		return n
	default:
		t.Fatalf("retcode 类型异常: %T (%v)", v, v)
		return 0
	}
}

// callapiActionMessage 按 CQ 路径同款参数构造 ActionMessage（data → Params）
func callapiActionMessage(action string, data map[string]interface{}) callapi.ActionMessage {
	params := callapi.ParamsContent{}
	if v, ok := data["group_id"].(string); ok {
		params.GroupID = v
	}
	if v, ok := data["user_id"].(string); ok {
		params.UserID = v
	}
	if v, ok := data["user_ids"].([]string); ok {
		params.UserIDs = v
	}
	if v, ok := data["add_blacklist"].(bool); ok {
		params.AddBlacklist = v
	}
	if v, ok := data["op"].(string); ok {
		params.Op = v
	}
	if v, ok := data["scope"].(string); ok {
		params.Scope = v
	}
	if v, ok := data["target_type"].(string); ok {
		params.TargetType = v
	}
	if v, ok := data["panel_id"].(string); ok {
		params.PanelID = v
	}
	if v, ok := data["user_openids"].([]string); ok {
		params.UserOpenIDs = v
	}
	if v, ok := data["group_openids"].([]string); ok {
		params.GroupOpenIDs = v
	}
	if v, ok := data["panel"]; ok {
		params.Panel = v
	}
	if v, ok := data["menu"]; ok {
		params.Menu = v
	}
	return callapi.ActionMessage{Action: action, Params: params, Echo: "test-echo"}
}

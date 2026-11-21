package handlers

// join_approval_strategy 契约测试：插件侧只见虚拟号，调上游必须用真实 group_openid。
// 改前：group_openids / group_ids 原样上行，虚拟群号（如 499967395）被当 openid 发给官方；
// 改后：32 位 openid 直通，虚拟号按 idmap 反查，反查失败回 1400 并指明是哪个值。

import (
	"context"
	"strconv"
	"strings"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

const (
	joinApprovalGroupOpenID = "31AB82CE7526BBFE4B1002C4B258FD0E"
	joinApprovalOtherOpenID = "B1F7E0A2C93D4E5F8091A2B3C4D5E6F7"
)

// joinApprovalMockAPI 记录创建/修改策略的请求体，其余方法走嵌入接口兜底。
type joinApprovalMockAPI struct {
	openapi.OpenAPI
	createReq *dto.CreateJoinApprovalStrategyRequest
	updateReq *dto.UpdateJoinApprovalStrategyRequest
	updateID  string
}

func (m *joinApprovalMockAPI) CreateJoinApprovalStrategy(_ context.Context, req *dto.CreateJoinApprovalStrategyRequest) (*dto.CreateJoinApprovalStrategyResponse, error) {
	m.createReq = req
	return &dto.CreateJoinApprovalStrategyResponse{StrategyID: "strategy-1"}, nil
}

func (m *joinApprovalMockAPI) UpdateJoinApprovalStrategy(_ context.Context, strategyID string, req *dto.UpdateJoinApprovalStrategyRequest) (*dto.UpdateJoinApprovalStrategyResponse, error) {
	m.updateID = strategyID
	m.updateReq = req
	return &dto.UpdateJoinApprovalStrategyResponse{IsEnable: "on"}, nil
}

// joinApprovalVirtualGroupID 预置 真实群 OpenID → 虚拟群号 映射并返回虚拟群号。
func joinApprovalVirtualGroupID(t *testing.T, openID string) uint64 {
	t.Helper()
	virtual, err := idmap.StoreIDv2(openID)
	if err != nil {
		t.Fatalf("StoreIDv2(%s): %v", openID, err)
	}
	return uint64(virtual)
}

// TestCreateJoinApprovalStrategyResolvesVirtualGroupIDs group_ids 里的虚拟群号必须还原成 openid 上行。
func TestCreateJoinApprovalStrategyResolvesVirtualGroupIDs(t *testing.T) {
	virtual := joinApprovalVirtualGroupID(t, joinApprovalGroupOpenID)
	mock := &joinApprovalMockAPI{}
	client := &contractTestClient{}

	raw, err := CreateJoinApprovalStrategy(client, nil, mock, callapi.ActionMessage{
		Action: "join_approval_strategy_create",
		Params: callapi.ParamsContent{GroupIDs: []uint64{virtual}},
		Echo:   "jas-create-ids",
	})
	if err != nil {
		t.Fatalf("CreateJoinApprovalStrategy: %v", err)
	}
	if mock.createReq == nil {
		t.Fatalf("未调用上游: raw=%s", raw)
	}
	if len(mock.createReq.GroupOpenIDs) != 1 || mock.createReq.GroupOpenIDs[0] != joinApprovalGroupOpenID {
		t.Fatalf("上游 group_openids = %v（虚拟群号被直接上行），want [%s]", mock.createReq.GroupOpenIDs, joinApprovalGroupOpenID)
	}
	if len(mock.createReq.GroupIDs) != 0 {
		t.Errorf("上游 group_ids 应为空（已并入 group_openids）: %v", mock.createReq.GroupIDs)
	}
	if client.last["retcode"] != float64(0) || client.last["status"] != "ok" {
		t.Errorf("回执应为成功: %v", client.last)
	}
}

// TestCreateJoinApprovalStrategyResolvesVirtualGroupOpenIDs 插件把虚拟群号填进 group_openids 时同样反查。
func TestCreateJoinApprovalStrategyResolvesVirtualGroupOpenIDs(t *testing.T) {
	virtual := joinApprovalVirtualGroupID(t, joinApprovalOtherOpenID)
	mock := &joinApprovalMockAPI{}

	if _, err := CreateJoinApprovalStrategy(&contractTestClient{}, nil, mock, callapi.ActionMessage{
		Action: "join_approval_strategy_create",
		Params: callapi.ParamsContent{GroupOpenIDs: callapi.StringList{strconv.FormatUint(virtual, 10)}},
	}); err != nil {
		t.Fatalf("CreateJoinApprovalStrategy: %v", err)
	}
	if mock.createReq == nil {
		t.Fatal("未调用上游")
	}
	if len(mock.createReq.GroupOpenIDs) != 1 || mock.createReq.GroupOpenIDs[0] != joinApprovalOtherOpenID {
		t.Fatalf("上游 group_openids = %v, want [%s]", mock.createReq.GroupOpenIDs, joinApprovalOtherOpenID)
	}
}

// TestCreateJoinApprovalStrategyPassesThroughOpenID 32 位原生 openid 直通，不反查。
func TestCreateJoinApprovalStrategyPassesThroughOpenID(t *testing.T) {
	mock := &joinApprovalMockAPI{}

	if _, err := CreateJoinApprovalStrategy(&contractTestClient{}, nil, mock, callapi.ActionMessage{
		Action: "join_approval_strategy_create",
		Params: callapi.ParamsContent{GroupOpenIDs: callapi.StringList{joinApprovalGroupOpenID}},
	}); err != nil {
		t.Fatalf("CreateJoinApprovalStrategy: %v", err)
	}
	if mock.createReq == nil {
		t.Fatal("未调用上游")
	}
	if len(mock.createReq.GroupOpenIDs) != 1 || mock.createReq.GroupOpenIDs[0] != joinApprovalGroupOpenID {
		t.Fatalf("上游 group_openids = %v, want 原样 [%s]", mock.createReq.GroupOpenIDs, joinApprovalGroupOpenID)
	}
}

// TestCreateJoinApprovalStrategyUnmappedGroupRepliesParam 查不到的虚拟群号 → 1400，且不调用上游。
func TestCreateJoinApprovalStrategyUnmappedGroupRepliesParam(t *testing.T) {
	mock := &joinApprovalMockAPI{}
	client := &contractTestClient{}

	raw, err := CreateJoinApprovalStrategy(client, nil, mock, callapi.ActionMessage{
		Action: "join_approval_strategy_create",
		Params: callapi.ParamsContent{GroupIDs: []uint64{499967395}},
		Echo:   "jas-unmapped",
	})
	if err != nil {
		t.Fatalf("CreateJoinApprovalStrategy response: %v", err)
	}
	assertFailedReceipt(t, client, raw, sendParamRetCode, "jas-unmapped")
	if mock.createReq != nil {
		t.Errorf("反查失败不应调用上游: %+v", mock.createReq)
	}
	if msg, _ := client.last["message"].(string); !strings.Contains(msg, "499967395") {
		t.Errorf("message 应指明是哪个群反查失败: %v", client.last["message"])
	}
}

// TestUpdateJoinApprovalStrategyResolvesVirtualGroupIDs 关联群增删同样按虚拟号 → openid 上行。
func TestUpdateJoinApprovalStrategyResolvesVirtualGroupIDs(t *testing.T) {
	virtual := joinApprovalVirtualGroupID(t, joinApprovalGroupOpenID)
	mock := &joinApprovalMockAPI{}

	if _, err := UpdateJoinApprovalStrategy(&contractTestClient{}, nil, mock, callapi.ActionMessage{
		Action: "join_approval_strategy_update",
		Params: callapi.ParamsContent{StrategyID: "strategy-1", Op: "add", GroupIDs: []uint64{virtual}},
	}); err != nil {
		t.Fatalf("UpdateJoinApprovalStrategy: %v", err)
	}
	if mock.updateReq == nil || mock.updateReq.GroupAction == nil {
		t.Fatalf("未构造 group_action: %+v", mock.updateReq)
	}
	action := mock.updateReq.GroupAction
	if action.Op != "add" || len(action.GroupOpenIDs) != 1 || action.GroupOpenIDs[0] != joinApprovalGroupOpenID {
		t.Fatalf("上游 group_action = %+v, want op=add / group_openids=[%s]", action, joinApprovalGroupOpenID)
	}
	if len(action.GroupIDs) != 0 {
		t.Errorf("上游 group_ids 应为空（已并入 group_openids）: %v", action.GroupIDs)
	}
}

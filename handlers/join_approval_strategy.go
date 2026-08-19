package handlers

import (
	"context"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

func init() {
	callapi.RegisterHandler("join_approval_strategy_create", CreateJoinApprovalStrategy)
	callapi.RegisterHandler("join_approval_strategy_list", ListJoinApprovalStrategy)
	callapi.RegisterHandler("join_approval_strategy_update", UpdateJoinApprovalStrategy)
	callapi.RegisterHandler("join_approval_strategy_execute", ExecuteJoinApprovalStrategy)
	callapi.RegisterHandler("join_approval_strategy_whitelist", UpdateJoinApprovalStrategyWhitelist)
	callapi.RegisterHandler("join_approval_strategy_delete", DeleteJoinApprovalStrategy)
}

// CreateJoinApprovalStrategy 创建入群自动审批策略
// params: group_openids/group_ids(二选一必填), is_enable(on/off 默认 on), expire_at(RFC3339), remark
func CreateJoinApprovalStrategy(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	req := &dto.CreateJoinApprovalStrategyRequest{
		GroupOpenIDs: message.Params.GroupOpenIDs,
		GroupIDs:     message.Params.GroupIDs,
		IsEnable:     message.Params.IsEnable,
		ExpireAt:     message.Params.ExpireAt,
		Remark:       message.Params.Remark,
	}
	resp, err := apiv2.CreateJoinApprovalStrategy(context.TODO(), req)
	if err != nil {
		mylog.Printf("join_approval_strategy_create: 创建失败: %v", err)
		return sendActionResult(client, message, err.Error(), 100)
	}
	return sendActionResultWithData(client, message, resp)
}

// ListJoinApprovalStrategy 查询入群自动审批策略列表
// params: cursor(可选分页游标), limit(可选单页数量 默认20 最大100)
func ListJoinApprovalStrategy(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	resp, err := apiv2.ListJoinApprovalStrategy(context.TODO(), message.Params.Cursor, message.Params.Limit)
	if err != nil {
		mylog.Printf("join_approval_strategy_list: 查询失败: %v", err)
		return sendActionResult(client, message, err.Error(), 100)
	}
	return sendActionResultWithData(client, message, resp)
}

// UpdateJoinApprovalStrategy 修改入群自动审批策略
// params: strategy_id, is_enable/expire_at/remark 可选, op(add/del)+group_openids/group_ids 用于增删关联群
func UpdateJoinApprovalStrategy(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	req := &dto.UpdateJoinApprovalStrategyRequest{
		IsEnable: message.Params.IsEnable,
		ExpireAt: message.Params.ExpireAt,
		Remark:   message.Params.Remark,
	}
	if message.Params.Op != "" {
		req.GroupAction = &dto.GroupAction{
			Op:           message.Params.Op,
			GroupOpenIDs: message.Params.GroupOpenIDs,
			GroupIDs:     message.Params.GroupIDs,
		}
	}
	resp, err := apiv2.UpdateJoinApprovalStrategy(context.TODO(), message.Params.StrategyID, req)
	if err != nil {
		mylog.Printf("join_approval_strategy_update: 修改失败: %v", err)
		return sendActionResult(client, message, err.Error(), 100)
	}
	return sendActionResultWithData(client, message, resp)
}

// ExecuteJoinApprovalStrategy 执行入群自动审批策略（对关联群全量扫描, 异步约10分钟）
// params: strategy_id
func ExecuteJoinApprovalStrategy(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	if err := apiv2.ExecuteJoinApprovalStrategy(context.TODO(), message.Params.StrategyID); err != nil {
		mylog.Printf("join_approval_strategy_execute: 执行失败: %v", err)
		return sendActionResult(client, message, err.Error(), 100)
	}
	return sendActionResultWithData(client, message, nil)
}

// UpdateJoinApprovalStrategyWhitelist 修改入群自动审批策略白名单号码
// params: strategy_id, op(add/del), whitelist_users(QQ号码列表 单次最多10000个)
func UpdateJoinApprovalStrategyWhitelist(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	req := &dto.WhitelistUsersRequest{
		Op:             message.Params.Op,
		WhitelistUsers: message.Params.WhitelistUsers,
	}
	resp, err := apiv2.UpdateJoinApprovalStrategyWhitelist(context.TODO(), message.Params.StrategyID, req)
	if err != nil {
		mylog.Printf("join_approval_strategy_whitelist: 修改白名单失败: %v", err)
		return sendActionResult(client, message, err.Error(), 100)
	}
	return sendActionResultWithData(client, message, resp)
}

// DeleteJoinApprovalStrategy 删除入群自动审批策略
// params: strategy_id
func DeleteJoinApprovalStrategy(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	if err := apiv2.DeleteJoinApprovalStrategy(context.TODO(), message.Params.StrategyID); err != nil {
		mylog.Printf("join_approval_strategy_delete: 删除失败: %v", err)
		return sendActionResult(client, message, err.Error(), 100)
	}
	return sendActionResultWithData(client, message, nil)
}

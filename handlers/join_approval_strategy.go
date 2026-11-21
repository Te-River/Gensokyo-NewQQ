package handlers

import (
	"context"
	"fmt"
	"strconv"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/internal/domain/identity"
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
//
// 插件侧只见虚拟号：两个群字段都可能装虚拟群号，统一还原为上游群标识 group_openid
// （32 位原生 openid 直通），任一反查失败回 1400 并指明是哪个值。
func CreateJoinApprovalStrategy(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	groupOpenIDs, err := resolveJoinApprovalGroupOpenIDs(message.Params.GroupOpenIDs, message.Params.GroupIDs)
	if err != nil {
		mylog.Printf("join_approval_strategy_create: %v", err)
		return sendActionResult(client, message, err.Error(), sendParamRetCode)
	}
	req := &dto.CreateJoinApprovalStrategyRequest{
		GroupOpenIDs: groupOpenIDs,
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
		groupOpenIDs, err := resolveJoinApprovalGroupOpenIDs(message.Params.GroupOpenIDs, message.Params.GroupIDs)
		if err != nil {
			mylog.Printf("join_approval_strategy_update: %v", err)
			return sendActionResult(client, message, err.Error(), sendParamRetCode)
		}
		req.GroupAction = &dto.GroupAction{
			Op:           message.Params.Op,
			GroupOpenIDs: groupOpenIDs,
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

// resolveJoinApprovalGroupOpenIDs 把插件侧的关联群（group_openids / group_ids）统一还原为
// 上游群标识 group_openid 列表。官方 dto 的 group_ids 是数值群号，而 Gensokyo 的虚拟群号
// 不是真实群号、bot API 也拿不到真实群号，故两个字段都按「32 位 openid 直通 / 否则 idmap
// 反查」处理，结果一并填进请求的 group_openids 上行（官方要求两字段二选一）。
func resolveJoinApprovalGroupOpenIDs(groupOpenIDs []string, groupIDs []uint64) ([]string, error) {
	openIDs, err := resolveOpenIDList(groupOpenIDs, func(value string) (string, error) {
		return resolveJoinApprovalGroupOpenID(value, "group_openids")
	})
	if err != nil {
		return nil, err
	}

	values := make([]string, 0, len(groupIDs))
	for _, id := range groupIDs {
		values = append(values, strconv.FormatUint(id, 10))
	}
	resolvedIDs, err := resolveOpenIDList(values, func(value string) (string, error) {
		return resolveJoinApprovalGroupOpenID(value, "group_ids")
	})
	if err != nil {
		return nil, err
	}
	return append(openIDs, resolvedIDs...), nil
}

// resolveJoinApprovalGroupOpenID 单个群标识还原为群 OpenID（32 位原生 OpenID 直通），
// 错误信息带上字段名与取值，便于定位插件传错了哪个群。
func resolveJoinApprovalGroupOpenID(value, field string) (string, error) {
	if identity.IsOpenID(value) {
		return value, nil
	}
	openID, err := idmap.RetrieveRowByIDv2(value)
	if err != nil {
		return "", fmt.Errorf("%s=%s 无法反查群 OpenID: %w", field, value, err)
	}
	if openID == "" {
		return "", fmt.Errorf("%s=%s 无法反查群 OpenID", field, value)
	}
	return openID, nil
}

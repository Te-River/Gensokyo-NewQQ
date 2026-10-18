package handlers

import (
	"context"
	"math"
	"strconv"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

func init() {
	callapi.RegisterHandler("get_group_join_request_list", GetGroupJoinRequestList)
}

// joinRequestDefaultLimit / joinRequestMaxLimit 官方单页数量默认与上限
const (
	joinRequestDefaultLimit = 20
	joinRequestMaxLimit     = 50
)

// JoinRequestItem OneBot 入群申请条目
// group_id/user_id/flag 与 ProcessGroupJoinRequest 上报的 request 事件一致, 可直接回传 set_group_add_request 审批
type JoinRequestItem struct {
	GroupID      int64          `json:"group_id"`
	UserID       int64          `json:"user_id"`
	Flag         string         `json:"flag"`
	Username     string         `json:"username"`
	ApplyAt      string         `json:"apply_at"` // RFC3339（官方为字符串；旧版本此处为秒级时间戳 int）
	ApplySource  string         `json:"apply_source"`
	InvitedBy    string         `json:"invited_by"`
	UnionOpenID  string         `json:"union_openid"`
	Bot          bool           `json:"bot"`
	RiskTips     string         `json:"risk_tips"`
	VerifyInfo   string         `json:"verify_info"`
	VerifyMethod string         `json:"verify_method"` // 官方 verify_info.method 原样透传
	ReviewQAList []dto.ReviewQA `json:"review_qa_list"`
	AutoApproved bool           `json:"auto_approved"` // 官方列表项无此字段，由 auto_approved 对象是否存在推导
}

// JoinRequestListResponse 入群申请列表响应
// join_requests 为 list 的兼容别名（数组内容相同），next_index 为 next_cursor 的兼容别名
// （仅当 next_cursor 可解析为 int64 且不超过 math.MaxInt32 时取其值，否则 0）
type JoinRequestListResponse struct {
	List         []JoinRequestItem `json:"list"`
	JoinRequests []JoinRequestItem `json:"join_requests"`
	NextCursor   string            `json:"next_cursor"`
	NextIndex    int               `json:"next_index"`
}

// resolveJoinRequestCursor 优先取官方 cursor（string），缺失时回退旧 next_index（int 转字符串）
func resolveJoinRequestCursor(params callapi.ParamsContent) string {
	if params.Cursor != "" {
		return params.Cursor
	}
	if params.NextIndex > 0 {
		return strconv.Itoa(params.NextIndex)
	}
	return ""
}

// resolveJoinRequestLimit 单页数量：未传/<=0 用官方默认 20，超过上限 50 截断
func resolveJoinRequestLimit(limit int) int {
	if limit <= 0 {
		return joinRequestDefaultLimit
	}
	if limit > joinRequestMaxLimit {
		return joinRequestMaxLimit
	}
	return limit
}

// GetGroupJoinRequestList 拉取入群申请列表
// params: group_id(虚拟/真实群ID), cursor(官方分页游标, 优先) / next_index(旧字段, cursor 为空时回退), limit(单页数量, 默认20 最大50)
func GetGroupJoinRequestList(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	groupID := message.Params.GroupID.(string)

	// 反查真实 OpenID（32 位原生 OpenID 直接使用）
	groupOpenID := groupID
	if len(groupID) != 32 {
		realGroupID, err := idmap.RetrieveRowByIDv2(groupID)
		if err != nil || realGroupID == "" {
			mylog.Printf("get_group_join_request_list: 反查 group_openid 失败: %v", err)
			return sendActionResult(client, message, "无法反查群 OpenID", 100)
		}
		groupOpenID = realGroupID
	}

	cursor := resolveJoinRequestCursor(message.Params)
	list, err := apiv2.JoinRequestList(context.TODO(), groupOpenID, cursor, resolveJoinRequestLimit(message.Params.Limit))
	if err != nil {
		mylog.Printf("get_group_join_request_list: 拉取申请列表失败: %v", err)
		return sendActionResult(client, message, err.Error(), 100)
	}

	groupID64, _ := idmap.StoreIDv2(groupOpenID)
	items := make([]JoinRequestItem, 0, len(list.List))
	for _, req := range list.List {
		// 申请者 openid → 虚拟 user_id（与入站 request 事件上报一致, 审批时可直接回传）
		userID, _ := idmap.StoreIDv2(req.MemberOpenID)
		items = append(items, JoinRequestItem{
			GroupID:      groupID64,
			UserID:       userID,
			Flag:         req.JoinRequestID,
			Username:     req.Username,
			ApplyAt:      req.ApplyAt,
			ApplySource:  req.ApplySource,
			InvitedBy:    req.InvitedBy,
			UnionOpenID:  req.UnionOpenID,
			Bot:          req.Bot,
			RiskTips:     req.RiskTips,
			VerifyInfo:   req.VerifyInfo.String(),
			VerifyMethod: req.VerifyInfo.Method,
			ReviewQAList: req.VerifyInfo.ReviewQAList,
			AutoApproved: req.AutoApproved != nil && req.AutoApproved.Approved,
		})
	}
	// 旧 next_index 兼容：官方 next_cursor 为字符串（实测 16 位十进制，如 1785767153250497），
	// 仅当可解析且落在 32 位可表示范围内才同步给旧字段；否则置 0 并提示改用 next_cursor，
	// 避免旧插件按 next_index != 0 翻页时静默只拿第一页
	nextIndex := 0
	if list.NextCursor != "" {
		if v, convErr := strconv.ParseInt(list.NextCursor, 10, 64); convErr == nil && v <= math.MaxInt32 {
			nextIndex = int(v)
		} else {
			mylog.Printf("get_group_join_request_list: 官方 next_cursor=%s 非 32 位可表示值, 兼容字段 next_index 置 0, 请改用 next_cursor 翻页", list.NextCursor)
		}
	}
	resp := JoinRequestListResponse{
		List:         items,
		JoinRequests: items,
		NextCursor:   list.NextCursor,
		NextIndex:    nextIndex,
	}
	mylog.Printf("get_group_join_request_list: group=%s 共 %d 条申请", groupOpenID, len(items))
	return sendActionResultWithData(client, message, resp)
}

package handlers

import (
	"context"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/openapi"
)

func init() {
	callapi.RegisterHandler("get_group_join_request_list", GetGroupJoinRequestList)
}

// JoinRequestItem OneBot 入群申请条目
// group_id/user_id/flag 与 ProcessGroupJoinRequest 上报的 request 事件一致, 可直接回传 set_group_add_request 审批
type JoinRequestItem struct {
	GroupID      int64  `json:"group_id"`
	UserID       int64  `json:"user_id"`
	Flag         string `json:"flag"`
	Username     string `json:"username"`
	ApplyAt      int64  `json:"apply_at"`
	ApplySource  string `json:"apply_source"`
	InvitedBy    string `json:"invited_by"`
	VerifyInfo   string `json:"verify_info"`
	AutoApproved bool   `json:"auto_approved"`
}

// JoinRequestListResponse 入群申请列表响应
type JoinRequestListResponse struct {
	JoinRequests []JoinRequestItem `json:"join_requests"`
	NextIndex    int               `json:"next_index"`
}

// GetGroupJoinRequestList 拉取入群申请列表
// params: group_id(虚拟/真实群ID), next_index(可选分页游标)
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

	list, err := apiv2.JoinRequestList(context.TODO(), groupOpenID, message.Params.NextIndex)
	if err != nil {
		mylog.Printf("get_group_join_request_list: 拉取申请列表失败: %v", err)
		return sendActionResult(client, message, err.Error(), 100)
	}

	resp := JoinRequestListResponse{NextIndex: list.NextIndex}
	for _, req := range list.JoinRequests {
		// 申请者 openid → 虚拟 user_id（与入站 request 事件上报一致, 审批时可直接回传）
		groupID64, _ := idmap.StoreIDv2(req.GroupOpenID)
		userID, _ := idmap.StoreIDv2(req.MemberOpenID)
		resp.JoinRequests = append(resp.JoinRequests, JoinRequestItem{
			GroupID:      groupID64,
			UserID:       userID,
			Flag:         req.JoinRequestID,
			Username:     req.Username,
			ApplyAt:      req.ApplyAt,
			ApplySource:  req.ApplySource,
			InvitedBy:    req.InvitedBy,
			VerifyInfo:   req.VerifyInfo,
			AutoApproved: req.AutoApproved,
		})
	}
	mylog.Printf("get_group_join_request_list: group=%s 共 %d 条申请", groupOpenID, len(resp.JoinRequests))
	return sendActionResultWithData(client, message, resp)
}

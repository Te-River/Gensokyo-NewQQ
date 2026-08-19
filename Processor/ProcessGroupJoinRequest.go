package Processor

import (
	"strconv"
	"time"

	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/dto"
)

// ProcessGroupJoinRequest 处理用户申请加群事件
// 上报 OneBot request 事件（request_type=group, sub_type=add），flag 为 join_request_id 供 set_group_add_request 审批使用
func (p *Processors) ProcessGroupJoinRequest(data *dto.GroupJoinRequestEvent) {
	if data == nil {
		mylog.Printf("ProcessGroupJoinRequest: 数据为空")
		return
	}

	// 将 group_openid 转为虚拟 group_id
	groupID, err := idmap.StoreIDv2(data.GroupOpenID)
	if err != nil {
		mylog.Printf("ProcessGroupJoinRequest: group_id 转换失败: %v", err)
		return
	}

	// 将 member_openid 转为虚拟 user_id（申请入群的用户）
	userID, err := idmap.StoreIDv2(data.MemberOpenID)
	if err != nil {
		mylog.Printf("ProcessGroupJoinRequest: user_id 转换失败: %v", err)
		return
	}
	mylog.Printf("[message] join_request id mapped: raw_group=%s vGroup=%d raw_user=%s vUser=%d", data.GroupOpenID, groupID, data.MemberOpenID, userID)

	// 时间戳
	var timestamp int64
	switch v := data.ApplyAt.(type) {
	case string:
		timestamp, _ = strconv.ParseInt(v, 10, 64)
	case int64:
		timestamp = v
	case float64:
		timestamp = int64(v)
	default:
		timestamp = time.Now().Unix()
	}
	if timestamp == 0 {
		timestamp = time.Now().Unix()
	}

	var selfid64 int64
	if config.GetUseUin() {
		selfid64 = config.GetUinint64()
	} else {
		selfid64 = int64(p.Settings.AppID)
	}

	request := GroupRequestEvent{
		Comment:     data.VerifyInfo.String(),
		Flag:        data.JoinRequestID,
		GroupID:     groupID,
		PostType:    "request",
		RequestType: "group",
		SelfID:      selfid64,
		SubType:     "add",
		Time:        timestamp,
		UserID:      userID,
		Username:    data.Username,
	}
	//增强配置
	if !config.GetNativeOb11() {
		request.RealUserID = data.MemberOpenID
		request.RealGroupID = data.GroupOpenID
	}

	outputMap := structToMap(request)
	//上报信息到onebotv11应用端(正反ws)
	p.BroadcastMessageToAll(outputMap, p.Apiv2, data)

	mylog.Printf("用户申请加群(request): group=%s user=%s flag=%s", data.GroupOpenID, data.MemberOpenID, data.JoinRequestID)
}

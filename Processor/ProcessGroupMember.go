package Processor

import (
	"fmt"
	"strconv"
	"time"

	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/echo"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/dto"
)

// ProcessGroupMember 处理群成员变动事件
// eventType: "GROUP_MEMBER_ADD" 或 "GROUP_MEMBER_REMOVE"
func (p *Processors) ProcessGroupMember(data *dto.GroupMemberEvent, eventType string) {
	if data == nil {
		mylog.Printf("ProcessGroupMember: 数据为空")
		return
	}

	selfID := int64(config.GetAppID())

	// 将 group_openid 转为虚拟 group_id
	groupID, err := idmap.StoreIDv2(data.GroupOpenID)
	if err != nil {
		mylog.Printf("ProcessGroupMember: group_id 转换失败: %v", err)
		return
	}

	// 将 member_openid 转为虚拟 user_id（入群/退群成员）
	userID, err := idmap.StoreIDv2(data.MemberOpenID)
	if err != nil {
		mylog.Printf("ProcessGroupMember: user_id 转换失败: %v", err)
		return
	}

	// 时间戳
	var timestamp int64
	switch v := data.Timestamp.(type) {
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

	// 入群事件存储 event_id 以便后续被动回复
	if eventType == "GROUP_MEMBER_ADD" && data.EventID != "" {
		echo.AddEvnetIDv2(
			strconv.FormatInt(selfID, 10),
			data.GroupOpenID,
			data.EventID,
		)
		mylog.Printf("已存储群成员入群 event_id: %s (group=%s)", data.EventID, data.GroupOpenID)
	}

	// CQ 码描述
	memberCQ := fmt.Sprintf("[CQ:member,type=%s,group_id=%d,user_id=%d]", map[string]string{
		"GROUP_MEMBER_ADD":    "add",
		"GROUP_MEMBER_REMOVE": "remove",
	}[eventType], groupID, userID)

	switch eventType {
	case "GROUP_MEMBER_ADD":
		groupMsg := OnebotGroupMessage{
			GroupID:     groupID,
			MessageType: "group",
			PostType:    "message",
			SelfID:      selfID,
			SubType:     "normal",
			Time:        timestamp,
			UserID:      userID,
			RawMessage:  memberCQ,
			Message:     memberCQ,
			RealUserID:  data.MemberOpenID,
			RealGroupID: data.GroupOpenID,
			Sender: Sender{
				UserID: userID,
			},
		}
		outputMap := structToMap(groupMsg)
		outputMap["event_id"] = data.EventID // 保留 event_id 供 Gsk 内部使用
		mylog.Printf("群成员加入: group=%s, user=%s", data.GroupOpenID, data.MemberOpenID)
		p.BroadcastMessageToAll(outputMap, p.Apiv2, data)

	case "GROUP_MEMBER_REMOVE":
		groupMsg := OnebotGroupMessage{
			GroupID:     groupID,
			MessageType: "group",
			PostType:    "message",
			SelfID:      selfID,
			SubType:     "normal",
			Time:        timestamp,
			UserID:      userID,
			RawMessage:  memberCQ,
			Message:     memberCQ,
			RealUserID:  data.MemberOpenID,
			RealGroupID: data.GroupOpenID,
			Sender: Sender{
				UserID: userID,
			},
		}
		outputMap := structToMap(groupMsg)
		mylog.Printf("群成员离开: group=%s, user=%s", data.GroupOpenID, data.MemberOpenID)
		p.BroadcastMessageToAll(outputMap, p.Apiv2, data)

	default:
		mylog.Printf("ProcessGroupMember: 未知事件类型 %s", eventType)
	}
}

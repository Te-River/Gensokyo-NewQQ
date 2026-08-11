package event

import (
	"encoding/json"
	"fmt"
	"sync"
	"time"

	"github.com/tidwall/gjson" // 由于回包的 d 类型不确定，gjson 用于从回包json中提取 d 并进行针对性的解析

	"github.com/tencent-connect/botgo/dto"
	botlog "github.com/tencent-connect/botgo/log"
)

func init() {
	// Start a goroutine for periodic cleaning
	go cleanProcessedIDs()
}

func cleanProcessedIDs() {
	ticker := time.NewTicker(5 * time.Minute) // Adjust the interval as needed
	defer ticker.Stop()

	for range ticker.C {
		// Clean processedIDs, remove entries which are no longer needed
		processedIDs.Range(func(key, value interface{}) bool {
			processedIDs.Delete(key)
			return true
		})
	}
}

var processedIDs sync.Map

var eventParseFuncMap = map[dto.OPCode]map[dto.EventType]eventParseFunc{
	dto.WSDispatchEvent: {
		dto.EventInteractionCreate:    interactionHandler,
		dto.EventGroupAtMessageCreate: groupAtMessageHandler,
		dto.EventGroupMessageCreate:   groupMessageHandler, // [新增] 映射到新建的groupMessageHandler
		dto.EventC2CMessageCreate:     c2cMessageHandler,
		dto.EventGroupAddRobot:        groupaddbothandler,
		dto.EventGroupDelRobot:        groupdelbothandler,
		dto.EventGroupMsgReject:       groupMsgRejecthandler,
		dto.EventGroupMsgReceive:      groupMsgReceivehandler,

		// [新增] 注册用户关系链与C2C开关事件处理函数
		dto.EventFriendAdd:     friendAddHandler,
		dto.EventFriendDel:     friendDelHandler,
		dto.EventC2CMsgReject:  c2cMsgRejectHandler,
		dto.EventC2CMsgReceive: c2cMsgReceiveHandler,

		// [新增] 群成员变动事件
		dto.EventGroupMemberAdd:    groupMemberAddHandler,
		dto.EventGroupMemberRemove: groupMemberRemoveHandler,

		// [新增] 入群申请事件
		dto.EventGroupJoinRequest: groupJoinRequestHandler,
	},
}

type eventParseFunc func(event *dto.WSPayload, message []byte) error

// ParseAndHandle 处理回调事件
func ParseAndHandle(payload *dto.WSPayload) error {
	// 指定类型的 handler
	if h, ok := eventParseFuncMap[payload.OPCode][payload.Type]; ok {
		return h(payload, payload.RawMessage)
	}
	// 透传handler，如果未注册具体类型的 handler，会统一投递到这个 handler
	if DefaultHandlers.Plain != nil {
		return DefaultHandlers.Plain(payload, payload.RawMessage)
	}
	// 未知事件类型：打印日志方便排查
	fmt.Printf("[botgo] 未处理的事件类型: OP=%d, Type=%s, Raw=%s\n", payload.OPCode, payload.Type, string(payload.RawMessage))
	return nil
}

// ParseData 解析数据
func ParseData(message []byte, target interface{}) error {
	// 获取数据部分
	data := gjson.Get(string(message), "d")
	// 外层ID 与内层ID不同 外层id是event_id 用于发送参数 d内层id是id,用于put回调接口
	eventid := gjson.Get(string(message), "id").String()

	// 使用switch语句处理不同类型
	switch v := target.(type) {
	case *dto.GroupAddBotEvent:
		// 特殊处理dto.GroupAddBotEvent
		if err := json.Unmarshal([]byte(data.String()), v); err != nil {
			return err
		}
		// 设置ID字段
		v.EventID = eventid
		return nil

	case *dto.WSInteractionData:
		// 特殊处理dto.WSInteractionData
		if err := json.Unmarshal([]byte(data.String()), v); err != nil {
			return err
		}
		// 设置ID字段
		v.EventID = eventid
		return nil

	case *dto.GroupMsgRejectEvent:
		// 特殊处理dto.GroupMsgRejectEvent
		if err := json.Unmarshal([]byte(data.String()), v); err != nil {
			return err
		}
		// 设置ID字段
		v.EventID = eventid
		return nil

	case *dto.GroupMsgReceiveEvent:
		// 特殊处理dto.GroupMsgReceiveEvent
		if err := json.Unmarshal([]byte(data.String()), v); err != nil {
			return err
		}
		// 设置ID字段
		v.EventID = eventid
		return nil

		// [新增] 处理用户/C2C相关事件，注入 EventID
	case *dto.WSFriendAddData:
		if err := json.Unmarshal([]byte(data.String()), v); err != nil {
			return err
		}
		v.EventID = eventid
		return nil

	case *dto.WSFriendDelData:
		if err := json.Unmarshal([]byte(data.String()), v); err != nil {
			return err
		}
		v.EventID = eventid
		return nil

	case *dto.WSC2CMsgRejectData:
		if err := json.Unmarshal([]byte(data.String()), v); err != nil {
			return err
		}
		v.EventID = eventid
		return nil

	case *dto.WSC2CMsgReceiveData:
		if err := json.Unmarshal([]byte(data.String()), v); err != nil {
			return err
		}
		v.EventID = eventid
		return nil

	case *dto.GroupJoinRequestEvent:
		if err := json.Unmarshal([]byte(data.String()), v); err != nil {
			return err
		}
		v.EventID = eventid
		return nil

	default:
		// 对于其他类型，继续原有逻辑
		return json.Unmarshal([]byte(data.String()), target)
	}
}

func groupAtMessageHandler(payload *dto.WSPayload, message []byte) error {
	data := &dto.WSGroupATMessageData{}
	if err := ParseData(message, data); err != nil {
		return err
	}
	if _, loaded := processedIDs.LoadOrStore(data.ID, struct{}{}); loaded {
		return nil
	}
	if DefaultHandlers.GroupATMessage != nil {
		return DefaultHandlers.GroupATMessage(payload, data)
	}
	return nil
}

func c2cMessageHandler(payload *dto.WSPayload, message []byte) error {
	data := &dto.WSC2CMessageData{}
	if err := ParseData(message, data); err != nil {
		return err
	}
	if DefaultHandlers.C2CMessage != nil {
		return DefaultHandlers.C2CMessage(payload, data)
	}
	return nil
}

func groupaddbothandler(payload *dto.WSPayload, message []byte) error {
	data := &dto.GroupAddBotEvent{}
	if err := ParseData(message, data); err != nil {
		return err
	}
	if DefaultHandlers.GroupAddbot != nil {
		return DefaultHandlers.GroupAddbot(payload, data)
	}
	return nil
}

func groupdelbothandler(payload *dto.WSPayload, message []byte) error {
	data := &dto.GroupAddBotEvent{}
	if err := ParseData(message, data); err != nil {
		return err
	}
	if DefaultHandlers.GroupDelbot != nil {
		return DefaultHandlers.GroupDelbot(payload, data)
	}
	return nil
}

func groupMemberAddHandler(payload *dto.WSPayload, message []byte) error {
	data := &dto.GroupMemberEvent{}
	if err := ParseData(message, data); err != nil {
		return err
	}
	if DefaultHandlers.GroupMemberAdd != nil {
		return DefaultHandlers.GroupMemberAdd(payload, data)
	}
	botlog.Warnf("[event] GROUP_MEMBER_ADD received but GroupMemberAddEventHandler is not registered; add it to text_intent to process this event. event_id=%s group_openid=%s member_openid=%s", payload.ID, data.GroupOpenID, data.MemberOpenID)
	return nil
}

func groupMemberRemoveHandler(payload *dto.WSPayload, message []byte) error {
	data := &dto.GroupMemberEvent{}
	if err := ParseData(message, data); err != nil {
		return err
	}
	if DefaultHandlers.GroupMemberRemove != nil {
		return DefaultHandlers.GroupMemberRemove(payload, data)
	}
	botlog.Warnf("[event] GROUP_MEMBER_REMOVE received but GroupMemberRemoveEventHandler is not registered; add it to text_intent to process this event. event_id=%s group_openid=%s member_openid=%s", payload.ID, data.GroupOpenID, data.MemberOpenID)
	return nil
}

func interactionHandler(payload *dto.WSPayload, message []byte) error {
	data := &dto.WSInteractionData{}
	if err := ParseData(message, data); err != nil {
		return err
	}
	if DefaultHandlers.Interaction != nil {
		return DefaultHandlers.Interaction(payload, data)
	}
	return nil
}

func groupMsgRejecthandler(payload *dto.WSPayload, message []byte) error {
	data := &dto.GroupMsgRejectEvent{}
	if err := ParseData(message, data); err != nil {
		return err
	}
	if DefaultHandlers.GroupMsgReject != nil {
		return DefaultHandlers.GroupMsgReject(payload, data)
	}
	return nil
}

func groupMsgReceivehandler(payload *dto.WSPayload, message []byte) error {
	data := &dto.GroupMsgReceiveEvent{}
	if err := ParseData(message, data); err != nil {
		return err
	}
	if DefaultHandlers.GroupMsgReceive != nil {
		return DefaultHandlers.GroupMsgReceive(payload, data)
	}
	return nil
}

// [新增] 好友添加事件处理
func friendAddHandler(payload *dto.WSPayload, message []byte) error {
	data := &dto.WSFriendAddData{}
	if err := ParseData(message, data); err != nil {
		return err
	}
	if DefaultHandlers.FriendAdd != nil {
		return DefaultHandlers.FriendAdd(payload, data)
	}
	return nil
}

// [新增] 好友删除事件处理
func friendDelHandler(payload *dto.WSPayload, message []byte) error {
	data := &dto.WSFriendDelData{}
	if err := ParseData(message, data); err != nil {
		return err
	}
	if DefaultHandlers.FriendDel != nil {
		return DefaultHandlers.FriendDel(payload, data)
	}
	return nil
}

// [新增] C2C消息拒绝事件处理
func c2cMsgRejectHandler(payload *dto.WSPayload, message []byte) error {
	data := &dto.WSC2CMsgRejectData{}
	if err := ParseData(message, data); err != nil {
		return err
	}
	if DefaultHandlers.C2CMsgReject != nil {
		return DefaultHandlers.C2CMsgReject(payload, data)
	}
	return nil
}

// [新增] C2C消息接收事件处理
func c2cMsgReceiveHandler(payload *dto.WSPayload, message []byte) error {
	data := &dto.WSC2CMsgReceiveData{}
	if err := ParseData(message, data); err != nil {
		return err
	}
	if DefaultHandlers.C2CMsgReceive != nil {
		return DefaultHandlers.C2CMsgReceive(payload, data)
	}
	return nil
}

// [新增] 普通群消息事件处理
func groupMessageHandler(payload *dto.WSPayload, message []byte) error {
	data := &dto.WSGroupMessageData{}
	if err := ParseData(message, data); err != nil {
		return err
	}
	if _, loaded := processedIDs.LoadOrStore(data.ID, struct{}{}); loaded {
		return nil
	}
	if DefaultHandlers.GroupMessage != nil {
		return DefaultHandlers.GroupMessage(payload, data)
	}
	botlog.Warnf("[event] GROUP_MESSAGE_CREATE received but GroupMessageEventHandler is not registered; add it to text_intent to process this event. event_id=%s msg_id=%s group_openid=%s author_openid=%s", payload.ID, data.ID, data.GroupID, data.Author.ID)
	return nil
}

// [新增] 入群申请事件处理
func groupJoinRequestHandler(payload *dto.WSPayload, message []byte) error {
	data := &dto.GroupJoinRequestEvent{}
	if err := ParseData(message, data); err != nil {
		return err
	}
	if DefaultHandlers.GroupJoinRequest != nil {
		return DefaultHandlers.GroupJoinRequest(payload, data)
	}
	botlog.Warnf("[event] GROUP_JOIN_REQUEST received but GroupJoinRequestEventHandler is not registered; add it to text_intent to process this event. event_id=%s group_openid=%s member_openid=%s", payload.ID, data.GroupOpenID, data.MemberOpenID)
	return nil
}

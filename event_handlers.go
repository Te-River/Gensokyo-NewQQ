package main

import (
	"log"
	"time"

	"github.com/hoshinonyaruko/gensokyo/acnode"
	"github.com/hoshinonyaruko/gensokyo/botstats"
	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/event"
)

func runWithTimer(eventName string, fn func()) {
	go func() {
		start := time.Now()
		fn()
		elapsed := time.Since(start)
		threshold := time.Duration(config.GetLogSlowEventThresholdMS()) * time.Millisecond
		if elapsed > threshold {
			mylog.IncrementSlowEvents()
			mylog.Warnf("[SLOW] Event %s took %v (threshold: %v)", eventName, elapsed, threshold)
		}
	}()
}

func ReadyHandler() event.ReadyHandler {
	return func(event *dto.WSPayload, data *dto.WSReadyData) {
		log.Println("连接成功,ready event receive: ", data)
	}
}

func ErrorNotifyHandler() event.ErrorNotifyHandler {
	return func(err error) {
		log.Println("error notify receive: ", err)
	}
}

func ATMessageEventHandler() event.ATMessageEventHandler {
	return func(event *dto.WSPayload, data *dto.WSATMessageData) error {
		botstats.RecordMessageReceived()
		if config.GetEnableChangeWord() {
			data.Content = acnode.CheckWordIN(data.Content)
			if data.Author.Username != "" {
				data.Author.Username = acnode.CheckWordIN(data.Author.Username)
			}
		}
		runWithTimer("ATMessage", func() {
			p.ProcessGuildATMessage(data)
		})
		return nil
	}
}

func GuildEventHandler() event.GuildEventHandler {
	return func(event *dto.WSPayload, data *dto.WSGuildData) error {
		log.Println(data)
		return nil
	}
}

func ChannelEventHandler() event.ChannelEventHandler {
	return func(event *dto.WSPayload, data *dto.WSChannelData) error {
		log.Println(data)
		return nil
	}
}

func MemberEventHandler() event.GuildMemberEventHandler {
	return func(event *dto.WSPayload, data *dto.WSGuildMemberData) error {
		go p.ProcessGuildMember(data, string(event.Type))
		return nil
	}
}

func DirectMessageHandler() event.DirectMessageEventHandler {
	return func(event *dto.WSPayload, data *dto.WSDirectMessageData) error {
		botstats.RecordMessageReceived()
		if config.GetEnableChangeWord() {
			data.Content = acnode.CheckWordIN(data.Content)
			if data.Author.Username != "" {
				data.Author.Username = acnode.CheckWordIN(data.Author.Username)
			}
		}
		runWithTimer("DirectMessage", func() {
			p.ProcessChannelDirectMessage(data)
		})
		return nil
	}
}

func CreateMessageHandler() event.MessageEventHandler {
	return func(event *dto.WSPayload, data *dto.WSMessageData) error {
		botstats.RecordMessageReceived()
		if config.GetEnableChangeWord() {
			data.Content = acnode.CheckWordIN(data.Content)
			if data.Author.Username != "" {
				data.Author.Username = acnode.CheckWordIN(data.Author.Username)
			}
		}
		runWithTimer("CreateMessage", func() {
			p.ProcessGuildNormalMessage(data)
		})
		return nil
	}
}

func InteractionHandler() event.InteractionEventHandler {
	return func(event *dto.WSPayload, data *dto.WSInteractionData) error {
		mylog.Printf("收到按钮回调:%v", data)
		go p.ProcessInlineSearch(data)
		return nil
	}
}

func ThreadEventHandler() event.ThreadEventHandler {
	return func(event *dto.WSPayload, data *dto.WSThreadData) error {
		mylog.Printf("收到帖子事件:%v", data)
		go p.ProcessThreadMessage(data)
		return nil
	}
}

func GroupATMessageEventHandler() event.GroupATMessageEventHandler {
	return func(event *dto.WSPayload, data *dto.WSGroupATMessageData) error {
		runWithTimer("GroupATMessage", func() {
			p.ProcessGroupMessage(data)
		})
		if !config.GetDisableErrorChan() {
			botstats.RecordMessageReceived()
		}
		if config.GetEnableChangeWord() {
			data.Content = acnode.CheckWordIN(data.Content)
			if data.Author.Username != "" {
				data.Author.Username = acnode.CheckWordIN(data.Author.Username)
			}
		}
		return nil
	}
}

func C2CMessageEventHandler() event.C2CMessageEventHandler {
	return func(event *dto.WSPayload, data *dto.WSC2CMessageData) error {
		runWithTimer("C2CMessage", func() {
			p.ProcessC2CMessage(data)
		})
		if !config.GetDisableErrorChan() {
			botstats.RecordMessageReceived()
		}
		if config.GetEnableChangeWord() {
			data.Content = acnode.CheckWordIN(data.Content)
			if data.Author.Username != "" {
				data.Author.Username = acnode.CheckWordIN(data.Author.Username)
			}
		}
		return nil
	}
}

func GroupAddRobotEventHandler() event.GroupAddRobotEventHandler {
	return func(event *dto.WSPayload, data *dto.GroupAddBotEvent) error {
		go p.ProcessGroupAddBot(data)
		return nil
	}
}

func GroupDelRobotEventHandler() event.GroupDelRobotEventHandler {
	return func(event *dto.WSPayload, data *dto.GroupAddBotEvent) error {
		go p.ProcessGroupDelBot(data)
		return nil
	}
}

func GroupMsgRejectHandler() event.GroupMsgRejectHandler {
	return func(event *dto.WSPayload, data *dto.GroupMsgRejectEvent) error {
		go p.ProcessGroupMsgReject(data)
		return nil
	}
}

func GroupMsgReceiveHandler() event.GroupMsgReceiveHandler {
	return func(event *dto.WSPayload, data *dto.GroupMsgReceiveEvent) error {
		go p.ProcessGroupMsgRecive(data)
		return nil
	}
}

func FriendAddEventHandler() event.FriendAddEventHandler {
	return func(event *dto.WSPayload, data *dto.WSFriendAddData) error {
		go p.ProcessFriendAdd(data)
		return nil
	}
}

func FriendDelEventHandler() event.FriendDelEventHandler {
	return func(event *dto.WSPayload, data *dto.WSFriendDelData) error {
		go p.ProcessFriendDel(data)
		return nil
	}
}

func C2CMsgRejectHandler() event.C2CMsgRejectHandler {
	return func(event *dto.WSPayload, data *dto.WSC2CMsgRejectData) error {
		go p.ProcessC2CMsgReject(data)
		return nil
	}
}

func C2CMsgReceiveHandler() event.C2CMsgReceiveHandler {
	return func(event *dto.WSPayload, data *dto.WSC2CMsgReceiveData) error {
		go p.ProcessC2CMsgReceive(data)
		return nil
	}
}

func GroupMemberAddEventHandler() event.GroupMemberAddEventHandler {
	return func(event *dto.WSPayload, data *dto.GroupMemberEvent) error {
		data.EventID = event.ID
		go p.ProcessGroupMember(data, "GROUP_MEMBER_ADD")
		return nil
	}
}

func GroupMemberRemoveEventHandler() event.GroupMemberRemoveEventHandler {
	return func(event *dto.WSPayload, data *dto.GroupMemberEvent) error {
		go p.ProcessGroupMember(data, "GROUP_MEMBER_REMOVE")
		return nil
	}
}

func getHandlerByName(handlerName string) (interface{}, bool) {
	switch handlerName {
	case "ReadyHandler":
		return ReadyHandler(), true
	case "ErrorNotifyHandler":
		return ErrorNotifyHandler(), true
	case "ATMessageEventHandler":
		return ATMessageEventHandler(), true
	case "GuildEventHandler":
		return GuildEventHandler(), true
	case "MemberEventHandler":
		return MemberEventHandler(), true
	case "ChannelEventHandler":
		return ChannelEventHandler(), true
	case "DirectMessageHandler":
		return DirectMessageHandler(), true
	case "CreateMessageHandler":
		return CreateMessageHandler(), true
	case "InteractionHandler":
		return InteractionHandler(), true
	case "ThreadEventHandler":
		return ThreadEventHandler(), true
	case "GroupATMessageEventHandler":
		return GroupATMessageEventHandler(), true
	case "C2CMessageEventHandler":
		return C2CMessageEventHandler(), true
	case "GroupAddRobotEventHandler":
		return GroupAddRobotEventHandler(), true
	case "GroupDelRobotEventHandler":
		return GroupDelRobotEventHandler(), true
	case "GroupMsgRejectHandler":
		return GroupMsgRejectHandler(), true
	case "GroupMsgReceiveHandler":
		return GroupMsgReceiveHandler(), true
	case "FriendAddEventHandler":
		return FriendAddEventHandler(), true
	case "FriendDelEventHandler":
		return FriendDelEventHandler(), true
	case "C2CMsgRejectHandler":
		return C2CMsgRejectHandler(), true
	case "C2CMsgReceiveHandler":
		return C2CMsgReceiveHandler(), true
	case "GroupMessageEventHandler":
		return GroupMessageEventHandler(), true
	case "GroupMemberAddEventHandler":
		return GroupMemberAddEventHandler(), true
	case "GroupMemberRemoveEventHandler":
		return GroupMemberRemoveEventHandler(), true
	default:
		log.Printf("Unknown handler: %s\n", handlerName)
		return nil, false
	}
}

func GroupMessageEventHandler() event.GroupMessageEventHandler {
	return func(event *dto.WSPayload, data *dto.WSGroupMessageData) error {
		runWithTimer("GroupMessage", func() {
			p.ProcessGroupNormalMessage(data)
		})
		if !config.GetDisableErrorChan() {
			botstats.RecordMessageReceived()
		}
		return nil
	}
}
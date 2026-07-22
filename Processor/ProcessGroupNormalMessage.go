package Processor

import (
	"fmt"
	"log"
	"strconv"
	"strings"
	"time"

	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/echo"
	"github.com/hoshinonyaruko/gensokyo/handlers"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/hoshinonyaruko/gensokyo/unioncache"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/websocket/client"
)

// ProcessGroupNormalMessage 处理 GROUP_MESSAGE_CREATE 事件（QQ 全量群消息）
// 注意：QQ 平台的 GROUP_MESSAGE_CREATE 事件包含群内所有消息（含 @Bot 的），
// Content 中可能带有 <@OpenID> 原文，后续 RevertTransformedText 会统一处理 @ 格式转换，
// 此处无需手动剥离。
func (p *Processors) ProcessGroupNormalMessage(data *dto.WSGroupMessageData) error {
	s := client.GetGlobalS()
	AppIDString := strconv.FormatUint(p.Settings.AppID, 10)
	currentTimeMillis := time.Now().UnixNano() / 1e6
	echostr := fmt.Sprintf("%s_%d_%d", AppIDString, s, currentTimeMillis)

	var userid64, GroupID64 int64
	var err error

	if _, err := idmap.RecordQQGroupMessageReception(data.GroupID, data.ID, true); err != nil {
		mylog.Errorf("[idmap] 更新QQ群全量消息接收标志失败: group=%s message=%s event=GROUP_MESSAGE_CREATE error=%v", data.GroupID, data.ID, err)
	}

	if data.Author.ID == "" {
		mylog.Printf("出现ID为空未知错误.%v\n", data)
		return nil
	}

	// union id 缓存
	if data.Author.UnionOpenID != "" && data.Author.ID != "" {
		unioncache.Store(data.Author.ID, data.Author.UnionOpenID)
	}

	var platform string
	if config.GetUnionID() {
		data.Author.ID = data.Author.UnionOpenID
		platform = "unionqq"
	} else {
		platform = "qq"
	}

	if !config.GetStringOb11() {
		if config.GetIdmapPro() {
			GroupID64, userid64, err = idmap.StoreIDv2Pro(data.GroupID, data.Author.ID)
			if err != nil {
				mylog.Errorf("Error storing ID: %v", err)
			}
			_, _ = idmap.StoreIDv2(data.GroupID)
			_, _ = idmap.StoreIDv2(data.Author.ID)
			if !config.GetHashIDValue() {
				mylog.Fatalf("避坑日志:你开启了高级id转换,请设置hash_id为true,并且删除idmaps并重启")
			}
			idmap.SimplifiedStoreID(data.Author.ID)
			idmap.SimplifiedStoreID(data.GroupID)
			echo.AddMsgIDv3(AppIDString, data.GroupID, data.ID)
		} else {
			GroupID64, err = idmap.StoreIDv2(data.GroupID)
			if err != nil {
				mylog.Errorf("failed to convert GroupID64 to int: %v", err)
				return nil
			}
			userid64, err = idmap.StoreIDv2(data.Author.ID)
			if err != nil {
				mylog.Printf("Error storing ID: %v", err)
				return nil
			}
			// 缓存用户名，供出站 [CQ:at,qq=虚拟ID] 转换为 <@username>
			if data.Author.Username != "" {
				idmap.StoreUserName(strconv.FormatInt(userid64, 10), data.Author.Username)
			}
		}
		mylog.Printf("[message] group id mapped: raw_group=%s vGroup=%d raw_user=%s vUser=%d", data.GroupID, GroupID64, data.Author.ID, userid64)
	}

	// 前置兼容：注册 bot 自身的 OpenID 到 selfAtIDs。
	// QQ 平台在 GROUP_MESSAGE_CREATE 中使用 OpenID 格式标识被 @ 的用户，
	// 与 handlers.BotID（来自 Ready 事件）不同，必须从 Mentions 中获取真实 ID。
	// is_you 字段可能不准确（如多实例场景），因此 bot:true 的 OpenID 也注册。
	// 不在此处剥离 <@OpenID>，统一交给 RevertTransformedText 处理：
	//   - 自身 @ → 全量群消息始终剥离，不依赖 remove_at 配置
	//   - 其他 @ → 转换为 [CQ:at,qq=虚拟ID]
	// 这样消息内容仅含 <@bot> 时，仍能产生非空 messageText，不会被误判为黑白名单拦截。
	toMe := false
	for _, mention := range data.Mentions {
		if mention.IsYou || mention.Bot {
			if mention.IsYou {
				toMe = true
			}
			handlers.RememberSelfAtID(mention.ID)
			break
		}
	}

	messageText := data.Content
	GetDisableErrorChan := config.GetDisableErrorChan()

	if !GetDisableErrorChan {
		messageText = handlers.RevertTransformedText(data, "group", p.Api, p.Apiv2, GroupID64, userid64, config.GetWhiteEnable(4))
		if messageText == "" {
			mylog.Printf("信息被自定义黑白名单拦截")
			return nil
		}
		if err := p.HandleFrameworkCommand(messageText, data, "group"); err != nil {
			mylog.Errorf("处理 GROUP_MESSAGE_CREATE 框架指令失败: %v", err)
		}
	} else {
		messageText = strings.TrimSpace(messageText)
		if messageText == "/ " || messageText == " /" {
			messageText = " "
		}
		if config.GetRemovePrefixValue() {
			if idx := strings.Index(messageText, "/"); idx != -1 {
				messageText = messageText[:idx] + messageText[idx+1:]
			}
		}
	}

	if config.GetAddAtGroup() {
		messageText = "[CQ:at,qq=" + config.GetAppIDStr() + "] " + messageText
	}

	var messageID int
	if !config.GetStringOb11() {
		var messageID64 int64
		if config.GetMemoryMsgid() {
			messageID64, err = echo.StoreCacheInMemory(data.ID)
		} else {
			messageID64, err = idmap.StoreCachev2(data.ID)
		}
		if err != nil {
			log.Fatalf("Error storing ID: %v", err)
		}
		messageID = int(messageID64)
		mylog.Printf("[message] group msg_id mapped: raw_msg=%s vMsg=%d", data.ID, messageID64)
	}
	// 记录该群该用户最新一条消息的 real msg_id（用于 delete_group_msg 撤回）
	idmap.StoreLatestMsgID(data.GroupID, data.Author.ID, data.ID)

	if config.GetAutoBind() {
		if len(data.Attachments) > 0 && data.Attachments[0].URL != "" {
			p.Autobind(data)
		}
	}

	var segmentedMessages interface{} = messageText
	if config.GetArrayValue() {
		segmentedMessages = handlers.ConvertToSegmentedMessage(data)
	}

	var IsBindedUserId, IsBindedGroupId bool
	if !config.GetStringOb11() {
		if config.GetHashIDValue() {
			IsBindedUserId = idmap.CheckValue(data.Author.ID, userid64)
			IsBindedGroupId = idmap.CheckValue(data.GroupID, GroupID64)
		} else {
			IsBindedUserId = idmap.CheckValuev2(userid64)
			IsBindedGroupId = idmap.CheckValuev2(GroupID64)
		}
	}

	var selfid64 int64
	if config.GetUseUin() {
		selfid64 = config.GetUinint64()
	} else {
		selfid64 = int64(p.Settings.AppID)
	}

	var groupMsg OnebotGroupMessage
	var groupMsgS OnebotGroupMessageS
	var groupMsgMap map[string]interface{}

	if !config.GetStringOb11() {
		groupMsg = OnebotGroupMessage{
			RawMessage:  messageText,
			Message:     segmentedMessages,
			MessageID:   messageID,
			GroupID:     GroupID64,
			MessageType: "group",
			PostType:    "message",
			SelfID:      selfid64,
			UserID:      userid64,
			Sender: Sender{
				UserID: userid64,
				Sex:    "0",
				Age:    0,
				Area:   "0",
				Level:  "0",
			},
			// ------ 修改 start ------
			SubType: "normal",
			// ------ 修改 end ------
			Time: time.Now().Unix(),
			ToMe: toMe,
		}
		if !config.GetNativeOb11() {
		       groupMsg.RealMessageType = "group"
		       groupMsg.IsBindedUserId = IsBindedUserId
		       groupMsg.IsBindedGroupId = IsBindedGroupId
		       groupMsg.RealGroupID = data.GroupID
		       groupMsg.RealUserID = data.Author.ID
		       groupMsg.Avatar, _ = GenerateAvatarURLV2(data.Author.ID)
		      }
		      groupMsg.IsFullGroupMessage = true
		// nick/card
		if CaN := config.GetCardAndNick(); CaN != "" {
			groupMsg.Sender.Nickname = CaN
			groupMsg.Sender.Card = CaN
		} else if data.Author.Username != "" {
			groupMsg.Sender.Nickname = data.Author.Username
		}
		if config.GetTwoWayEcho() {
			groupMsg.Echo = echostr
			echo.AddMsgIDv3(AppIDString, echostr, messageText)
		}
		// role
		masterIDs := config.GetMasterID()
		isMaster := false
		for _, id := range masterIDs {
			if strconv.FormatInt(userid64, 10) == id {
				isMaster = true
				break
			}
		}
		if data.Author != nil && data.Author.MemberRole != "" {
			groupMsg.Sender.Role = data.Author.MemberRole
		} else if isMaster {
			groupMsg.Sender.Role = "owner"
		} else {
			groupMsg.Sender.Role = "member"
		}
		echo.AddMsgID(AppIDString, s, data.ID)
		echo.AddMsgType(AppIDString, s, "group")
		echo.AddMsgID(AppIDString, GroupID64, data.ID)
		echo.AddMsgIDv2(AppIDString, GroupID64, userid64, data.ID)
		idmap.WriteConfigv2(fmt.Sprint(GroupID64), "type", "group")
		echo.AddMsgType(AppIDString, GroupID64, "group")
		echo.AddLazyMessageId(strconv.FormatInt(GroupID64, 10), data.ID, time.Now())
		echo.AddLazyMessageIdv2(strconv.FormatInt(GroupID64, 10), strconv.FormatInt(userid64, 10), data.ID, time.Now())
		if config.GetStringAction() {
			echo.AddLazyMessageId(data.GroupID, data.ID, time.Now())
			echo.AddLazyMessageIdv2(data.GroupID, data.Author.ID, data.ID, time.Now())
		}
		groupMsgMap = structToMap(groupMsg)
	} else {
		var imgurl string
		if len(data.Attachments) > 0 {
			imgurl = data.Attachments[0].URL
		}
		groupMsgS = OnebotGroupMessageS{
			RawMessage:  messageText,
			Message:     segmentedMessages,
			MessageID:   data.ID,
			GroupID:     data.GroupID,
			MessageType: "group",
			PostType:    "message",
			SelfID:      selfid64,
			UserID:      data.Author.ID,
			Sender: Sender{
				UserID: userid64,
				Sex:    "0",
				Age:    0,
				Area:   imgurl,
				Level:  "0",
			},
			// ------ 修改 start ------
			SubType: "normal",
			// ------ 修改 end ------
			Time:     time.Now().Unix(),
			ToMe:     toMe,
			Platform: platform,
		}
		if !config.GetNativeOb11() {
		       groupMsgS.RealMessageType = "group"
		       groupMsgS.RealGroupID = data.GroupID
		       groupMsgS.RealUserID = data.Author.ID
		       groupMsgS.Avatar, _ = GenerateAvatarURLV2(data.Author.ID)
		      }
		      groupMsgS.IsFullGroupMessage = true
		if CaN := config.GetCardAndNick(); CaN != "" {
			groupMsgS.Sender.Nickname = CaN
			groupMsgS.Sender.Card = CaN
		} else if data.Author.Username != "" {
			groupMsgS.Sender.Nickname = data.Author.Username
		}
		if config.GetTwoWayEcho() {
			groupMsgS.Echo = echostr
			echo.AddMsgIDv3(AppIDString, echostr, messageText)
		}
		// role
		masterIDs := config.GetMasterID()
		isMaster := false
		for _, id := range masterIDs {
			if strconv.FormatInt(userid64, 10) == id {
				isMaster = true
				break
			}
		}
		if data.Author != nil && data.Author.MemberRole != "" {
			groupMsgS.Sender.Role = data.Author.MemberRole
		} else if isMaster {
			groupMsgS.Sender.Role = "owner"
		} else {
			groupMsgS.Sender.Role = "member"
		}
		echo.AddMsgID(AppIDString, s, data.ID)
		echo.AddMsgType(AppIDString, s, "group")
		idmap.WriteConfigv2(data.GroupID, "type", "group")
		echo.AddLazyMessageId(data.GroupID, data.ID, time.Now())
		echo.AddLazyMessageIdv2(data.GroupID, data.Author.ID, data.ID, time.Now())
		groupMsgMap = structToMap(groupMsgS)
	}

	if !GetDisableErrorChan {
		go p.BroadcastMessageToAll(groupMsgMap, p.Apiv2, data)
	} else {
		go p.BroadcastMessageToAllFAF(groupMsgMap, p.Apiv2, data)
	}
	return nil
}

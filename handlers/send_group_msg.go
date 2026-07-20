package handlers

import (
	"bytes"
	"context"
	"encoding/base64"
	"fmt"
	"io"
	"math/rand"
	"net"
	"net/http"
	neturl "net/url"
	"os"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"time"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/echo"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/images"
	"github.com/hoshinonyaruko/gensokyo/mdutil"
	"github.com/hoshinonyaruko/gensokyo/messagequeue"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/hoshinonyaruko/gensokyo/silk"
	"github.com/hoshinonyaruko/gensokyo/structs"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/dto/keyboard"
	"github.com/tencent-connect/botgo/openapi"
)

// 包级 HTTP 客户端，统一设置超时（30 秒），用于所有出站 HTTP 请求
var httpClient = &http.Client{
	Timeout: 30 * time.Second,
	CheckRedirect: func(req *http.Request, via []*http.Request) error {
		if len(via) >= 10 {
			return fmt.Errorf("too many redirects")
		}
		// 每次重定向都重新校验目标地址，防止 302 绕过 SSRF 检查
		if isPrivateOrLoopback(req.URL.String()) {
			return fmt.Errorf("SSRF 阻止: 重定向目标为私有地址: %s", req.URL.String())
		}
		return nil
	},
}

func init() {
	callapi.RegisterHandler("send_group_msg", HandleSendGroupMsg)
	callapi.RegisterHandler("send_to_group", HandleSendGroupMsg)
}

func HandleSendGroupMsg(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	// 使用 message.Echo 作为key来获取消息类型
	var msgType string
	if echoStr, ok := message.Echo.(string); ok {
		// 当 message.Echo 是字符串类型时执行此块
		msgType = echo.GetMsgTypeByKey(echoStr)
	}
	// 检查GroupID是否为0
	checkZeroGroupID := func(id interface{}) bool {
		switch v := id.(type) {
		case int:
			return v != 0
		case int64:
			return v != 0
		case string:
			return v != "0" // 检查字符串形式的0
		default:
			return true // 如果不是int、int64或string，假定它不为0
		}
	}

	// 检查UserID是否为0
	checkZeroUserID := func(id interface{}) bool {
		switch v := id.(type) {
		case int:
			return v != 0
		case int64:
			return v != 0
		case string:
			return v != "0" // 同样检查字符串形式的0
		default:
			return true // 如果不是int、int64或string，假定它不为0
		}
	}

	if message.Params.GroupID != nil && len(message.Params.GroupID.(string)) != 32 {
		if msgType == "" && message.Params.GroupID != nil && checkZeroGroupID(message.Params.GroupID) {
			msgType = GetMessageTypeByGroupid(config.GetAppIDStr(), message.Params.GroupID)
		}
		if msgType == "" && message.Params.UserID != nil && checkZeroUserID(message.Params.UserID) {
			msgType = GetMessageTypeByUserid(config.GetAppIDStr(), message.Params.UserID)
		}
		if msgType == "" && message.Params.GroupID != nil && checkZeroGroupID(message.Params.GroupID) {
			msgType = GetMessageTypeByGroupidV2(message.Params.GroupID)
		}
		if msgType == "" && message.Params.UserID != nil && checkZeroUserID(message.Params.UserID) {
			msgType = GetMessageTypeByUseridV2(message.Params.UserID)
		}
	}

	// New checks for UserID and GroupID being nil or 0
	if (message.Params.UserID == nil || !checkZeroUserID(message.Params.UserID)) &&
		(message.Params.GroupID == nil || !checkZeroGroupID(message.Params.GroupID)) {
		mylog.Printf("send_group_msgs接收到错误action: %v", message)
		return "", nil
	}

	// 内部逻辑 ProcessGroupAddBot.go 中定义的 通过http和ws无法触发 锁定类型
	if message.Action == "send_group_msg_group" {
		msgType = "group"
	}

	mylog.Printf("send_group_msg获取到信息类型:%v", msgType)
	var idInt64 int64
	var err error
	var retmsg string

	if len(message.Params.GroupID.(string)) == 32 {
		msgType = "group"
	} else if message.Params.UserID != nil && len(message.Params.UserID.(string)) == 32 {
		msgType = "group_private"
	} else {
		if message.Params.GroupID != "" {
			idInt64, err = ConvertToInt64(message.Params.GroupID)
		} else if message.Params.UserID != "" {
			idInt64, err = ConvertToInt64(message.Params.UserID)
		}
	}

	if message.Params.GroupID != nil && len(message.Params.GroupID.(string)) != 32 {
		// stringob11通过字段判断类型,不需要递归
		if !config.GetStringOb11() {
			//设置递归 对直接向gsk发送action时有效果
			if msgType == "" {
				messageCopy := message
				if err != nil {
					mylog.Printf("错误：无法转换 ID %v\n", err)
				} else {
					// 递归3次
					echo.AddMapping(idInt64, 4)
					// 递归调用handleSendGroupMsg，使用设置的消息类型
					echo.AddMsgType(config.GetAppIDStr(), idInt64, "group_private")
					retmsg, _ = HandleSendGroupMsg(client, api, apiv2, messageCopy)
				}
			} else if echo.GetMapping(idInt64) <= 0 {
				// 特殊值代表不递归
				echo.AddMapping(idInt64, 10)
			}
		}
	}

	switch msgType {
	case "group":
		// 解析消息内容
		messageText, foundItems := parseMessageContent(message.Params, message, client, api, apiv2)
		var SSM bool
		// 使用 echo 获取消息ID
		var messageID string
		// EventID
		var eventID string
		if config.GetLazyMessageId() {
			//由于实现了Params的自定义unmarshell 所以可以类型安全的断言为string
			messageID = echo.GetLazyMessagesId(message.Params.GroupID.(string))
			mylog.Printf("GetLazyMessagesId: %v", messageID)
			//如果应用端传递了user_id 就让at不要顺序乱套
			if message.Params.UserID != nil && message.Params.UserID.(string) != "" && message.Params.UserID.(string) != "0" {
				messageID = echo.GetLazyMessagesIdv2(message.Params.GroupID.(string), message.Params.UserID.(string))
				mylog.Printf("GetLazyMessagesIdv2: %v", messageID)
			} else {
				//如果应用端没有传递userid 那就用群号模式的lazyid 但是不保证顺序是对的
				messageID = echo.GetLazyMessagesId(message.Params.GroupID.(string))
				mylog.Printf("GetLazyMessagesIdv1: %v", messageID)
			}
			//2000是群主动 此时不能被动转主动
			//仅在开启lazy_message_id时，有信息主动转被动特性，即，SSM
			if messageID != "2000" {
				//尝试发送栈内信息
				SSM = true
			}
		}
		if messageID == "" {
			if echoStr, ok := message.Echo.(string); ok {
				messageID = echo.GetMsgIDByKey(echoStr)
				mylog.Println("echo取群组发信息对应的message_id:", messageID)
			}
		}

		var originalGroupID, originalUserID string
		if len(message.Params.GroupID.(string)) != 32 {
			// 检查UserID是否为nil
			if message.Params.UserID != nil && config.GetIdmapPro() && message.Params.UserID.(string) != "" && message.Params.UserID.(string) != "0" {
				// 如果UserID不是nil且配置为使用Pro版本，则调用RetrieveRowByIDv2Pro
				originalGroupID, originalUserID, err = idmap.RetrieveRowByIDv2Pro(message.Params.GroupID.(string), message.Params.UserID.(string))
				if err != nil {
					mylog.Printf("Error1 retrieving original GroupID: %v", err)
				}
				mylog.Printf("测试,通过idmaps-pro获取的originalGroupID:%v", originalGroupID)
				if originalGroupID == "" {
					originalGroupID, err = idmap.RetrieveRowByIDv2(message.Params.GroupID.(string))
					if err != nil {
						mylog.Printf("Error2 retrieving original GroupID: %v", err)
						return "", nil
					}
					mylog.Printf("测试,通过idmaps获取的originalGroupID:%v", originalGroupID)
				}
			} else {
				// 如果UserID是nil或配置不使用Pro版本，则调用RetrieveRowByIDv2
				originalGroupID, err = idmap.RetrieveRowByIDv2(message.Params.GroupID.(string))
				if err != nil {
					mylog.Printf("Error retrieving original GroupID: %v", err)
				}
				// 检查 message.Params.UserID 是否为 nil
				if message.Params.UserID == nil {
					//mylog.Println("UserID is nil")
				} else {
					// 进行类型断言，确认 UserID 不是 nil
					userID, ok := message.Params.UserID.(string)
					if !ok {
						mylog.Println("UserID is not a string")
						// 处理类型断言失败的情况
					} else {
						originalUserID, err = idmap.RetrieveRowByIDv2(userID)
						if err != nil {
							mylog.Printf("Error retrieving original UserID: %v", err)
						}
					}
				}
			}
			// 这里已经重复覆盖为32位数ID了
			message.Params.GroupID = originalGroupID
			message.Params.UserID = originalUserID
		}

		//2000是群主动 此时不能被动转主动
		if SSM {
			//mylog.Printf("正在使用Msgid:%v 补发之前失败的主动信息,请注意AtoP不要设置超过3,否则可能会影响正常信息发送", messageID)
			//mylog.Printf("originalGroupID:%v ", originalGroupID)
			SendStackMessages(apiv2, messageID, message.Params.GroupID.(string))
		}
		mylog.Println("群组发信息messageText:", messageText)
		//mylog.Println("foundItems:", foundItems)
		if messageID == "" {
			// 检查 UserID 是否为 nil
			if message.Params.UserID != nil && message.Params.UserID.(string) != "" && message.Params.UserID.(string) != "0" {
				messageID = GetMessageIDByUseridAndGroupid(config.GetAppIDStr(), message.Params.UserID, message.Params.GroupID)
				mylog.Println("通过GetMessageIDByUseridAndGroupid函数获取的message_id:", message.Params.GroupID, messageID)
			} else {
				// 如果 UserID 是 nil，可以在这里处理，例如记录日志或采取其他措施
				mylog.Println("UserID 为 nil,跳过 GetMessageIDByUseridAndGroupid 调用")
			}
		}
		// 如果messageID为空，通过函数获取
		if messageID == "" {
			messageID = GetMessageIDByUseridOrGroupid(config.GetAppIDStr(), message.Params.GroupID)
			mylog.Println("通过GetMessageIDByUseridOrGroupid函数获取的message_id:", message.Params.GroupID, messageID)
		}
		// 主动推送：没有用户上下文时，缓存的 msg_id 已过期，清掉
		if messageID != "" && (message.Params.UserID == nil || message.Params.UserID.(string) == "" || message.Params.UserID.(string) == "0") && eventID == "" {
			messageID = ""
			mylog.Println("主动推送模式，清空缓存的 msg_id")
		}
		// [CQ:active] 标记：强制走主动推送，即使有 msg_id
		if _, ok := foundItems["active"]; ok {
			messageID = ""
			eventID = ""
			mylog.Println("[CQ:active] 标记，强制主动推送")
		}
		//开发环境用 1000在群里无效
		// if config.GetDevMsgID() {
		// 	messageID = "1000"
		// }
		if messageID == "2000" {
			messageID = ""
			mylog.Println("通过lazymessage_id模式发送群聊/频道主动信息,群聊每月仅4次机会,如果本信息非主动推送信息,请提交issue")
			// 不使用stringob11的
			if !config.GetStringOb11() {
				eventID = GetEventIDByUseridOrGroupid(config.GetAppIDStr(), message.Params.GroupID)
			} else {
				eventID = GetEventIDByUseridOrGroupidv2(config.GetAppIDStr(), message.Params.GroupID)
			}
			mylog.Printf("尝试获取当前是否有eventID可用,如果有则不消耗主动次数:%v", eventID)
		}
		mylog.Printf("群组发信息使用messageID:[%v]", messageID)
		var singleItem = make(map[string][]string)
		var imageType, imageUrl string
		imageCount := 0

		// 检查不同类型的图片并计算数量
		if imageURLs, ok := foundItems["local_image"]; ok && len(imageURLs) == 1 {
			imageType = "local_image"
			imageUrl = imageURLs[0]
			imageCount++
		} else if imageURLs, ok := foundItems["url_image"]; ok && len(imageURLs) == 1 {
			imageType = "url_image"
			imageUrl = imageURLs[0]
			imageCount++
		} else if imageURLs, ok := foundItems["url_images"]; ok && len(imageURLs) == 1 {
			imageType = "url_images"
			imageUrl = imageURLs[0]
			imageCount++
		} else if base64Images, ok := foundItems["base64_image"]; ok && len(base64Images) == 1 {
			imageType = "base64_image"
			imageUrl = base64Images[0]
			imageCount++
		}

		if imageCount == 1 && messageText != "" {
			var groupMessage *dto.MessageToCreate
			mylog.Printf("发图文混合信息-群")
			// 创建包含单个图片的 singleItem
			singleItem[imageType] = []string{imageUrl}
			msgseq := echo.GetMappingSeq(messageID)
			echo.AddMappingSeq(messageID, msgseq+1)
			groupReply := generateGroupMessage(messageID, eventID, singleItem, "", msgseq+1, apiv2, message.Params.GroupID.(string))
			// 进行类型断言
			richMediaMessage, ok := groupReply.(*dto.RichMediaMessage)
			// 如果断言为RichMediaMessage失败
			if !ok {
				// 尝试断言为MessageToCreate
				groupMessage, ok = groupReply.(*dto.MessageToCreate)
				if !ok {
					mylog.Printf("Error: Expected RichMediaMessage type for key,value:%v", groupReply)
					return "", nil
				}
			}
			var transmd bool
			var md *dto.Markdown
			var kb *keyboard.MessageKeyboard
			//判断是否需要自动转换md
			if config.GetTwoWayEcho() {
				md, kb, transmd = auto_md(message, messageText, richMediaMessage)
			}
			// 如果groupMessage是nil 说明groupReply是richMediaMessage类型 如果groupMessage不是nil 说明groupReply是MessageToCreate
			if groupMessage == nil {
				//如果没有转换成md发送
				if !transmd {
					// 上传图片并获取FileInfo
					fileInfo, err := uploadMedia(context.TODO(), message.Params.GroupID.(string), richMediaMessage, apiv2)
					if err != nil {
						mylog.Printf("上传图片失败: %v", err)
						return "", nil // 或其他错误处理
					}
					// 创建包含文本和图像信息的消息
					msgseq = echo.GetMappingSeq(messageID)
					echo.AddMappingSeq(messageID, msgseq+1)
					groupMessage = &dto.MessageToCreate{
						Content: messageText, // 添加文本内容
						Media: &dto.Media{
							FileInfo: fileInfo, // 添加图像信息
						},
						MsgID:   messageID,
						EventID: eventID,
						MsgSeq:  msgseq,
						MsgType: 7, // 假设7是组合消息类型
					}
					groupMessage.Timestamp = time.Now().Unix() // 设置时间戳
				} else {
					//将kb和md组合成groupMessage并用MsgType=2发送

					msgseq = echo.GetMappingSeq(messageID)
					echo.AddMappingSeq(messageID, msgseq+1)
					groupMessage = &dto.MessageToCreate{
						Content:  "markdown", // 添加文本内容
						MsgID:    messageID,
						EventID:  eventID,
						MsgSeq:   msgseq,
						Markdown: md,
						Keyboard: kb,
						MsgType:  2, // 假设7是组合消息类型
					}
					groupMessage.Timestamp = time.Now().Unix() // 设置时间戳

				}
			} else {
				// 为groupMessage附加内容 变成图文信息
				groupMessage.Content = messageText
				groupMessage.Timestamp = time.Now().Unix() // 设置时间戳
			}

			var resp *dto.GroupMessageResponse
			// 发送组合消息
			resp, err = apiv2.PostGroupMessage(context.TODO(), message.Params.GroupID.(string), groupMessage)
			if err != nil {
				mylog.Printf("发送组合消息失败: %v", err)
				// 错误保存到本地
				if config.GetSaveError() {
					mylog.ErrLogToFile("type", "PostGroupMessage")
					mylog.ErrInterfaceToFile("request", groupMessage)
					mylog.ErrLogToFile("error", err.Error())
				}
			}
			if err != nil && strings.Contains(err.Error(), `"code":22009`) {
				mylog.Printf("信息发送失败,加入到队列中,下次被动信息进行发送")
				var pair echo.MessageGroupPair
				pair.Group = message.Params.GroupID.(string)
				pair.GroupMessage = groupMessage
				echo.PushGlobalStack(pair)
			} else if err != nil && strings.Contains(err.Error(), `"code":40034025`) {
				// event_id无效的时候
				groupMessage.EventID = ""
				resp, err = apiv2.PostGroupMessage(context.TODO(), message.Params.GroupID.(string), groupMessage)
				if err != nil {
					mylog.Printf("发送组合消息失败: %v", err)
					// 错误保存到本地
					if config.GetSaveError() {
						mylog.ErrLogToFile("type", "PostGroupMessage")
						mylog.ErrInterfaceToFile("request", groupMessage)
						mylog.ErrLogToFile("error", err.Error())
					}
				}
			} else if err != nil && strings.Contains(err.Error(), "context deadline exceeded") {
				postGroupMessageWithRetry(apiv2, message.Params.GroupID.(string), groupMessage)
			}

			rememberLatestBotGroupMessageInGroup(message.Params.GroupID.(string), resp)
			if !config.GetNoRetMsg() {
				if config.GetThreadsRetMsg() {
					if !config.GetStringOb11() {
						go SendResponse(client, err, &message, resp, api, apiv2)
					} else {
						go SendResponseSB(client, err, &message, resp, api, apiv2)
					}
				} else {
					if !config.GetStringOb11() {
						// 发送成功回执
						retmsg, _ = SendResponse(client, err, &message, resp, api, apiv2)
					} else {
						// 发送成功回执
						retmsg, _ = SendResponseSB(client, err, &message, resp, api, apiv2)
					}

				}
			}

			delete(foundItems, imageType) // 从foundItems中删除已处理的图片项
			messageText = ""
		}

		// 优先发送文本信息
		if messageText != "" {
			// 处理出站 [CQ:member] → 自动转换 user_id 并设置 eventID/主动模式
			// 返回的 realGroupID 是转换后的真实 GroupOpenID，可用作目标群
			var realGroupID string
			messageText, realGroupID, _ = ProcessCQMemberOutbound(messageText, &eventID, message.Params.GroupID.(string), apiv2)
			if realGroupID != "" {
				mylog.Printf("[CQ:member] CQ 码 group_id 已转为 OpenID=%s", realGroupID)
			}

			// 如果 CQ 码中携带了 group_id，优先使用它作为目标群（支持跨群路由）
			targetGroupID := message.Params.GroupID.(string)
			if realGroupID != "" {
				targetGroupID = realGroupID
			}

			// 处理出站 [CQ:remove] → 撤回消息并从文本中移除 CQ 码
			messageText = ProcessCQRemoveOutbound(messageText, apiv2, targetGroupID)
			// 如果处理后 messageText 为空（纯 CQ:remove 消息），直接返回客户端回执
			if strings.TrimSpace(messageText) == "" && len(foundItems) == 0 {
				mylog.Printf("[CQ:remove] 纯 CQ 码消息，不发送到 QQ 频道")
				SendResponse(client, nil, &message, nil, api, apiv2)
				return "", nil
			}

					var md *dto.Markdown
					    var kb *keyboard.MessageKeyboard
					    if mdItems, ok := foundItems["markdown"]; ok && len(mdItems) > 0 {
					     md, kb = parseMarkdownFromMessage(mdItems[0])
					     // 步骤 1: 转换 markdown 内部和 messageText 中的 [CQ:at] 为 <qqbot-at-user>
					     if md != nil && md.Content != "" {
					      md.Content = ResolveMarkdownAtMentions(md.Content)
					      md.Content = ResolveMarkdownImages(md.Content, apiv2)
					     }
					     messageText = ResolveMarkdownAtMentions(messageText)
					     // 步骤 2: 将整个 messageText 合并到 markdown 内容头部（qq 官方 md 整个消息以 md 语法渲染）
					     md.Content = messageText + "\n" + md.Content
					     // 步骤 3: 清理 messageText 中的 [CQ:markdown] 标记
					     mdRe := regexp.MustCompile(`\[CQ:markdown,[^\]]*\]`)
					     messageText = mdRe.ReplaceAllString(messageText, "")
					    }

					    // 没有 markdown 时，纯文本消息转换 [CQ:at] 为 @用户名
					    if md == nil {
					     messageText = resolvePlainTextAtMentions(messageText)
					    }

			// message.Params.GroupID 已在前面转换为真实 OpenID，直接使用

			msgseq := echo.GetMappingSeq(messageID)
			echo.AddMappingSeq(messageID, msgseq+1)
			groupReply := generateGroupMessage(messageID, eventID, nil, messageText, msgseq+1, apiv2, targetGroupID)

			// 进行类型断言
			groupMessage, ok := groupReply.(*dto.MessageToCreate)
			if !ok {
				mylog.Println("Error: Expected MessageToCreate type.")
				return "", nil // 或其他错误处理
			}

			// 如果有 markdown，改为 markdown 消息
			if md != nil {
				groupMessage.Markdown = md
				groupMessage.Keyboard = kb // 携带按钮
				groupMessage.MsgType = 2
				groupMessage.Content = ""
				// 从 foundItems 中移除 markdown，避免下方循环重复发送
				delete(foundItems, "markdown")
				mylog.Printf("[CQ:markdown] 将消息类型切换为 markdown")
			}

			// 处理 [CQ:reply,id=数字] → message_reference + msg_id
			    if replyIDs, ok := foundItems["reply_msg_id"]; ok && len(replyIDs) > 0 {
			     if messageText != "" {
			      // 虚拟 reply ID → 真实 QQ API message_id
			      realReplyID, err := idmap.RetrieveRowByCachev2(replyIDs[0])
			      if err == nil && realReplyID != "" {
			       // echo 缓存中的 ID 格式为 "GroupID MessageID"，取后半段
			       parts := strings.Split(realReplyID, " ")
			       refID := parts[len(parts)-1]
			       groupMessage.MessageReference = &dto.MessageReference{
			        MessageID:             refID,
			        IgnoreGetMessageError: true,
			       }
			       // 同时设置 msg_id，确保 v2 API 识别为回复
			       groupMessage.MsgID = refID
			       mylog.Printf("[CQ:reply] 设置回复消息: msg_id=%s", refID)
			      } else {
			       mylog.Printf("[CQ:reply] 虚拟 ID %s 反查失败: %v", replyIDs[0], err)
			      }
			     }
			    }

			var resp *dto.GroupMessageResponse
			groupMessage.Timestamp = time.Now().Unix() // 设置时间戳
			//重新为err赋值
			resp, err = apiv2.PostGroupMessage(context.TODO(), targetGroupID, groupMessage)
			if err != nil {
				mylog.Printf("发送文本群组信息失败: %v", err)
				// 错误保存到本地
				if config.GetSaveError() {
					mylog.ErrLogToFile("type", "PostGroupMessage")
					mylog.ErrInterfaceToFile("request", groupMessage)
					mylog.ErrLogToFile("error", err.Error())
				}
			}
			if err != nil && strings.Contains(err.Error(), `"code":22009`) {
				mylog.Printf("信息发送失败,加入到队列中,下次被动信息进行发送")
				var pair echo.MessageGroupPair
				pair.Group = targetGroupID
				pair.GroupMessage = groupMessage
				echo.PushGlobalStack(pair)
			} else if err != nil && strings.Contains(err.Error(), `"code":40034025`) {
				groupMessage.EventID = ""
				resp, err = apiv2.PostGroupMessage(context.TODO(), targetGroupID, groupMessage)
				if err != nil {
					mylog.Printf("发送文本群组信息失败: %v", err)
					// 错误保存到本地
					if config.GetSaveError() {
						mylog.ErrLogToFile("type", "PostGroupMessage")
						mylog.ErrInterfaceToFile("request", groupMessage)
						mylog.ErrLogToFile("error", err.Error())
					}
				}
			} else if err != nil && strings.Contains(err.Error(), "context deadline exceeded") {
				postGroupMessageWithRetry(apiv2, targetGroupID, groupMessage)
			}

			rememberLatestBotGroupMessageInGroup(targetGroupID, resp)
			if !config.GetNoRetMsg() {
				//发送成功回执
				if config.GetThreadsRetMsg() {
					if !config.GetStringOb11() {
						go SendResponse(client, err, &message, resp, api, apiv2)
					} else {
						go SendResponseSB(client, err, &message, resp, api, apiv2)
					}

				} else {
					if !config.GetStringOb11() {
						retmsg, _ = SendResponse(client, err, &message, resp, api, apiv2)
					} else {
						retmsg, _ = SendResponseSB(client, err, &message, resp, api, apiv2)
					}

				}
			}

		}
		var resp *dto.GroupMessageResponse
		// 遍历foundItems并发送每种信息
		   for key, urls := range foundItems {
		    // 跳过控制型 key，避免误发送空消息
		    if key == "active" || key == "active_type" || key == "active_sub_type" ||
		     key == "reply_msg_id" || key == "file_name" {
		     continue
		    }
		    for i, url := range urls {
				var singleItem = make(map[string][]string)
				singleItem[key] = []string{url} // 创建一个只包含一个 URL 的 singleItem
				// 如果存在 file_name，传递到 singleItem
				if fileNames, ok := foundItems["file_name"]; ok && i < len(fileNames) {
					singleItem["file_name"] = []string{fileNames[i]}
				}
				//mylog.Println("singleItem:", singleItem)
				msgseq := echo.GetMappingSeq(messageID)
				echo.AddMappingSeq(messageID, msgseq+1)
				groupReply := generateGroupMessage(messageID, eventID, singleItem, "", msgseq+1, apiv2, message.Params.GroupID.(string))
				// 进行类型断言
				richMediaMessage, ok := groupReply.(*dto.RichMediaMessage)
				if !ok {
					// 定义一个map来存储关键字
						keyMap := map[string]bool{
						       "markdown":      true,
						       "qqmusic":       true,
						       "local_image":   true,
							"local_record":  true,
							"url_image":     true,
							"url_images":    true,
							"base64_record": true,
							"base64_image":  true,
						       "local_video":   true,
						       "base64_video": true,
							"local_file":    true,
							"url_file":      true,
							"url_files":     true,
							"base64_file":   true,
						}
						// key是 for key, urls := range foundItems { 这里的key
						if _, exists := keyMap[key]; exists {
						// 进行类型断言
						groupMessage, ok := groupReply.(*dto.MessageToCreate)
						if !ok {
							mylog.Println("Error: Expected MessageToCreate type.")
							return "", nil // 或其他错误处理
						}
						// 将 reply 引用合并到 markdown 消息中
						       if replyIDs, ok := foundItems["reply_msg_id"]; ok && len(replyIDs) > 0 && key == "markdown" {
						        realReplyID, err := idmap.RetrieveRowByCachev2(replyIDs[0])
						        if err == nil && realReplyID != "" {
						         parts := strings.Split(realReplyID, " ")
						         refID := parts[len(parts)-1]
						         groupMessage.MessageReference = &dto.MessageReference{
						          MessageID:             refID,
						          IgnoreGetMessageError: true,
						         }
						         groupMessage.MsgID = refID
						         mylog.Printf("[CQ:reply] 设置 markdown 回复消息: msg_id=%s", refID)
						        }
						       }
						//重新为err赋值
						resp, err = apiv2.PostGroupMessage(context.TODO(), message.Params.GroupID.(string), groupMessage)
						if err != nil {
							mylog.Printf("发送 MessageToCreate 信息失败: %v", err)
							// 错误保存到本地
							if config.GetSaveError() {
								mylog.ErrLogToFile("type", "PostGroupMessage")
								mylog.ErrInterfaceToFile("request", groupMessage)
								mylog.ErrLogToFile("error", err.Error())
							}
						}
						if err != nil && strings.Contains(err.Error(), `"code":22009`) {
							mylog.Printf("信息发送失败,加入到队列中,下次被动信息进行发送")
							var pair echo.MessageGroupPair
							pair.Group = message.Params.GroupID.(string)
							pair.GroupMessage = groupMessage
							echo.PushGlobalStack(pair)
						} else if err != nil && strings.Contains(err.Error(), `"code":40034025`) {
							//请求参数event_id无效 重试
							groupMessage.EventID = ""
							//重新为err赋值
							resp, err = apiv2.PostGroupMessage(context.TODO(), message.Params.GroupID.(string), groupMessage)
							if err != nil {
								mylog.Printf("发送 MessageToCreate 信息失败 on code 40034025: %v", err)
								// 错误保存到本地
								if config.GetSaveError() {
									mylog.ErrLogToFile("type", "PostGroupMessage")
									mylog.ErrInterfaceToFile("request", groupMessage)
									mylog.ErrLogToFile("error", err.Error())
								}
							}
						} else if err != nil && strings.Contains(err.Error(), "context deadline exceeded") {
							postGroupMessageWithRetry(apiv2, message.Params.GroupID.(string), groupMessage)
						}

						rememberLatestBotGroupMessageInGroup(message.Params.GroupID.(string), resp)
						if !config.GetNoRetMsg() {
							//发送成功回执
							if config.GetThreadsRetMsg() {
								if !config.GetStringOb11() {
									go SendResponse(client, err, &message, resp, api, apiv2)
								} else {
									go SendResponseSB(client, err, &message, resp, api, apiv2)
								}

							} else {
								if !config.GetStringOb11() {
									retmsg, _ = SendResponse(client, err, &message, resp, api, apiv2)
								} else {
									retmsg, _ = SendResponseSB(client, err, &message, resp, api, apiv2)
								}

							}
						}
					}
					continue // 跳过这个项，继续下一个
				}
				message_return, err := apiv2.PostGroupMessage(context.TODO(), message.Params.GroupID.(string), richMediaMessage)
				if err != nil {
					mylog.Printf("发送 richMediaMessage 信息失败: %v", err)
					// 错误保存到本地
					if config.GetSaveError() {
						mylog.ErrLogToFile("type", "PostGroupMessage-richMediaMessage")
						mylog.ErrInterfaceToFile("request", richMediaMessage)
						mylog.ErrLogToFile("error", err.Error())
					}
				}
				if err != nil && (strings.Contains(err.Error(), "context deadline exceeded") || strings.Contains(err.Error(), "富媒体文件上传超时")) {
					message_return, err = postGroupRichMediaMessageWithRetry(apiv2, message.Params.GroupID.(string), richMediaMessage)
				}

				if message_return != nil && message_return.MediaResponse != nil && message_return.MediaResponse.FileInfo != "" {
					msgseq := echo.GetMappingSeq(messageID)
					echo.AddMappingSeq(messageID, msgseq+1)
					media := dto.Media{
						FileInfo: message_return.MediaResponse.FileInfo,
					}
					// 文件类型使用 RichMediaMessage 中的文件名，其他类型保持空格
					content := richMediaMessage.Content
					if content == "" {
						content = " "
					}
					groupMessage := &dto.MessageToCreate{
						Content: content,
						MsgID:   messageID,
						EventID: eventID,
						MsgSeq:  msgseq,
						MsgType: 7, // 默认文本类型
						Media:   &media,
					}
					groupMessage.Timestamp = time.Now().Unix() // 设置时间戳
					//重新为err赋值
					resp, err = apiv2.PostGroupMessage(context.TODO(), message.Params.GroupID.(string), groupMessage)
					if err != nil {
						mylog.Printf("发送图片失败: %v", err)
						// 错误保存到本地
						if config.GetSaveError() {
							mylog.ErrLogToFile("type", "PostGroupMessage")
							mylog.ErrInterfaceToFile("request", groupMessage)
							mylog.ErrLogToFile("error", err.Error())
						}
					}
					if err != nil && strings.Contains(err.Error(), `"code":22009`) {
						mylog.Printf("信息发送失败,加入到队列中,下次被动信息进行发送")
						var pair echo.MessageGroupPair
						pair.Group = message.Params.GroupID.(string)
						pair.GroupMessage = groupMessage
						echo.PushGlobalStack(pair)
					} else if err != nil && strings.Contains(err.Error(), `"code":40034025`) {
						groupMessage.EventID = ""
						resp, err = apiv2.PostGroupMessage(context.TODO(), message.Params.GroupID.(string), groupMessage)
						if err != nil {
							mylog.Printf("发送图片失败: %v", err)
						}
					} else if err != nil && strings.Contains(err.Error(), "context deadline exceeded") {
						postGroupMessageWithRetry(apiv2, message.Params.GroupID.(string), groupMessage)
					}
				}

				rememberLatestBotGroupMessageInGroup(message.Params.GroupID.(string), resp)
				if !config.GetNoRetMsg() {
					//发送成功回执
					if config.GetThreadsRetMsg() {
						if !config.GetStringOb11() {
							go SendResponse(client, err, &message, resp, api, apiv2)
						} else {
							go SendResponseSB(client, err, &message, resp, api, apiv2)
						}

					} else {
						if !config.GetStringOb11() {
							retmsg, _ = SendResponse(client, err, &message, resp, api, apiv2)
						} else {
							retmsg, _ = SendResponseSB(client, err, &message, resp, api, apiv2)
						}

					}
				}

			}
		}
	case "guild":
		//用GroupID给ChannelID赋值,因为我们是把频道虚拟成了群
		message.Params.ChannelID = message.Params.GroupID.(string)
		var RChannelID string
		if message.Params.UserID != nil && config.GetIdmapPro() && message.Params.UserID.(string) != "" && message.Params.UserID.(string) != "0" {
			RChannelID, _, err = idmap.RetrieveRowByIDv2Pro(message.Params.ChannelID.(string), message.Params.UserID.(string))
			mylog.Printf("测试,通过Proid获取的RChannelID:%v", RChannelID)
		}
		if RChannelID == "" {
			// 使用RetrieveRowByIDv2还原真实的ChannelID
			RChannelID, err = idmap.RetrieveRowByIDv2(message.Params.ChannelID.(string))
		}
		if err != nil {
			mylog.Printf("error retrieving real RChannelID: %v", err)
		}
		message.Params.ChannelID = RChannelID
		//这一句是group_private的逻辑,发频道信息用的是channelid
		//message.Params.GroupID = value
		retmsg, _ = HandleSendGuildChannelMsg(client, api, apiv2, message)
	case "guild_private":
		//用group_id还原出channelid 这是虚拟成群的私聊信息
		var RChannelID string
		var Vuserid string
		message.Params.ChannelID = message.Params.GroupID.(string)
		Vuserid, ok := message.Params.UserID.(string)
		if !ok {
			mylog.Printf("Error illegal UserID")
			return "", nil
		}
		if Vuserid != "" && config.GetIdmapPro() {
			RChannelID, _, err = idmap.RetrieveRowByIDv2Pro(message.Params.ChannelID.(string), Vuserid)
			mylog.Printf("测试,通过Proid获取的RChannelID:%v", RChannelID)
		} else {
			// 使用RetrieveRowByIDv2还原真实的ChannelID
			RChannelID, err = idmap.RetrieveRowByIDv2(message.Params.ChannelID.(string))
		}
		if err != nil {
			mylog.Printf("error retrieving real ChannelID: %v", err)
		}
		//读取ini 通过ChannelID取回之前储存的guild_id
		value, err := idmap.ReadConfigv2(RChannelID, "guild_id")
		if err != nil {
			mylog.Printf("Error reading config: %v", err)
			return "", nil
		}
		retmsg, _ = HandleSendGuildChannelPrivateMsg(client, api, apiv2, message, &value, &RChannelID)
	case "group_private":
		//用userid还原出openid 这是虚拟成群的群聊私聊信息
		if message.Params.GroupID != nil && message.Params.GroupID.(string) != "" {
			message.Params.UserID = message.Params.GroupID.(string)
		}
		retmsg, _ = HandleSendPrivateMsg(client, api, apiv2, message)
	case "forum":
		//用GroupID给ChannelID赋值,因为我们是把频道虚拟成了群
		message.Params.ChannelID = message.Params.GroupID.(string)
		var RChannelID string
		if message.Params.UserID != nil && config.GetIdmapPro() && message.Params.UserID.(string) != "" && message.Params.UserID.(string) != "0" {
			RChannelID, _, err = idmap.RetrieveRowByIDv2Pro(message.Params.ChannelID.(string), message.Params.UserID.(string))
			mylog.Printf("测试,通过Proid获取的RChannelID:%v", RChannelID)
		}
		if RChannelID == "" {
			// 使用RetrieveRowByIDv2还原真实的ChannelID
			RChannelID, err = idmap.RetrieveRowByIDv2(message.Params.ChannelID.(string))
		}
		if err != nil {
			mylog.Printf("error retrieving real RChannelID: %v", err)
		}
		message.Params.ChannelID = RChannelID
		//这一句是group_private的逻辑,发频道信息用的是channelid
		//message.Params.GroupID = value
		retmsg, _ = HandleSendGuildChannelForum(client, api, apiv2, message)
	default:
		mylog.Printf("Unknown message type: %s", msgType)
	}

	// stringob11不需要递归
	if !config.GetStringOb11() {
		// 如果递归id不是10(不递归特殊值)
		if echo.GetMapping(idInt64) != 10 {
			//重置递归类型 递归结束重置类型,避免下一次同样id,不同类型的请求被使用上一次类型
			if echo.GetMapping(idInt64) <= 0 {
				echo.AddMsgType(config.GetAppIDStr(), idInt64, "")
			}

			//减少递归计数器
			echo.AddMapping(idInt64, echo.GetMapping(idInt64)-1)

			//递归3次枚举类型
			if echo.GetMapping(idInt64) > 0 {
				tryMessageTypes := []string{"group", "guild", "guild_private"}
				messageCopy := message // 创建message的副本
				echo.AddMsgType(config.GetAppIDStr(), idInt64, tryMessageTypes[echo.GetMapping(idInt64)-1])
				delay := config.GetSendDelay()
				time.Sleep(time.Duration(delay) * time.Millisecond)
				retmsg, _ = HandleSendGroupMsg(client, api, apiv2, messageCopy)
			}
		}
	}

	return retmsg, nil
}

// 上传富媒体信息
// isPrivateOrLoopback 检查URL是否指向私有或回环地址（SSRF防护）
func isPrivateOrLoopback(rawURL string) bool {
	parsed, err := neturl.Parse(rawURL)
	if err != nil {
		return true // 解析失败则拒绝
	}
	host := parsed.Hostname()
	ip := net.ParseIP(host)
	if ip != nil {
		return ip.IsPrivate() || ip.IsLoopback() || ip.IsLinkLocalUnicast()
	}
	// 域名解析后检查
	ips, err := net.LookupHost(host)
	if err != nil {
		return true // 无法解析则拒绝
	}
	for _, ipStr := range ips {
		ip = net.ParseIP(ipStr)
		if ip != nil && (ip.IsPrivate() || ip.IsLoopback() || ip.IsLinkLocalUnicast()) {
			return true
		}
	}
	return false
}

// normalizeAndCheckSSRF 为 unknown 类型的 URL 补全协议头并执行 SSRF 校验。
// 返回补全后的完整 URL 和校验结果；校验失败时返回空字符串和 true。
func normalizeAndCheckSSRF(rawURL string) (string, bool) {
	if rawURL == "" {
		return "", true
	}
	// 补全协议头
	if !strings.HasPrefix(rawURL, "http://") && !strings.HasPrefix(rawURL, "https://") {
		rawURL = "http://" + rawURL
	}
	if isPrivateOrLoopback(rawURL) {
		return "", true
	}
	return rawURL, false
}

func generateGroupMessage(id string, eventid string, foundItems map[string][]string, messageText string, msgseq int, apiv2 openapi.OpenAPI, groupid string) interface{} {
	if imageURLs, ok := foundItems["local_image"]; ok && len(imageURLs) > 0 {
		// 从本地路径读取图片
		// 安全校验：防止路径穿越
		safePath, err := safeLocalPath(imageURLs[0], ".")
		if err != nil {
			mylog.Printf("安全校验失败，跳过本地图片: %v", err)
			return nil
		}
		imageData, err := os.ReadFile(safePath)
		if err != nil {
			// 读入文件失败
			mylog.Printf("Error reading the image from path %s: %v", imageURLs[0], err)
			// 返回文本信息，提示图片文件不存在
			return &dto.MessageToCreate{
				Content: "错误: 图片文件不存在",
				MsgID:   id,
				EventID: eventid,
				MsgSeq:  msgseq,
				MsgType: 0, // 默认文本类型
			}
		}
		// 首先压缩图片 默认不压缩
		compressedData, err := images.CompressSingleImage(imageData)
		if err != nil {
			mylog.Printf("Error compressing image: %v", err)
			return &dto.MessageToCreate{
				Content: "错误: 压缩图片失败",
				MsgID:   id,
				EventID: eventid,
				MsgSeq:  msgseq,
				MsgType: 0, // 默认文本类型
			}
		}

		// base64编码
		base64Encoded := base64.StdEncoding.EncodeToString(compressedData)

		// 直接 base64 上传图片到 QQ CDN
		messageToCreate, err := images.CreateAndUploadMediaMessage(context.TODO(), base64Encoded, eventid, 1, false, "", groupid, id, msgseq, apiv2)
		if err != nil {
			mylog.Printf("Error messageToCreate: %v", err)
			return &dto.MessageToCreate{
				Content: "错误: 上传图片失败",
				MsgID:   id,
				EventID: eventid,
				MsgSeq:  msgseq,
				MsgType: 0, // 默认文本类型
			}
		}
		return messageToCreate
	} else if RecordURLs, ok := foundItems["local_record"]; ok && len(RecordURLs) > 0 {
		// 从本地路径读取语音
		// 安全校验：防止路径穿越
		safePath, err := safeLocalPath(RecordURLs[0], ".")
		if err != nil {
			mylog.Printf("安全校验失败，跳过本地语音: %v", err)
			return nil
		}
		RecordData, err := os.ReadFile(safePath)
		if err != nil {
			// 读入文件失败
			mylog.Printf("Error reading the record from path %s: %v", RecordURLs[0], err)
			// 返回文本信息，提示语音文件不存在
			return &dto.MessageToCreate{
				Content: "错误: 语音文件不存在",
				MsgID:   id,
				EventID: eventid,
				MsgSeq:  msgseq,
				MsgType: 0, // 默认文本类型
			}
		}
		//判断并转码
		if !silk.IsAMRorSILK(RecordData) {
			mt, ok := silk.CheckAudio(bytes.NewReader(RecordData))
			if !ok {
				mylog.Errorf("voice type error: %s", mt)
				return nil
			}
			RecordData = silk.EncoderSilk(RecordData)
			mylog.Printf("音频转码ing")
		}

		if RecordData == nil || len(RecordData) == 0 {
			mylog.Errorf("语音转码失败，返回空数据")
			return &dto.MessageToCreate{
				Content: "错误: 语音转码失败，请检查 ffmpeg 是否安装",
				MsgID:   id,
				EventID: eventid,
				MsgSeq:  msgseq,
				MsgType: 0, // 默认文本类型
			}
		}

		base64Encoded := base64.StdEncoding.EncodeToString(RecordData)
		// 语音直接 base64 上传到 QQ CDN，不需要本地图床中转
		messageToCreate, err := images.CreateAndUploadMediaMessage(context.TODO(), base64Encoded, eventid, 3, false, "", groupid, id, msgseq, apiv2)
		if err != nil {
			mylog.Printf("Error messageToCreate: %v", err)
			return &dto.MessageToCreate{
				Content: "错误: 上传语音失败",
				MsgID:   id,
				EventID: eventid,
				MsgSeq:  msgseq,
				MsgType: 0, // 默认文本类型
			}
		}
		return messageToCreate
	} else if imageURLs, ok := foundItems["url_image"]; ok && len(imageURLs) > 0 {
		var newpiclink string
		if config.GetUrlPicTransfer() {
			// 从URL下载图片
			if isPrivateOrLoopback("http://" + imageURLs[0]) {
			mylog.Printf("SSRF 阻止: 目标地址为私有地址: http://%s", imageURLs[0])
			return nil
		}
		resp, err := httpClient.Get("http://" + imageURLs[0])
			if err != nil {
				mylog.Printf("Error downloading the image: %v", err)
				return &dto.MessageToCreate{
					Content: "错误: 下载图片失败",
					MsgID:   id,
					EventID: eventid,
					MsgSeq:  msgseq,
					MsgType: 0, // 默认文本类型
				}
			}
			defer resp.Body.Close()

			// 读取图片数据
			imageData, err := io.ReadAll(resp.Body)
			if err != nil {
				mylog.Printf("Error reading the image data: %v", err)
				return &dto.MessageToCreate{
					Content: "错误: 读取图片数据失败",
					MsgID:   id,
					EventID: eventid,
					MsgSeq:  msgseq,
					MsgType: 0,
				}
			}

			// 转换为base64
			base64Encoded := base64.StdEncoding.EncodeToString(imageData)

			// 直接 base64 上传到 QQ CDN
			messageToCreate, err := images.CreateAndUploadMediaMessage(context.TODO(), base64Encoded, eventid, 1, false, "", groupid, id, msgseq, apiv2)
			if err != nil {
				mylog.Printf("Error messageToCreate: %v", err)
				return &dto.MessageToCreate{
					Content: "错误: 上传图片失败",
					MsgID:   id,
					EventID: eventid,
					MsgSeq:  msgseq,
					MsgType: 0,
				}
			}
			return messageToCreate
		} else {
			newpiclink = "http://" + imageURLs[0]
		}

		// 发链接图片
		return &dto.RichMediaMessage{
			EventID:    id,
			FileType:   1,          // 1代表图片
			URL:        newpiclink, // 新图片链接
			Content:    "",         // 这个字段文档没有了
			SrvSendMsg: false,
		}
	} else if imageURLs, ok := foundItems["url_images"]; ok && len(imageURLs) > 0 {
		var newpiclink string
		if config.GetUrlPicTransfer() {
			// 从URL下载图片
			if isPrivateOrLoopback("https://" + imageURLs[0]) {
			mylog.Printf("SSRF 阻止: 目标地址为私有地址: https://%s", imageURLs[0])
			return nil
		}
		resp, err := httpClient.Get("https://" + imageURLs[0])
			if err != nil {
				mylog.Printf("Error downloading the image: %v", err)
				return &dto.MessageToCreate{
					Content: "错误: 下载图片失败",
					MsgID:   id,
					EventID: eventid,
					MsgSeq:  msgseq,
					MsgType: 0, // 默认文本类型
				}
			}
			defer resp.Body.Close()

			// 读取图片数据
			imageData, err := io.ReadAll(resp.Body)
			if err != nil {
				mylog.Printf("Error reading the image data: %v", err)
				return &dto.MessageToCreate{
					Content: "错误: 读取图片数据失败",
					MsgID:   id,
					EventID: eventid,
					MsgSeq:  msgseq,
					MsgType: 0,
				}
			}

			// 转换为base64
			base64Encoded := base64.StdEncoding.EncodeToString(imageData)

			// 直接 base64 上传到 QQ CDN
			messageToCreate, err := images.CreateAndUploadMediaMessage(context.TODO(), base64Encoded, eventid, 1, false, "", groupid, id, msgseq, apiv2)
			if err != nil {
				mylog.Printf("Error messageToCreate: %v", err)
				return &dto.MessageToCreate{
					Content: "错误: 上传图片失败",
					MsgID:   id,
					EventID: eventid,
					MsgSeq:  msgseq,
					MsgType: 0, // 默认文本类型
				}
			}
			return messageToCreate
		} else {
			newpiclink = "https://" + imageURLs[0]
		}

		// 发链接图片
		return &dto.RichMediaMessage{
			EventID:    id,
			FileType:   1,          // 1代表图片
			URL:        newpiclink, // 新图片链接
			Content:    "",         // 这个字段文档没有了
			SrvSendMsg: false,
		}
	} else if base64URLs, ok := foundItems["base64_record"]; ok && len(base64URLs) > 0 {
		// 适配base64 slik
		// 解码base64语音数据
		fileRecordData, err := base64.StdEncoding.DecodeString(base64URLs[0])
		if err != nil {
			mylog.Printf("failed to decode base64 record: %v", err)
			return nil
		}
		//判断并转码
		if !silk.IsAMRorSILK(fileRecordData) {
			mt, ok := silk.CheckAudio(bytes.NewReader(fileRecordData))
			if !ok {
				mylog.Errorf("voice type error: %s", mt)
				return nil
			}
			fileRecordData = silk.EncoderSilk(fileRecordData)
			mylog.Printf("音频转码ing")
		}
		if len(fileRecordData) == 0 {
			mylog.Errorf("语音转码失败，返回空数据")
			return &dto.MessageToCreate{
				Content: "错误: 语音转码失败，请检查 ffmpeg 是否安装",
				MsgID:   id,
				EventID: eventid,
				MsgSeq:  msgseq,
				MsgType: 0, // 默认文本类型
			}
		}
		base64Encoded := base64.StdEncoding.EncodeToString(fileRecordData)
		// 语音直接 base64 上传到 QQ CDN，不需要本地图床中转
		messageToCreate, err := images.CreateAndUploadMediaMessage(context.TODO(), base64Encoded, eventid, 3, false, "", groupid, id, msgseq, apiv2)
		if err != nil {
			mylog.Printf("Error messageToCreate: %v", err)
			return &dto.MessageToCreate{
				Content: "错误: 上传语音失败",
				MsgID:   id,
				EventID: eventid,
				MsgSeq:  msgseq,
				MsgType: 0, // 默认文本类型
			}
		}
		return messageToCreate
	} else if recordURLs, ok := foundItems["url_record"]; ok && len(recordURLs) > 0 {
		// 从URL下载语音
		if isPrivateOrLoopback("http://" + recordURLs[0]) {
		mylog.Printf("SSRF 阻止: 目标地址为私有地址: http://%s", recordURLs[0])
			return nil
		}
		resp, err := httpClient.Get("http://" + recordURLs[0])
		if err != nil {
			mylog.Printf("Error downloading the record: %v", err)
			return &dto.MessageToCreate{
				Content: "错误: 下载语音失败",
				MsgID:   id,
				MsgSeq:  msgseq,
				MsgType: 0, // 默认文本类型
			}
		}
		defer resp.Body.Close()

		// 读取语音数据
		recordData, err := io.ReadAll(resp.Body)
		if err != nil {
			mylog.Printf("Error reading the record data: %v", err)
			return &dto.MessageToCreate{
				Content: "错误: 读取语音数据失败",
				MsgID:   id,
				EventID: eventid,
				MsgSeq:  msgseq,
				MsgType: 0,
			}
		}
		//判断并转码
		if !silk.IsAMRorSILK(recordData) {
			mt, ok := silk.CheckAudio(bytes.NewReader(recordData))
			if !ok {
				mylog.Errorf("voice type error: %s", mt)
				return nil
			}
			recordData = silk.EncoderSilk(recordData)
			mylog.Printf("音频转码ing")
		}
		// 转换为base64
		base64Encoded := base64.StdEncoding.EncodeToString(recordData)

		// 上传语音并获取新的URL
		newURL, err := images.UploadBase64RecordToServer(base64Encoded)
		if err != nil {
			mylog.Printf("Error uploading base64 encoded image: %v", err)
			return &dto.MessageToCreate{
				Content: "错误: 上传语音失败",
				MsgID:   id,
				EventID: eventid,
				MsgSeq:  msgseq,
				MsgType: 0,
			}
		}

		// 发链接语音
		return &dto.RichMediaMessage{
			EventID:    id,
			FileType:   3,      // 3代表语音
			URL:        newURL, // 新语音链接
			Content:    "",     // 这个字段文档没有了
			SrvSendMsg: false,
		}
	} else if recordURLs, ok := foundItems["url_records"]; ok && len(recordURLs) > 0 {
// 从URL下载语音
if isPrivateOrLoopback("https://" + recordURLs[0]) {
		mylog.Printf("SSRF 阻止: 目标地址为私有地址: https://%s", recordURLs[0])
			return nil
		}
		resp, err := httpClient.Get("https://" + recordURLs[0])
		if err != nil {
			mylog.Printf("Error downloading the record: %v", err)
			return &dto.MessageToCreate{
				Content: "错误: 下载语音失败",
				MsgID:   id,
				EventID: eventid,
				MsgSeq:  msgseq,
				MsgType: 0, // 默认文本类型
			}
		}
		defer resp.Body.Close()

		// 读取语音数据
		recordData, err := io.ReadAll(resp.Body)
		if err != nil {
			mylog.Printf("Error reading the record data: %v", err)
			return &dto.MessageToCreate{
				Content: "错误: 读取语音数据失败",
				MsgID:   id,
				EventID: eventid,
				MsgSeq:  msgseq,
				MsgType: 0,
			}
		}
		//判断并转码
		if !silk.IsAMRorSILK(recordData) {
			mt, ok := silk.CheckAudio(bytes.NewReader(recordData))
			if !ok {
				mylog.Errorf("voice type error: %s", mt)
				return nil
			}
			recordData = silk.EncoderSilk(recordData)
			mylog.Printf("音频转码ing")
		}
		// 转换为base64
		base64Encoded := base64.StdEncoding.EncodeToString(recordData)

		// 上传语音并获取新的URL
		newURL, err := images.UploadBase64RecordToServer(base64Encoded)
		if err != nil {
			mylog.Printf("Error uploading base64 encoded image: %v", err)
			return &dto.MessageToCreate{
				Content: "错误: 上传语音失败",
				MsgID:   id,
				EventID: eventid,
				MsgSeq:  msgseq,
				MsgType: 0,
			}
		}

		// 发链接语音
		return &dto.RichMediaMessage{
			EventID:    id,
			FileType:   3,      // 3代表语音
			URL:        newURL, // 新语音链接
			Content:    "",     // 这个字段文档没有了
			SrvSendMsg: false,
		}
	} else if base64Image, ok := foundItems["base64_image"]; ok && len(base64Image) > 0 {
		// todo 适配base64图片
		//因为QQ群没有 form方式上传,所以在gensokyo内置了图床,需公网,或以lotus方式连接位于公网的gensokyo
		//要正确的开放对应的端口和设置正确的ip地址在config,这对于一般用户是有一些难度的
		// 解码base64图片数据
		fileImageData, err := base64.StdEncoding.DecodeString(base64Image[0])
		if err != nil {
			mylog.Printf("failed to decode base64 image: %v", err)
			return nil
		}

		// 首先压缩图片 默认不压缩
		compressedData, err := images.CompressSingleImage(fileImageData)
		if err != nil {
			mylog.Printf("Error compressing image: %v", err)
			return &dto.MessageToCreate{
				Content: "错误: 压缩图片失败",
				MsgID:   id,
				EventID: eventid,
				MsgSeq:  msgseq,
				MsgType: 0, // 默认文本类型
			}
		}

		base64Encoded := base64.StdEncoding.EncodeToString(compressedData)
		// 直接 base64 上传到 QQ CDN
		messageToCreate, err := images.CreateAndUploadMediaMessage(context.TODO(), base64Encoded, eventid, 1, false, "", groupid, id, msgseq, apiv2)
		if err != nil {
			mylog.Printf("Error messageToCreate: %v", err)
			return &dto.MessageToCreate{
				Content: "错误: 上传图片失败",
				MsgID:   id,
				EventID: eventid,
				MsgSeq:  msgseq,
				MsgType: 0, // 默认文本类型
			}
		}
		return messageToCreate
	} else if mdContent, ok := foundItems["markdown"]; ok && len(mdContent) > 0 {
		// 解码base64 markdown数据
		mdData, err := base64.StdEncoding.DecodeString(mdContent[0])
		if err != nil {
			mylog.Printf("failed to decode base64 md: %v", err)
			return nil
		}
		markdown, keyboard, err := parseMDData(mdData)
		if err != nil {
			mylog.Printf("failed to parseMDData: %v", err)
			return nil
		}

		if config.GetUnionID() && markdown != nil && markdown.Content != "" {
			markdown.Content = mdutil.ReplaceQQBotAtUserIDUnionToRaw(markdown.Content)
		}

		// 将 markdown 内容中的 CQ at 码转换为 QQ @ 语法
		if markdown != nil && markdown.Content != "" {
			markdown.Content = ResolveMarkdownAtMentions(markdown.Content)
			markdown.Content = ResolveMarkdownImages(markdown.Content, apiv2)
		}

		return &dto.MessageToCreate{
			Content:  "markdown",
			MsgID:    id,
			EventID:  eventid,
			MsgSeq:   msgseq,
			Markdown: markdown,
			Keyboard: keyboard,
			MsgType:  2,
		}
	} else if qqmusic, ok := foundItems["qqmusic"]; ok && len(qqmusic) > 0 {
		// 转换qq音乐id到一个md
		music_id := qqmusic[0]
		markdown, keyboard, err := parseQQMuiscMDData(music_id)
		if err != nil {
			mylog.Printf("failed to parseMDData: %v", err)
			return nil
		}
		if markdown != nil {
			return &dto.MessageToCreate{
				Content:  "markdown",
				MsgID:    id,
				EventID:  eventid,
				MsgSeq:   msgseq,
				Markdown: markdown,
				Keyboard: keyboard,
				MsgType:  2,
			}
		} else {
			return &dto.MessageToCreate{
				Content:  "markdown",
				MsgID:    id,
				EventID:  eventid,
				MsgSeq:   msgseq,
				Keyboard: keyboard,
				MsgType:  2,
			}
		}
	} else if videoURL, ok := foundItems["url_video"]; ok && len(videoURL) > 0 {
		newvideolink := "http://" + videoURL[0]
		// 发链接视频 http
		return &dto.RichMediaMessage{
			EventID:    id,
			FileType:   2,            // 2代表视频
			URL:        newvideolink, // 新图片链接
			Content:    "",           // 这个字段文档没有了
			SrvSendMsg: false,
		}
	} else if videoURLs, ok := foundItems["url_videos"]; ok && len(videoURLs) > 0 {
		newvideolink := "https://" + videoURLs[0]
		// 发链接视频 https
		return &dto.RichMediaMessage{
			EventID:    id,
			FileType:   2,            // 2代表视频
			URL:        newvideolink, // 新图片链接
			Content:    "",           // 这个字段文档没有了
			SrvSendMsg: false,
		}
	} else if base64Videos, ok := foundItems["base64_video"]; ok && len(base64Videos) > 0 {
		// base64 视频数据上传到 QQ CDN（FileType=2 视频）
		messageToCreate, err := images.CreateAndUploadMediaMessage(context.TODO(), base64Videos[0], eventid, 2, false, "", groupid, id, msgseq, apiv2)
		if err != nil {
			mylog.Printf("Error uploading base64 video: %v", err)
			return &dto.MessageToCreate{
				Content: "错误: 上传视频失败",
				MsgID:   id,
				EventID: eventid,
				MsgSeq:  msgseq,
				MsgType: 0,
			}
		}
		return messageToCreate
	} else if localVideos, ok := foundItems["local_video"]; ok && len(localVideos) > 0 {
		// 从本地路径读取视频并 base64 上传到 QQ CDN
		safePath, err := safeLocalPath(localVideos[0], ".")
		if err != nil {
			mylog.Printf("安全校验失败，跳过本地视频: %v", err)
			return nil
		}
		videoData, err := os.ReadFile(safePath)
		if err != nil {
			mylog.Printf("Error reading the video from path %s: %v", localVideos[0], err)
			return &dto.MessageToCreate{
				Content: "错误: 视频文件不存在",
				MsgID:   id,
				EventID: eventid,
				MsgSeq:  msgseq,
				MsgType: 0,
			}
		}
		base64Encoded := base64.StdEncoding.EncodeToString(videoData)
		messageToCreate, err := images.CreateAndUploadMediaMessage(context.TODO(), base64Encoded, eventid, 2, false, "", groupid, id, msgseq, apiv2)
		if err != nil {
			mylog.Printf("Error uploading local video: %v", err)
			return &dto.MessageToCreate{
				Content: "错误: 上传视频失败",
				MsgID:   id,
				EventID: eventid,
				MsgSeq:  msgseq,
				MsgType: 0,
			}
		}
		return messageToCreate
	} else if fileURLs, ok := foundItems["local_file"]; ok && len(fileURLs) > 0 {
		// 从本地路径读取文件
		// 安全校验：防止路径穿越
		safePath, err := safeLocalPath(fileURLs[0], ".")
		if err != nil {
			mylog.Printf("安全校验失败，跳过本地文件: %v", err)
			return nil
		}
		fileData, err := os.ReadFile(safePath)
		if err != nil {
			mylog.Printf("Error reading the file from path %s: %v", fileURLs[0], err)
			return &dto.MessageToCreate{
				Content: "错误: 文件不存在",
				MsgID:   id,
				EventID: eventid,
				MsgSeq:  msgseq,
				MsgType: 0,
			}
		}
		// 提取文件名，优先使用 file_name 参数，否则自动从路径提取
		fileName := ""
		if fns, ok := foundItems["file_name"]; ok && len(fns) > 0 && fns[0] != "" {
			fileName = fns[0]
		} else {
			fileName = filepath.Base(fileURLs[0])
		}
		// base64编码
		base64Encoded := base64.StdEncoding.EncodeToString(fileData)
		// 直接 base64 上传文件到 QQ CDN，文件名作为 content 传递
		messageToCreate, err := images.CreateAndUploadMediaMessage(context.TODO(), base64Encoded, eventid, 4, false, fileName, groupid, id, msgseq, apiv2)
		if err != nil {
			mylog.Printf("Error uploading file: %v", err)
			return &dto.MessageToCreate{
				Content: "错误: 上传文件失败",
				MsgID:   id,
				EventID: eventid,
				MsgSeq:  msgseq,
				MsgType: 0,
			}
		}
		return messageToCreate
	} else if fileURLs, ok := foundItems["url_file"]; ok && len(fileURLs) > 0 {
		// 提取文件名，优先使用 file_name 参数，否则自动从 URL 提取
		fileName := ""
		if fns, ok := foundItems["file_name"]; ok && len(fns) > 0 && fns[0] != "" {
			fileName = fns[0]
		} else {
			fileName = filepath.Base(fileURLs[0])
			if fileName == "." || fileName == "/" {
				fileName = ""
			}
		}
		// 发链接文件 http
		return &dto.RichMediaMessage{
			EventID:    id,
			FileType:   4,           // 4代表文件
			URL:        "http://" + fileURLs[0],
			Content:    fileName,
			FileName:   fileName,
			SrvSendMsg: false,
		}
	} else if fileURLs, ok := foundItems["url_files"]; ok && len(fileURLs) > 0 {
		// 提取文件名，优先使用 file_name 参数，否则自动从 URL 提取
		fileName := ""
		if fns, ok := foundItems["file_name"]; ok && len(fns) > 0 && fns[0] != "" {
			fileName = fns[0]
		} else {
			fileName = filepath.Base(fileURLs[0])
			if fileName == "." || fileName == "/" {
				fileName = ""
			}
		}
		// 发链接文件 https
		return &dto.RichMediaMessage{
			EventID:    id,
			FileType:   4,           // 4代表文件
			URL:        "https://" + fileURLs[0],
			Content:    fileName,
			FileName:   fileName,
			SrvSendMsg: false,
		}
	} else if base64Files, ok := foundItems["base64_file"]; ok && len(base64Files) > 0 {
	   // base64文件数据上传到 QQ CDN
	   messageToCreate, err := images.CreateAndUploadMediaMessage(context.TODO(), base64Files[0], eventid, 4, false, "", groupid, id, msgseq, apiv2)
	   if err != nil {
	    mylog.Printf("Error uploading base64 file: %v", err)
	    return &dto.MessageToCreate{
	     Content: "错误: 上传文件失败",
	     MsgID:   id,
	     EventID: eventid,
	     MsgSeq:  msgseq,
	     MsgType: 0,
	    }
	   }
	   return messageToCreate
	  } else if unknownImages, ok := foundItems["unknown_image"]; ok && len(unknownImages) > 0 {
	   // 无前缀图片，尝试作为 URL 处理
	   mylog.Printf("unknown_image 作为 URL 图片尝试发送: %s", unknownImages[0])
	   safeURL, blocked := normalizeAndCheckSSRF(unknownImages[0])
	   if blocked {
	    mylog.Printf("SSRF 阻止: unknown_image 目标地址不安全: %s", unknownImages[0])
	    return nil
	   }
	   return &dto.RichMediaMessage{
	    EventID:    id,
	    FileType:   1,
	    URL:        safeURL,
	    Content:    "",
	    SrvSendMsg: false,
	   }
	  } else if unknownRecords, ok := foundItems["unknown_record"]; ok && len(unknownRecords) > 0 {
	   // 无前缀语音，尝试作为 URL 处理
	   mylog.Printf("unknown_record 作为 URL 语音尝试发送: %s", unknownRecords[0])
	   safeURL, blocked := normalizeAndCheckSSRF(unknownRecords[0])
	   if blocked {
	    mylog.Printf("SSRF 阻止: unknown_record 目标地址不安全: %s", unknownRecords[0])
	    return nil
	   }
	   return &dto.RichMediaMessage{
	    EventID:    id,
	    FileType:   3,
	    URL:        safeURL,
	    Content:    "",
	    SrvSendMsg: false,
	   }
	  } else if unknownFiles, ok := foundItems["unknown_file"]; ok && len(unknownFiles) > 0 {
	   // 无前缀文件，尝试作为 URL 处理
	   fileName := ""
	   if fns, ok := foundItems["file_name"]; ok && len(fns) > 0 && fns[0] != "" {
	    fileName = fns[0]
	   }
	   mylog.Printf("unknown_file 作为 URL 文件尝试发送: %s", unknownFiles[0])
	   safeURL, blocked := normalizeAndCheckSSRF(unknownFiles[0])
	   if blocked {
	    mylog.Printf("SSRF 阻止: unknown_file 目标地址不安全: %s", unknownFiles[0])
	    return nil
	   }
	   return &dto.RichMediaMessage{
	    EventID:    id,
	    FileType:   4,
	    URL:        safeURL,
	    Content:    fileName,
	    FileName:   fileName,
	    SrvSendMsg: false,
	   }
	  } else {
	   // 返回文本信息
	   return &dto.MessageToCreate{
	    Content: messageText,
	    MsgID:   id,
	    EventID: eventid,
	    MsgSeq:  msgseq,
	    MsgType: 0, // 默认文本类型
	   }
	  }
	 }

	 // 上传富媒体信息
func generatePrivateMessage(id string, eventid string, foundItems map[string][]string, messageText string, msgseq int, apiv2 openapi.OpenAPI, userid string) interface{} {
	if imageURLs, ok := foundItems["local_image"]; ok && len(imageURLs) > 0 {
		// 从本地路径读取图片
		// 安全校验：防止路径穿越
		safePath, err := safeLocalPath(imageURLs[0], ".")
		if err != nil {
			mylog.Printf("安全校验失败，跳过本地图片: %v", err)
			return nil
		}
		imageData, err := os.ReadFile(safePath)
		if err != nil {
			// 读入文件失败
			mylog.Printf("Error reading the image from path %s: %v", imageURLs[0], err)
			// 返回文本信息，提示图片文件不存在
			return &dto.MessageToCreate{
				Content: "错误: 图片文件不存在",
				MsgID:   id,
				EventID: eventid,
				MsgSeq:  msgseq,
				MsgType: 0, // 默认文本类型
			}
		}
		// 首先压缩图片 默认不压缩
		compressedData, err := images.CompressSingleImage(imageData)
		if err != nil {
			mylog.Printf("Error compressing image: %v", err)
			return &dto.MessageToCreate{
				Content: "错误: 压缩图片失败",
				MsgID:   id,
				EventID: eventid,
				MsgSeq:  msgseq,
				MsgType: 0, // 默认文本类型
			}
		}

		// base64编码
		base64Encoded := base64.StdEncoding.EncodeToString(compressedData)

		// 直接 base64 上传到 QQ CDN
		messageToCreate, err := images.CreateAndUploadMediaMessagePrivate(context.TODO(), base64Encoded, eventid, 1, false, "", userid, id, msgseq, apiv2)
		if err != nil {
			mylog.Printf("Error messageToCreate: %v", err)
			return &dto.MessageToCreate{
				Content: "错误: 上传图片失败",
				MsgID:   id,
				EventID: eventid,
				MsgSeq:  msgseq,
				MsgType: 0, // 默认文本类型
			}
		}
		return messageToCreate
	} else if RecordURLs, ok := foundItems["local_record"]; ok && len(RecordURLs) > 0 {
		// 从本地路径读取语音
		// 安全校验：防止路径穿越
		safePath, err := safeLocalPath(RecordURLs[0], ".")
		if err != nil {
			mylog.Printf("安全校验失败，跳过本地语音: %v", err)
			return nil
		}
		RecordData, err := os.ReadFile(safePath)
		if err != nil {
			// 读入文件失败
			mylog.Printf("Error reading the record from path %s: %v", RecordURLs[0], err)
			// 返回文本信息，提示语音文件不存在
			return &dto.MessageToCreate{
				Content: "错误: 语音文件不存在",
				MsgID:   id,
				EventID: eventid,
				MsgSeq:  msgseq,
				MsgType: 0, // 默认文本类型
			}
		}
		//判断并转码
		if !silk.IsAMRorSILK(RecordData) {
			mt, ok := silk.CheckAudio(bytes.NewReader(RecordData))
			if !ok {
				mylog.Errorf("voice type error: %s", mt)
				return nil
			}
			RecordData = silk.EncoderSilk(RecordData)
			mylog.Printf("音频转码ing")
		}

		if RecordData == nil || len(RecordData) == 0 {
			mylog.Errorf("语音转码失败，返回空数据")
			return &dto.MessageToCreate{
				Content: "错误: 语音转码失败，请检查 ffmpeg 是否安装",
				MsgID:   id,
				EventID: eventid,
				MsgSeq:  msgseq,
				MsgType: 0, // 默认文本类型
			}
		}

		base64Encoded := base64.StdEncoding.EncodeToString(RecordData)
		// 语音直接 base64 上传到 QQ CDN，不需要本地图床中转
		messageToCreate, err := images.CreateAndUploadMediaMessagePrivate(context.TODO(), base64Encoded, eventid, 3, false, "", userid, id, msgseq, apiv2)
		if err != nil {
			mylog.Printf("Error messageToCreate: %v", err)
			return &dto.MessageToCreate{
				Content: "错误: 上传语音失败",
				MsgID:   id,
				EventID: eventid,
				MsgSeq:  msgseq,
				MsgType: 0, // 默认文本类型
			}
		}
		return messageToCreate
	} else if imageURLs, ok := foundItems["url_image"]; ok && len(imageURLs) > 0 {
		var newpiclink string
		if config.GetUrlPicTransfer() {
			// 从URL下载图片
			if isPrivateOrLoopback("http://" + imageURLs[0]) {
			mylog.Printf("SSRF 阻止: 目标地址为私有地址: http://%s", imageURLs[0])
			return nil
		}
		resp, err := httpClient.Get("http://" + imageURLs[0])
			if err != nil {
				mylog.Printf("Error downloading the image: %v", err)
				return &dto.MessageToCreate{
					Content: "错误: 下载图片失败",
					MsgID:   id,
					EventID: eventid,
					MsgSeq:  msgseq,
					MsgType: 0, // 默认文本类型
				}
			}
			defer resp.Body.Close()

			// 读取图片数据
			imageData, err := io.ReadAll(resp.Body)
			if err != nil {
				mylog.Printf("Error reading the image data: %v", err)
				return &dto.MessageToCreate{
					Content: "错误: 读取图片数据失败",
					MsgID:   id,
					EventID: eventid,
					MsgSeq:  msgseq,
					MsgType: 0,
				}
			}

			// 转换为base64
			base64Encoded := base64.StdEncoding.EncodeToString(imageData)

			// 直接 base64 上传到 QQ CDN
			messageToCreate, err := images.CreateAndUploadMediaMessagePrivate(context.TODO(), base64Encoded, eventid, 1, false, "", userid, id, msgseq, apiv2)
			if err != nil {
				mylog.Printf("Error messageToCreate: %v", err)
				return &dto.MessageToCreate{
					Content: "错误: 上传图片失败",
					MsgID:   id,
					EventID: eventid,
					MsgSeq:  msgseq,
					MsgType: 0, // 默认文本类型
				}
			}
			return messageToCreate
		} else {
			newpiclink = "http://" + imageURLs[0]
		}

		// 发链接图片
		return &dto.RichMediaMessage{
			EventID:    id,
			FileType:   1,          // 1代表图片
			URL:        newpiclink, // 新图片链接
			Content:    "",         // 这个字段文档没有了
			SrvSendMsg: false,
		}
	} else if imageURLs, ok := foundItems["url_images"]; ok && len(imageURLs) > 0 {
		var newpiclink string
		if config.GetUrlPicTransfer() {
			// 从URL下载图片
			if isPrivateOrLoopback("https://" + imageURLs[0]) {
			mylog.Printf("SSRF 阻止: 目标地址为私有地址: https://%s", imageURLs[0])
			return nil
		}
		resp, err := httpClient.Get("https://" + imageURLs[0])
			if err != nil {
				mylog.Printf("Error downloading the image: %v", err)
				return &dto.MessageToCreate{
					Content: "错误: 下载图片失败",
					MsgID:   id,
					EventID: eventid,
					MsgSeq:  msgseq,
					MsgType: 0, // 默认文本类型
				}
			}
			defer resp.Body.Close()

			// 读取图片数据
			imageData, err := io.ReadAll(resp.Body)
			if err != nil {
				mylog.Printf("Error reading the image data: %v", err)
				return &dto.MessageToCreate{
					Content: "错误: 读取图片数据失败",
					MsgID:   id,
					EventID: eventid,
					MsgSeq:  msgseq,
					MsgType: 0,
				}
			}

			// 转换为base64
			base64Encoded := base64.StdEncoding.EncodeToString(imageData)

			// 直接 base64 上传到 QQ CDN
			messageToCreate, err := images.CreateAndUploadMediaMessagePrivate(context.TODO(), base64Encoded, eventid, 1, false, "", userid, id, msgseq, apiv2)
			if err != nil {
				mylog.Printf("Error messageToCreate: %v", err)
				return &dto.MessageToCreate{
					Content: "错误: 上传图片失败",
					MsgID:   id,
					EventID: eventid,
					MsgSeq:  msgseq,
					MsgType: 0, // 默认文本类型
				}
			}
			return messageToCreate
		} else {
			newpiclink = "https://" + imageURLs[0]
		}

		// 发链接图片
		return &dto.RichMediaMessage{
			EventID:    id,
			FileType:   1,          // 1代表图片
			URL:        newpiclink, // 新图片链接
			Content:    "",         // 这个字段文档没有了
			SrvSendMsg: false,
		}
	} else if base64URLs, ok := foundItems["base64_record"]; ok && len(base64URLs) > 0 {
		// 适配base64 slik
		// 解码base64语音数据
		fileRecordData, err := base64.StdEncoding.DecodeString(base64URLs[0])
			if err != nil {
				mylog.Printf("failed to decode base64 record: %v", err)
				return nil
			}
			//判断并转码
			if !silk.IsAMRorSILK(fileRecordData) {
				mt, ok := silk.CheckAudio(bytes.NewReader(fileRecordData))
				if !ok {
					mylog.Errorf("voice type error: %s", mt)
					return nil
				}
				fileRecordData = silk.EncoderSilk(fileRecordData)
				mylog.Printf("音频转码ing")
			}
			if len(fileRecordData) == 0 {
				mylog.Errorf("语音转码失败，返回空数据")
				return &dto.MessageToCreate{
					Content: "错误: 语音转码失败，请检查 ffmpeg 是否安装",
					MsgID:   id,
					EventID: eventid,
					MsgSeq:  msgseq,
					MsgType: 0, // 默认文本类型
				}
			}
			base64Encoded := base64.StdEncoding.EncodeToString(fileRecordData)
			// 语音直接 base64 上传到 QQ CDN，不需要本地图床中转
			messageToCreate, err := images.CreateAndUploadMediaMessagePrivate(context.TODO(), base64Encoded, eventid, 3, false, "", userid, id, msgseq, apiv2)
			if err != nil {
				mylog.Printf("Error messageToCreate: %v", err)
				return &dto.MessageToCreate{
					Content: "错误: 上传语音失败",
					MsgID:   id,
					EventID: eventid,
					MsgSeq:  msgseq,
					MsgType: 0, // 默认文本类型
				}
			}
			return messageToCreate
	} else if recordURLs, ok := foundItems["url_record"]; ok && len(recordURLs) > 0 {
		// 从URL下载语音
		if isPrivateOrLoopback("http://" + recordURLs[0]) {
		mylog.Printf("SSRF 阻止: 目标地址为私有地址: http://%s", recordURLs[0])
			return nil
		}
		resp, err := httpClient.Get("http://" + recordURLs[0])
		if err != nil {
			mylog.Printf("Error downloading the record: %v", err)
			return &dto.MessageToCreate{
				Content: "错误: 下载语音失败",
				MsgID:   id,
				MsgSeq:  msgseq,
				MsgType: 0, // 默认文本类型
			}
		}
		defer resp.Body.Close()

		// 读取语音数据
		recordData, err := io.ReadAll(resp.Body)
		if err != nil {
			mylog.Printf("Error reading the record data: %v", err)
			return &dto.MessageToCreate{
				Content: "错误: 读取语音数据失败",
				MsgID:   id,
				EventID: eventid,
				MsgSeq:  msgseq,
				MsgType: 0,
			}
		}
		//判断并转码
		if !silk.IsAMRorSILK(recordData) {
			mt, ok := silk.CheckAudio(bytes.NewReader(recordData))
			if !ok {
				mylog.Errorf("voice type error: %s", mt)
				return nil
			}
			recordData = silk.EncoderSilk(recordData)
			mylog.Printf("音频转码ing")
		}
		// 转换为base64
		base64Encoded := base64.StdEncoding.EncodeToString(recordData)

		// 上传语音并获取新的URL
		newURL, err := images.UploadBase64RecordToServer(base64Encoded)
		if err != nil {
			mylog.Printf("Error uploading base64 encoded image: %v", err)
			return &dto.MessageToCreate{
				Content: "错误: 上传语音失败",
				MsgID:   id,
				EventID: eventid,
				MsgSeq:  msgseq,
				MsgType: 0,
			}
		}

		// 发链接语音
		return &dto.RichMediaMessage{
			EventID:    id,
			FileType:   3,      // 3代表语音
			URL:        newURL, // 新语音链接
			Content:    "",     // 这个字段文档没有了
			SrvSendMsg: false,
		}
	} else if recordURLs, ok := foundItems["url_records"]; ok && len(recordURLs) > 0 {
// 从URL下载语音
if isPrivateOrLoopback("https://" + recordURLs[0]) {
		mylog.Printf("SSRF 阻止: 目标地址为私有地址: https://%s", recordURLs[0])
			return nil
		}
		resp, err := httpClient.Get("https://" + recordURLs[0])
		if err != nil {
			mylog.Printf("Error downloading the record: %v", err)
			return &dto.MessageToCreate{
				Content: "错误: 下载语音失败",
				MsgID:   id,
				EventID: eventid,
				MsgSeq:  msgseq,
				MsgType: 0, // 默认文本类型
			}
		}
		defer resp.Body.Close()

		// 读取语音数据
		recordData, err := io.ReadAll(resp.Body)
		if err != nil {
			mylog.Printf("Error reading the record data: %v", err)
			return &dto.MessageToCreate{
				Content: "错误: 读取语音数据失败",
				MsgID:   id,
				EventID: eventid,
				MsgSeq:  msgseq,
				MsgType: 0,
			}
		}
		//判断并转码
		if !silk.IsAMRorSILK(recordData) {
			mt, ok := silk.CheckAudio(bytes.NewReader(recordData))
			if !ok {
				mylog.Errorf("voice type error: %s", mt)
				return nil
			}
			recordData = silk.EncoderSilk(recordData)
			mylog.Printf("音频转码ing")
		}
		// 转换为base64
		base64Encoded := base64.StdEncoding.EncodeToString(recordData)

		// 上传语音并获取新的URL
		newURL, err := images.UploadBase64RecordToServer(base64Encoded)
		if err != nil {
			mylog.Printf("Error uploading base64 encoded image: %v", err)
			return &dto.MessageToCreate{
				Content: "错误: 上传语音失败",
				MsgID:   id,
				EventID: eventid,
				MsgSeq:  msgseq,
				MsgType: 0,
			}
		}

		// 发链接语音
		return &dto.RichMediaMessage{
			EventID:    id,
			FileType:   3,      // 3代表语音
			URL:        newURL, // 新语音链接
			Content:    "",     // 这个字段文档没有了
			SrvSendMsg: false,
		}
	} else if base64Image, ok := foundItems["base64_image"]; ok && len(base64Image) > 0 {
		// todo 适配base64图片
		//因为QQ群没有 form方式上传,所以在gensokyo内置了图床,需公网,或以lotus方式连接位于公网的gensokyo
		//要正确的开放对应的端口和设置正确的ip地址在config,这对于一般用户是有一些难度的
		// 解码base64图片数据
		fileImageData, err := base64.StdEncoding.DecodeString(base64Image[0])
		if err != nil {
			mylog.Printf("failed to decode base64 image: %v", err)
			return nil
		}

		// 首先压缩图片 默认不压缩
		compressedData, err := images.CompressSingleImage(fileImageData)
		if err != nil {
			mylog.Printf("Error compressing image: %v", err)
			return &dto.MessageToCreate{
				Content: "错误: 压缩图片失败",
				MsgID:   id,
				EventID: eventid,
				MsgSeq:  msgseq,
				MsgType: 0, // 默认文本类型
			}
		}

		base64Encoded := base64.StdEncoding.EncodeToString(compressedData)
		// 直接 base64 上传到 QQ CDN
		messageToCreate, err := images.CreateAndUploadMediaMessagePrivate(context.TODO(), base64Encoded, eventid, 1, false, "", userid, id, msgseq, apiv2)
		if err != nil {
			mylog.Printf("Error messageToCreate: %v", err)
			return &dto.MessageToCreate{
				Content: "错误: 上传图片失败",
				MsgID:   id,
				EventID: eventid,
				MsgSeq:  msgseq,
				MsgType: 0, // 默认文本类型
			}
		}
		return messageToCreate
	} else if mdContent, ok := foundItems["markdown"]; ok && len(mdContent) > 0 {
		// 解码base64 markdown数据
		mdData, err := base64.StdEncoding.DecodeString(mdContent[0])
		if err != nil {
			mylog.Printf("failed to decode base64 md: %v", err)
			return nil
		}
		markdown, keyboard, err := parseMDData(mdData)
		if err != nil {
			mylog.Printf("failed to parseMDData: %v", err)
			return nil
		}

		// 替换 keyboard 中 __USER_ID__ 占位符为实际用户 OpenID
		if keyboard != nil {
			userOpenID := idmap.ResolveOriginalID(userid)
			ResolvePlaceholderUserIDs(keyboard, userOpenID)
		}

		// 将 markdown 内容中的 CQ at 码转换为 QQ @ 语法
		if markdown != nil && markdown.Content != "" {
			markdown.Content = ResolveMarkdownAtMentions(markdown.Content)
			markdown.Content = ResolveMarkdownImages(markdown.Content, apiv2)
		}

		return &dto.MessageToCreate{
			Content:  "markdown",
			MsgID:    id,
			EventID:  eventid,
			MsgSeq:   msgseq,
			Markdown: markdown,
			Keyboard: keyboard,
			MsgType:  2,
		}
	} else if qqmusic, ok := foundItems["qqmusic"]; ok && len(qqmusic) > 0 {
		// 转换qq音乐id到一个md
		music_id := qqmusic[0]
		markdown, keyboard, err := parseQQMuiscMDData(music_id)
		if err != nil {
			mylog.Printf("failed to parseMDData: %v", err)
			return nil
		}
		if markdown != nil {
			return &dto.MessageToCreate{
				Content:  "markdown",
				MsgID:    id,
				EventID:  eventid,
				MsgSeq:   msgseq,
				Markdown: markdown,
				Keyboard: keyboard,
				MsgType:  2,
			}
		} else {
			return &dto.MessageToCreate{
				Content:  "markdown",
				MsgID:    id,
				EventID:  eventid,
				MsgSeq:   msgseq,
				Keyboard: keyboard,
				MsgType:  2,
			}
		}
	} else if videoURL, ok := foundItems["url_video"]; ok && len(videoURL) > 0 {
		newvideolink := "http://" + videoURL[0]
		// 发链接视频 http
		return &dto.RichMediaMessage{
			EventID:    id,
			FileType:   2,            // 2代表视频
			URL:        newvideolink, // 新图片链接
			Content:    "",           // 这个字段文档没有了
			SrvSendMsg: false,
		}
	} else if videoURLs, ok := foundItems["url_videos"]; ok && len(videoURLs) > 0 {
		newvideolink := "https://" + videoURLs[0]
		// 发链接视频 https
		return &dto.RichMediaMessage{
			EventID:    id,
			FileType:   2,            // 2代表视频
			URL:        newvideolink, // 新图片链接
			Content:    "",           // 这个字段文档没有了
			SrvSendMsg: false,
		}
	} else if base64Videos, ok := foundItems["base64_video"]; ok && len(base64Videos) > 0 {
		// base64 视频数据上传到 QQ CDN（FileType=2 视频）
		messageToCreate, err := images.CreateAndUploadMediaMessagePrivate(context.TODO(), base64Videos[0], eventid, 2, false, "", userid, id, msgseq, apiv2)
		if err != nil {
			mylog.Printf("Error uploading base64 video: %v", err)
			return &dto.MessageToCreate{
				Content: "错误: 上传视频失败",
				MsgID:   id,
				EventID: eventid,
				MsgSeq:  msgseq,
				MsgType: 0,
			}
		}
		return messageToCreate
	} else if localVideos, ok := foundItems["local_video"]; ok && len(localVideos) > 0 {
		// 从本地路径读取视频并 base64 上传到 QQ CDN
		safePath, err := safeLocalPath(localVideos[0], ".")
		if err != nil {
			mylog.Printf("安全校验失败，跳过本地视频: %v", err)
			return nil
		}
		videoData, err := os.ReadFile(safePath)
		if err != nil {
			mylog.Printf("Error reading the video from path %s: %v", localVideos[0], err)
			return &dto.MessageToCreate{
				Content: "错误: 视频文件不存在",
				MsgID:   id,
				EventID: eventid,
				MsgSeq:  msgseq,
				MsgType: 0,
			}
		}
		base64Encoded := base64.StdEncoding.EncodeToString(videoData)
		messageToCreate, err := images.CreateAndUploadMediaMessagePrivate(context.TODO(), base64Encoded, eventid, 2, false, "", userid, id, msgseq, apiv2)
		if err != nil {
			mylog.Printf("Error uploading local video: %v", err)
			return &dto.MessageToCreate{
				Content: "错误: 上传视频失败",
				MsgID:   id,
				EventID: eventid,
				MsgSeq:  msgseq,
				MsgType: 0,
			}
		}
		return messageToCreate
	} else if fileURLs, ok := foundItems["local_file"]; ok && len(fileURLs) > 0 {
		// 从本地路径读取文件
		// 安全校验：防止路径穿越
		safePath, err := safeLocalPath(fileURLs[0], ".")
		if err != nil {
			mylog.Printf("安全校验失败，跳过本地文件: %v", err)
			return nil
		}
		fileData, err := os.ReadFile(safePath)
		if err != nil {
			mylog.Printf("Error reading the file from path %s: %v", fileURLs[0], err)
			return &dto.MessageToCreate{
				Content: "错误: 文件不存在",
				MsgID:   id,
				EventID: eventid,
				MsgSeq:  msgseq,
				MsgType: 0,
			}
		}
		// 提取文件名，优先使用 file_name 参数，否则自动从路径提取
		fileName := ""
		if fns, ok := foundItems["file_name"]; ok && len(fns) > 0 && fns[0] != "" {
			fileName = fns[0]
		} else {
			fileName = filepath.Base(fileURLs[0])
		}
		// base64编码
		base64Encoded := base64.StdEncoding.EncodeToString(fileData)
		// 直接 base64 上传文件到 QQ CDN，文件名作为 content 传递
		messageToCreate, err := images.CreateAndUploadMediaMessagePrivate(context.TODO(), base64Encoded, eventid, 4, false, fileName, userid, id, msgseq, apiv2)
		if err != nil {
			mylog.Printf("Error uploading file: %v", err)
			return &dto.MessageToCreate{
				Content: "错误: 上传文件失败",
				MsgID:   id,
				EventID: eventid,
				MsgSeq:  msgseq,
				MsgType: 0,
			}
		}
		return messageToCreate
	} else if fileURLs, ok := foundItems["url_file"]; ok && len(fileURLs) > 0 {
		// 提取文件名，优先使用 file_name 参数，否则自动从 URL 提取
		fileName := ""
		if fns, ok := foundItems["file_name"]; ok && len(fns) > 0 && fns[0] != "" {
			fileName = fns[0]
		} else {
			fileName = filepath.Base(fileURLs[0])
			if fileName == "." || fileName == "/" {
				fileName = ""
			}
		}
		// 发链接文件 http
		return &dto.RichMediaMessage{
			EventID:    id,
			FileType:   4,           // 4代表文件
			URL:        "http://" + fileURLs[0],
			Content:    fileName,
			FileName:   fileName,
			SrvSendMsg: false,
		}
	} else if fileURLs, ok := foundItems["url_files"]; ok && len(fileURLs) > 0 {
		// 提取文件名，优先使用 file_name 参数，否则自动从 URL 提取
		fileName := ""
		if fns, ok := foundItems["file_name"]; ok && len(fns) > 0 && fns[0] != "" {
			fileName = fns[0]
		} else {
			fileName = filepath.Base(fileURLs[0])
			if fileName == "." || fileName == "/" {
				fileName = ""
			}
		}
		// 发链接文件 https
		return &dto.RichMediaMessage{
			EventID:    id,
			FileType:   4,           // 4代表文件
			URL:        "https://" + fileURLs[0],
			Content:    fileName,
			FileName:   fileName,
			SrvSendMsg: false,
		}
	} else if base64Files, ok := foundItems["base64_file"]; ok && len(base64Files) > 0 {
	   // base64文件数据上传到 QQ CDN
	   messageToCreate, err := images.CreateAndUploadMediaMessagePrivate(context.TODO(), base64Files[0], eventid, 4, false, "", userid, id, msgseq, apiv2)
	   if err != nil {
	    mylog.Printf("Error uploading base64 file: %v", err)
	    return &dto.MessageToCreate{
	     Content: "错误: 上传文件失败",
	     MsgID:   id,
	     EventID: eventid,
	     MsgSeq:  msgseq,
	     MsgType: 0,
	    }
	   }
	   return messageToCreate
	  } else if unknownImages, ok := foundItems["unknown_image"]; ok && len(unknownImages) > 0 {
	   // 无前缀图片，尝试作为 URL 处理
	   mylog.Printf("unknown_image 作为 URL 图片尝试发送: %s", unknownImages[0])
	   safeURL, blocked := normalizeAndCheckSSRF(unknownImages[0])
	   if blocked {
	    mylog.Printf("SSRF 阻止: unknown_image 目标地址不安全: %s", unknownImages[0])
	    return nil
	   }
	   return &dto.RichMediaMessage{
	    EventID:    id,
	    FileType:   1,
	    URL:        safeURL,
	    Content:    "",
	    SrvSendMsg: false,
	   }
	  } else if unknownRecords, ok := foundItems["unknown_record"]; ok && len(unknownRecords) > 0 {
	   // 无前缀语音，尝试作为 URL 处理
	   mylog.Printf("unknown_record 作为 URL 语音尝试发送: %s", unknownRecords[0])
	   safeURL, blocked := normalizeAndCheckSSRF(unknownRecords[0])
	   if blocked {
	    mylog.Printf("SSRF 阻止: unknown_record 目标地址不安全: %s", unknownRecords[0])
	    return nil
	   }
	   return &dto.RichMediaMessage{
	    EventID:    id,
	    FileType:   3,
	    URL:        safeURL,
	    Content:    "",
	    SrvSendMsg: false,
	   }
	  } else if unknownFiles, ok := foundItems["unknown_file"]; ok && len(unknownFiles) > 0 {
	   // 无前缀文件，尝试作为 URL 处理
	   fileName := ""
	   if fns, ok := foundItems["file_name"]; ok && len(fns) > 0 && fns[0] != "" {
	    fileName = fns[0]
	   }
	   mylog.Printf("unknown_file 作为 URL 文件尝试发送: %s", unknownFiles[0])
	   safeURL, blocked := normalizeAndCheckSSRF(unknownFiles[0])
	   if blocked {
	    mylog.Printf("SSRF 阻止: unknown_file 目标地址不安全: %s", unknownFiles[0])
	    return nil
	   }
	   return &dto.RichMediaMessage{
	    EventID:    id,
	    FileType:   4,
	    URL:        safeURL,
	    Content:    fileName,
	    FileName:   fileName,
	    SrvSendMsg: false,
	   }
	  } else {
	   // 返回文本信息
	   return &dto.MessageToCreate{
	    Content: messageText,
	    MsgID:   id,
	    EventID: eventid,
	    MsgSeq:  msgseq,
	    MsgType: 0, // 默认文本类型
	   }
	  }
	 }

	 // 通过user_id获取类型
func GetMessageTypeByUserid(appID string, userID interface{}) string {
	// 从appID和userID生成key
	var userIDStr string
	switch u := userID.(type) {
	case int:
		userIDStr = strconv.Itoa(u)
	case int64:
		userIDStr = strconv.FormatInt(u, 10)
	case float64:
		userIDStr = strconv.FormatFloat(u, 'f', 0, 64)
	case string:
		userIDStr = u
	default:
		// 可能需要处理其他类型或报错
		return ""
	}

	key := appID + "_" + userIDStr
	return echo.GetMsgTypeByKey(key)
}

// 通过user_id获取类型
func GetMessageTypeByUseridV2(userID interface{}) string {
	// 从appID和userID生成key
	var userIDStr string
	switch u := userID.(type) {
	case int:
		userIDStr = strconv.Itoa(u)
	case int64:
		userIDStr = strconv.FormatInt(u, 10)
	case float64:
		userIDStr = strconv.FormatFloat(u, 'f', 0, 64)
	case string:
		userIDStr = u
	default:
		// 可能需要处理其他类型或报错
		return ""
	}
	msgtype, _ := idmap.ReadConfigv2(userIDStr, "type")
	// if err != nil {
	// 	//mylog.Printf("GetMessageTypeByUseridV2失败:%v", err)
	// }
	return msgtype
}

// 通过group_id获取类型
func GetMessageTypeByGroupid(appID string, GroupID interface{}) string {
	// 从appID和userID生成key
	var GroupIDStr string
	switch u := GroupID.(type) {
	case int:
		GroupIDStr = strconv.Itoa(u)
	case int64:
		GroupIDStr = strconv.FormatInt(u, 10)
	case string:
		GroupIDStr = u
	default:
		// 可能需要处理其他类型或报错
		return ""
	}

	key := appID + "_" + GroupIDStr
	return echo.GetMsgTypeByKey(key)
}

// 通过group_id获取类型
func GetMessageTypeByGroupidV2(GroupID interface{}) string {
	// 从appID和userID生成key
	var GroupIDStr string
	switch u := GroupID.(type) {
	case int:
		GroupIDStr = strconv.Itoa(u)
	case int64:
		GroupIDStr = strconv.FormatInt(u, 10)
	case string:
		GroupIDStr = u
	default:
		// 可能需要处理其他类型或报错
		return ""
	}

	msgtype, _ := idmap.ReadConfigv2(GroupIDStr, "type")
	// if err != nil {
	// 	//mylog.Printf("GetMessageTypeByGroupidV2失败:%v", err)
	// }
	return msgtype
}

// uploadMedia 上传媒体并返回FileInfo
func uploadMedia(ctx context.Context, groupID string, richMediaMessage *dto.RichMediaMessage, apiv2 openapi.OpenAPI) (string, error) {
	// 调用API来上传媒体
	messageReturn, err := apiv2.PostGroupMessage(ctx, groupID, richMediaMessage)
	if err != nil {
		// 错误保存到本地
		if config.GetSaveError() {
			mylog.ErrLogToFile("type", "PostGroupMessage")
			mylog.ErrInterfaceToFile("request", richMediaMessage)
			mylog.ErrLogToFile("error", err.Error())
		}
		return "", err
	}
	// 返回上传后的FileInfo
	return messageReturn.MediaResponse.FileInfo, nil
}

// 发送栈中的消息
func SendStackMessages(apiv2 openapi.OpenAPI, messageid string, GroupID string) {
	count := config.GetAtoPCount()
	mylog.Printf("取出数量: %v", count)
	pairs := echo.PopGlobalStackMulti(count)
	for i, pair := range pairs {
		//mylog.Printf("发送栈中的消息匹配 %v: %v", pair.Group, GroupID)
		if pair.Group == GroupID {
			// 发送消息
			msgseq := echo.GetMappingSeq(messageid)
			echo.AddMappingSeq(messageid, msgseq+1)
			pair.GroupMessage.MsgSeq = msgseq + 1
			pair.GroupMessage.MsgID = messageid
			mylog.Printf("发送栈中的消息 使用MsgSeq[%v]使用MsgID[%v]", pair.GroupMessage.MsgSeq, pair.GroupMessage.MsgID)
			_, err := apiv2.PostGroupMessage(context.TODO(), pair.Group, pair.GroupMessage)
			if err != nil {
				mylog.Printf("发送组合消息失败: %v", err)
				// 错误保存到本地
				if config.GetSaveError() {
					mylog.ErrLogToFile("type", "PostGroupMessage")
					mylog.ErrInterfaceToFile("request", pair.GroupMessage)
					mylog.ErrLogToFile("error", err.Error())
				}
			} else {
				echo.RemoveFromGlobalStack(i)
			}
			// 检查错误码
			if err != nil && strings.Contains(err.Error(), `"code":22009`) {
				mylog.Printf("信息再次发送失败,加入到队列中,下次被动信息进行发送")
				echo.PushGlobalStack(pair)
			}
		}

	}
}

func auto_md(message callapi.ActionMessage, messageText string, richMediaMessage *dto.RichMediaMessage) (md *dto.Markdown, kb *keyboard.MessageKeyboard, transmd bool) {
	if echoStr, ok := message.Echo.(string); ok {
		// 当 message.Echo 是字符串类型时才执行此块
		msg_on_touch := echo.GetMsgIDv3(config.GetAppIDStr(), echoStr)
		mylog.Printf("msg_on_touch:%v", msg_on_touch)
		// 判断是否是 GetVisualkPrefixs 数组开头的文本
		visualkPrefixs := config.GetVisualkPrefixs()
		var matchedPrefix *structs.VisualPrefixConfig
		var isSpecialType bool // 用于标记是否为特殊类型
		// 去掉前缀开头的*
		// 处理特殊类型前缀
		specialPrefixes := make(map[int]string)
		for i, vp := range visualkPrefixs {
			if strings.HasPrefix(vp.Prefix, "*") {
				specialPrefixes[i] = vp.Prefix
				visualkPrefixs[i].Prefix = strings.TrimPrefix(vp.Prefix, "*")
			}
		}

		for i, vp := range visualkPrefixs {
			if strings.HasPrefix(msg_on_touch, vp.Prefix) {
				if _, ok := specialPrefixes[i]; ok {
					isSpecialType = true
				}
				if len(msg_on_touch) >= len(vp.Prefix) {
					if msg_on_touch != "" {
						transmd = true
						matchedPrefix = &vp
						break // 匹配到了
					}
				}
			}
		}
		if transmd {
			//将messageText和groupReply组合成一个md
			// 处理 Markdown
			CustomTemplateID := config.GetCustomTemplateID()
			imgURL := richMediaMessage.URL
			// 处理来自nt图床的链接
			imgURL = processImgUrl(imgURL)
			height, width, err := images.GetImageDimensions(imgURL)
			if err != nil {
				height = 480
				width = 480
				mylog.Printf("获取图片宽高出错,默认宽高480x480")
			}
			imgDesc := fmt.Sprintf("图片 #%dpx #%dpx", width, height)

			// 将所有的\r\n替换为\r
			messageText = strings.ReplaceAll(messageText, "\r\n", "\r")
			// 将所有的\n替换为\r
			messageText = strings.ReplaceAll(messageText, "\n", "\r")

			// // 检查messageText是否以\r开头
			// if !strings.HasPrefix(messageText, "\r") {
			// 	messageText = "\r" + messageText
			// }

			if config.GetEntersAsBlock() {
				messageText = strings.ReplaceAll(messageText, "\r", " ")
			}

			// 根据配置决定如何生成Markdown内容
			if !config.GetNativeMD() {
				// 创建 MarkdownParams 的实例
				mdParams := []*dto.MarkdownParams{
					{Key: "text_start", Values: []string{" "}}, //空着
					{Key: "img_dec", Values: []string{imgDesc}},
					{Key: "img_url", Values: []string{imgURL}},
					{Key: "text_end", Values: []string{messageText}},
				}
				// 组合模板 Markdown
				md = &dto.Markdown{
					CustomTemplateID: CustomTemplateID,
					Params:           mdParams,
				}
			} else {
				// 使用原生Markdown格式
				content := fmt.Sprintf(" %s![%s](%s)%s", " ", imgDesc, imgURL, messageText)
				// 原生 Markdown
				md = &dto.Markdown{
					Content: content,
				}
			}

			whiteList := matchedPrefix.WhiteList
			// 创建 CustomKeyboard
			customKeyboard := &keyboard.CustomKeyboard{
				Rows: []*keyboard.Row{},
			}

			var currentRow *keyboard.Row
			buttonCount := 0 // 当前行的按钮计数

			for _, whiteLabel := range whiteList {
				// 如果whiteList的成员数大于或等于15，才检查whiteLabel是否为纯数字
				if len(whiteList) >= 15 {
					if _, err := strconv.Atoi(whiteLabel); err == nil {
						// 如果没有错误，表示 whiteLabel 是一个数字，因此忽略这个元素并继续下一个迭代
						// 避免 因为纯数字按钮太多导致混乱,但是少量的纯数字按钮可以允许
						continue
					}
				}
				// 检查msg_on_touch是否已经以whiteLabel结尾
				//场景 按钮递归时 比如 随机meme 再来一次,同时随机meme是*类型一级指令
				var dataLabel string
				if !strings.HasSuffix(msg_on_touch, whiteLabel) {
					dataLabel = msg_on_touch + whiteLabel
				} else {
					dataLabel = msg_on_touch
				}
				//当虚拟二级指令是*开头,真实的二级指令
				//以xxxx指令为例 第一次 xxxx 然后 xx攻略 然后 xxxx攻略商店(会失效)
				//作用 第一次 xxxx 第二次 xxxx攻略 第三次 xxxx商店
				if isSpecialType && len(msg_on_touch) > len(matchedPrefix.Prefix) {
					dataLabel = matchedPrefix.Prefix + whiteLabel
				}

				//在虚拟二级指令白名单,设置*前缀,代表不真实添加,仅再来一次
				if strings.HasPrefix(whiteLabel, "*") {
					//移除whiteLabel前端的*,*仅作为判断,不作为显示
					whiteLabel = strings.TrimPrefix(whiteLabel, "*")
					dataLabel = matchedPrefix.Prefix
				}

				//在虚拟二级指令白名单,设置&前缀,代表仅触发其本身
				//如果&前缀指令包含了空格 则只显示空格右侧的文本
				if strings.HasPrefix(whiteLabel, "&") {
					//移除whiteLabel前端的*,*仅作为判断,不作为显示
					whiteLabel = strings.TrimPrefix(whiteLabel, "&")
					//这里是实际填充到data的
					dataLabel = whiteLabel
					// 找到最后一个空格的位置 显示空格右边的文本 没有找到空格则不变
					lastSpaceIndex := strings.LastIndex(whiteLabel, " ")
					if lastSpaceIndex != -1 && lastSpaceIndex < len(whiteLabel)-1 {
						// 获取空格右侧的子字符串
						whiteLabel = whiteLabel[lastSpaceIndex+1:]
					}
				}

				//在虚拟二级指令白名单,设置@前缀,代表仅触发其本身
				//如果@前缀指令包含了空格 则只显示和应用空格右侧的文本
				if strings.HasPrefix(whiteLabel, "@") {
					// 移除whiteLabel前端的"@"
					whiteLabel = strings.TrimPrefix(whiteLabel, "@")
					// 找到最后一个空格的位置
					lastSpaceIndex := strings.LastIndex(whiteLabel, " ")
					if lastSpaceIndex != -1 {
						// 先存储空格左侧的字符串到dataLabel
						dataLabel = whiteLabel[:lastSpaceIndex]
						// 然后更新whiteLabel为空格右侧的子字符串
						whiteLabel = whiteLabel[lastSpaceIndex+1:]
					} else {
						// 如果没有找到空格，将整个字符串赋给dataLabel
						dataLabel = whiteLabel
					}
				}

				var actiontype keyboard.ActionType
				var permission *keyboard.Permission
				var actiondata string
				var skip bool
				//检查是否设置了enter数组
				enter := checkDataLabelPrefix(dataLabel)
				//例外规则
				if checkDataLabelPrefixExcept(whiteLabel) {
					enter = false
				}

				switch {
				case strings.HasPrefix(whiteLabel, "邀请机器人"): //默认是群
					botuin := config.GetUinStr()
					botappid := config.GetAppIDStr()
					boturl := BuildQQBotShareLink(botuin, botappid)
					actiontype = 0
					actiondata = boturl
					permission = &keyboard.Permission{
						Type: 2, // 所有人可操作
					}
				case strings.HasPrefix(whiteLabel, "添加到群聊"):
					botuin := config.GetUinStr()
					botappid := config.GetAppIDStr()
					boturl := BuildQQBotShareLink(botuin, botappid)
					actiontype = 0
					actiondata = boturl
					permission = &keyboard.Permission{
						Type: 2, // 所有人可操作
					}
				case strings.HasPrefix(whiteLabel, "添加到频道"):
					botuin := config.GetUinStr()
					botappid := config.GetAppIDStr()
					boturl := BuildQQBotShareLinkGuild(botuin, botappid)
					actiontype = 0
					actiondata = boturl
					permission = &keyboard.Permission{
						Type: 2, // 所有人可操作
					}
				case strings.HasPrefix(whiteLabel, "权限判断"):
					actiontype = 1
					actiondata = "收到就代表是管理员哦"
					permission = &keyboard.Permission{
						Type: 1, // 仅管理可操作
					}
				case strings.HasPrefix(whiteLabel, "%"):
					// 分割whiteLabel来获取显示内容和URL
					parts := strings.SplitN(whiteLabel[1:], " ", 2) // [1:] 用于去除白名单标签开头的'%'
					if len(parts) == 2 {
						whiteLabel = parts[0] // 显示内容
						actiondata = parts[1] // URL
						actiontype = 0        // 链接类型
						permission = &keyboard.Permission{
							Type: 2, // 所有人可操作
						}
					}
				case strings.HasPrefix(whiteLabel, "$"):
					// 只有5%的概率执行以下代码
					if rand.Float64() < 0.05 {
						// 分割whiteLabel来获取显示内容和URL
						parts := strings.SplitN(whiteLabel[1:], " ", 2) // [1:] 用于去除白名单标签开头的'%'
						if len(parts) == 2 {
							whiteLabel = parts[0] // 显示内容
							actiondata = parts[1] // URL
							actiontype = 0        // 链接类型
							permission = &keyboard.Permission{
								Type: 2, // 所有人可操作
							}
						}
					} else {
						skip = true
					}
				case strings.HasPrefix(whiteLabel, "^"):
					// 分割whiteLabel来获取显示内容和URL
					parts := strings.SplitN(whiteLabel[1:], " ", 2) // [1:] 用于去除白名单标签开头的'^'
					if len(parts) == 2 {
						whiteLabel = parts[0] // 显示内容
						actiondata = parts[1] // 发送给服务端内容
						actiontype = 1        // 回调类型
						permission = &keyboard.Permission{
							Type: 2, // 所有人可操作
						}
					}
				default:
					actiontype = 2         //帮用户输入指令 用户自己回车发送
					actiondata = dataLabel //从虚拟前缀的二级指令组合md按钮
					permission = &keyboard.Permission{
						Type: 2, // 所有人可操作
					}
				}

				if !skip {
					// 创建按钮
					button := &keyboard.Button{
						RenderData: &keyboard.RenderData{
							Label:        whiteLabel,
							VisitedLabel: whiteLabel,
							Style:        1, //蓝色边缘
						},
						Action: &keyboard.Action{
							Type:          actiontype,
							Permission:    permission,
							Data:          actiondata,
							UnsupportTips: "请升级新版手机QQ",
							Enter:         enter,
						},
					}

					// 如果当前行为空或已满（4个按钮），则创建一个新行
					if currentRow == nil || buttonCount == 4 {
						currentRow = &keyboard.Row{}
						customKeyboard.Rows = append(customKeyboard.Rows, currentRow)
						buttonCount = 0 // 重置按钮计数
					}

					// 将按钮添加到当前行
					currentRow.Buttons = append(currentRow.Buttons, button)
					buttonCount++
				}
			}
			// 在循环结束后，最后一行可能不满4个按钮，但已经被正确处理

			// 在添加完所有按钮后，进行一次清理操作，如果按钮RenderData是空，不显示
			for rowIndex := 0; rowIndex < len(customKeyboard.Rows); rowIndex++ {
				row := customKeyboard.Rows[rowIndex]
				// 临时存储有效按钮的切片
				validButtons := []*keyboard.Button{}

				// 遍历行中的所有按钮
				for _, button := range row.Buttons {
					// 检查按钮的RenderData是否为单个空格，这里可以根据需求调整条件
					if button.RenderData.Label != " " {
						validButtons = append(validButtons, button)
					}
				}

				// 更新当前行的按钮为仅包含有效按钮的切片
				customKeyboard.Rows[rowIndex].Buttons = validButtons
			}

			// 创建 MessageKeyboard 并设置其 Content
			kb = &keyboard.MessageKeyboard{
				Content: customKeyboard,
			}
		}
	}
	return md, kb, transmd
}

// 构建QQ群机器人分享链接的函数
func BuildQQBotShareLink(uin string, appid string) string {
	return fmt.Sprintf("https://qun.qq.com/qunpro/robot/qunshare?robot_uin=%s&robot_appid=%s", uin, appid)
}

// 构建QQ群机器人分享链接的函数
func BuildQQBotShareLinkGuild(uin string, appid string) string {
	return fmt.Sprintf("https://qun.qq.com/qunpro/robot/share?robot_appid=%s", appid)
}

// 检查dataLabel是否以config中getenters返回的任一字符串开头
func checkDataLabelPrefix(dataLabel string) bool {
	enters := config.GetEnters()
	for _, enter := range enters {
		if enter != "" && strings.HasPrefix(dataLabel, enter) {
			return true
		}
	}
	return false
}

// 检查whiteLabel是否以config中getentersexcept返回的任一字符串开头
func checkDataLabelPrefixExcept(whiteLabel string) bool {
	enters := config.GetEntersExcept()
	for _, enter := range enters {
		if enter != "" && strings.HasPrefix(whiteLabel, enter) {
			return true
		}
	}
	return false
}

func processImgUrl(input string) string {
	// 将指定的URL前缀替换
	processed := strings.ReplaceAll(input, "https://multimedia.nt.qq.com.cn", "http://multimedia.nt.qq.com")

	// 仅当输入字符串包含 "multimedia.nt.qq" 时，才替换下划线为 "%5f"
	if strings.Contains(processed, "multimedia.nt.qq") {
		processed = strings.ReplaceAll(processed, "_", "%5f")
	}

	return processed
}

func postGroupMessageWithRetry(apiv2 openapi.OpenAPI, groupID string, groupMessage *dto.MessageToCreate) (resp *dto.GroupMessageResponse, err error) {
	retryCount := 3
	for i := 0; i < retryCount; i++ {
		msgseq := echo.GetMappingSeq(groupMessage.MsgID)
		echo.AddMappingSeq(groupMessage.MsgID, msgseq+1)
		groupMessage.MsgSeq = msgseq + 1

		if !messagequeue.GetRateLimiter().WaitWithTimeout(5 * time.Second) {
			mylog.Printf("[限流] 群消息发送等待超时，重试 %d/%d", i+1, retryCount)
			continue
		}

		resp, err = apiv2.PostGroupMessage(context.TODO(), groupID, groupMessage)
		if err != nil && strings.Contains(err.Error(), "context deadline exceeded") {
			mylog.Printf("超时重试第 %d 次: %v", i+1, err)
			if config.GetSaveError() {
				mylog.ErrLogToFile("type", "PostGroupMessage-context-deadline-exceeded-retry-"+strconv.Itoa(i+1))
				mylog.ErrInterfaceToFile("request", groupMessage)
				mylog.ErrLogToFile("error", err.Error())
			}
			time.Sleep(1 * time.Second) // 重试间隔1秒
			continue
		} else {
			mylog.Printf("超时重试第 %d 次成功: %v", i+1, err)
			if config.GetSaveError() {
				mylog.ErrLogToFile("type", "PostGroupMessage-context-deadline-exceeded-retry-"+strconv.Itoa(i+1)+"-successed")
				mylog.ErrInterfaceToFile("request", groupMessage)
				if resp != nil {
					mylog.ErrLogToFile("msgid", resp.Message.ID)
				}
			}
		}
		break
	}
	return resp, err
}

// 富媒体专用重试：针对 context deadline exceeded
func postGroupRichMediaMessageWithRetry(apiv2 openapi.OpenAPI, groupID string, richMediaMessage *dto.RichMediaMessage) (resp *dto.GroupMessageResponse, err error) {
	richMediaMessage.EventID = ""
	retryCount := 3 // 设置最大重试次数为 3
	for i := 0; i < retryCount; i++ {
		resp, err = apiv2.PostGroupMessage(context.TODO(), groupID, richMediaMessage)
		if err != nil && (strings.Contains(err.Error(), "context deadline exceeded") || strings.Contains(err.Error(), "富媒体文件上传超时")) {
			// 仅对超时做重试
			mylog.Printf("富媒体超时重试第 %d 次: %v", i+1, err)
			if config.GetSaveError() {
				mylog.ErrLogToFile("type", "PostGroupRichMediaMessage-context-deadline-exceeded-retry-"+strconv.Itoa(i+1))
				mylog.ErrInterfaceToFile("request", richMediaMessage)
				mylog.ErrLogToFile("error", err.Error())
			}
			time.Sleep(3 * time.Second) // 重试间隔 3 秒
			continue
		}

		// 成功 或 非超时错误，都直接退出循环
		if config.GetSaveError() {
			logType := "PostGroupRichMediaMessage-final"
			if err == nil {
				logType = "PostGroupRichMediaMessage-retry-success-" + strconv.Itoa(i+1)
			}
			mylog.ErrLogToFile("type", logType)
			mylog.ErrInterfaceToFile("request", richMediaMessage)
			if err != nil {
				mylog.ErrLogToFile("error", err.Error())
			} else if resp != nil {
				mylog.ErrLogToFile("Ret", fmt.Sprintf("%d", resp.MediaResponse.Ret))
			}
		}
		break
	}
	return resp, err
}

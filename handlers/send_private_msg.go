package handlers

import (
  "context"
  "encoding/base64"
  "encoding/json"
  "fmt"
  "strconv"
  "strings"
  "sync"
  "time"

  "github.com/hoshinonyaruko/gensokyo/callapi"
  "github.com/hoshinonyaruko/gensokyo/config"
  "github.com/hoshinonyaruko/gensokyo/echo"
  "github.com/hoshinonyaruko/gensokyo/idmap"
  "github.com/hoshinonyaruko/gensokyo/internal/domain/identity"
  "github.com/hoshinonyaruko/gensokyo/mylog"
  "github.com/tencent-connect/botgo/dto"
  "github.com/tencent-connect/botgo/dto/keyboard"
  "github.com/tencent-connect/botgo/openapi"
 )

func init() {
	callapi.RegisterHandler("send_private_msg", HandleSendPrivateMsg)
}

// sendPrivateMsgKeyMap 定义 foundItems 中需要按 MessageToCreate 路径发送的 key 集合
var sendPrivateMsgKeyMap = map[string]bool{
	"markdown":      true,
	"qqmusic":       true,
	"local_image":   true,
	"local_record":  true,
	"url_image":     true,
	"url_images":    true,
	"url_record":    true,
	"base64_record": true,
	"base64_image":  true,
	"local_video":   true,
	"url_video":     true,
	"base64_video":  true,
	"local_file":    true,
	"url_file":      true,
	"url_files":     true,
	"base64_file":   true,
}

// streamCache 存储流式消息的 stream_msg_id 和 index，key = qq
var streamCache sync.Map

func HandleSendPrivateMsg(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	// 使用 message.Echo 作为key来获取消息类型
	var msgType string
	var retmsg string
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

	if message.Params.UserID != nil && !identity.IsOpenID(message.Params.UserID.(string)) {
		if msgType == "" && message.Params.UserID != nil && checkZeroUserID(message.Params.UserID) {
			msgType = GetMessageTypeByUserid(config.GetAppIDStr(), message.Params.UserID)
		}
		if msgType == "" && message.Params.GroupID != nil && checkZeroGroupID(message.Params.GroupID) {
			msgType = GetMessageTypeByGroupid(config.GetAppIDStr(), message.Params.GroupID)
		}
		if msgType == "" && message.Params.UserID != nil && checkZeroUserID(message.Params.UserID) {
			msgType = GetMessageTypeByUseridV2(message.Params.UserID)
		}
		if msgType == "" && message.Params.GroupID != nil && checkZeroGroupID(message.Params.GroupID) {
			msgType = GetMessageTypeByGroupidV2(message.Params.GroupID)
		}
	}

	// New checks for UserID and GroupID being nil or 0
	if (message.Params.UserID == nil || !checkZeroUserID(message.Params.UserID)) &&
		(message.Params.GroupID == nil || !checkZeroGroupID(message.Params.GroupID)) {
		mylog.Printf("send_group_msgs接收到错误action: %v", message)
		return "", nil
	}

	var idInt64 int64
	var err error

	if message.Params.UserID != nil && identity.IsOpenID(message.Params.UserID.(string)) {
		idInt64, err = idmap.GenerateRowID(message.Params.UserID.(string), 9)
		// 临时的
		msgType = "group_private"
	} else {
		if message.Params.GroupID != "" {
			idInt64, err = ConvertToInt64(message.Params.GroupID)
		} else if message.Params.UserID != "" {
			idInt64, err = ConvertToInt64(message.Params.UserID)
		}
	}

	//设置递归 对直接向gsk发送action时有效果
	if msgType == "" {
		messageCopy := message
		if err != nil {
			mylog.Printf("错误：无法转换 ID %v\n", err)
		} else {
			// 递归1次（枚举剩余消息类型，当前仅 group）
			echo.AddMapping(idInt64, 2)
			// 递归调用handleSendPrivateMsg，使用设置的消息类型
			echo.AddMsgType(config.GetAppIDStr(), idInt64, "group_private")
			HandleSendPrivateMsg(client, api, apiv2, messageCopy)
		}
	} else if echo.GetMapping(idInt64) <= 0 {
		// 特殊值代表不递归
		echo.AddMapping(idInt64, 10)
	}

	var resp *dto.C2CMessageResponse
	switch msgType {
	//这里是pr上来的,我也不明白为什么私聊会出现group类型 猜测是为了匹配包含了groupid的私聊?
	case "group_private", "group":
		//私聊信息
		var UserID string
		if !identity.IsOpenID(message.Params.UserID.(string)) {
			if config.GetIdmapPro() {
				//还原真实的userid
				//mylog.Printf("group_private:%v", message.Params.UserID.(string))
				_, UserID, err = idmap.RetrieveRowByIDv2Pro("690426430", message.Params.UserID.(string))
				if err != nil {
					mylog.Printf("Error reading config: %v", err)
					return "", nil
				}
				mylog.Printf("测试,通过Proid获取的UserID:%v", UserID)
			} else {
				//还原真实的userid
				UserID, err = idmap.RetrieveRowByIDv2(message.Params.UserID.(string))
				if err != nil {
					mylog.Printf("Error reading config: %v", err)
					return "", nil
				}
			}
		} else {
			UserID = message.Params.UserID.(string)
		}

		// 解析消息内容
		messageText, foundItems := parseMessageContent(message.Params, message, client, api, apiv2)

		// [CQ:wakeup,userid=xxx] 标记：改为向指定用户发送 C2C 召回消息
		if wakeupIDs, ok := foundItems["wakeup"]; ok && len(wakeupIDs) > 0 {
			targetUserID := wakeupIDs[0]
			mylog.Printf("[CQ:wakeup] 目标用户: %s，转为召回消息发送", targetUserID)
			// 覆盖 user_id 为目标用户，交由召回 handler 统一处理（含虚拟ID→OpenID转换）
			message.Params.UserID = targetUserID
			return HandleSendPrivateMsgWakeup(client, api, apiv2, message)
		}

		// 使用 echo 获取消息ID
		var messageID string
		// EventID
		var eventID string
		if config.GetLazyMessageId() {
			//由于实现了Params的自定义unmarshell 所以可以类型安全的断言为string
			messageID = echo.GetLazyMessagesId(UserID)
			mylog.Printf("GetLazyMessagesId: %v", messageID)
		}
		if messageID == "" {
			if echoStr, ok := message.Echo.(string); ok {
				messageID = echo.GetMsgIDByKey(echoStr)
				mylog.Println("echo取私聊发信息对应的message_id:", messageID)
			}
		}
		// 如果messageID仍然为空，尝试使用config.GetAppID和UserID的组合来获取messageID
		// 如果messageID为空，通过函数获取
		if messageID == "" {
			messageID = GetMessageIDByUseridOrGroupid(config.GetAppIDStr(), UserID)
			mylog.Println("通过GetMessageIDByUserid函数获取的message_id:", messageID)
		}
		if messageID == "2000" {
			messageID = ""
			mylog.Println("通过lazymsgid发送群私聊主动信息,每月可发送1次")
			if !identity.IsOpenID(message.Params.UserID.(string)) {
				eventID = GetEventIDByUseridOrGroupid(config.GetAppIDStr(), message.Params.UserID)
			} else {
				eventID = GetEventIDByUseridOrGroupidv2(config.GetAppIDStr(), message.Params.UserID)
			}
			mylog.Printf("尝试获取当前是否有eventID可用,如果有则不消耗主动次数:%v", eventID)
		}
		// [CQ:active] 标记：强制走主动推送，即使有 msg_id
		if _, ok := foundItems["active"]; ok {
			messageID = ""
			eventID = ""
			mylog.Println("[CQ:active] 标记，强制主动推送")
		}
		//开发环境用 私聊不可用1000
		// if config.GetDevMsgID() {
		// 	messageID = "1000"
		// }
		mylog.Println("私聊发信息messageText:", messageText)
		//mylog.Println("foundItems:", foundItems)

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
			mylog.Printf("发私聊图文混合信息")
			// 创建包含单个图片的 singleItem
			singleItem[imageType] = []string{imageUrl}
			msgseq := echo.IncrementMappingSeq(messageID)
			groupReply := generatePrivateMessage(messageID, eventID, singleItem, "", msgseq, apiv2, UserID)
			// 进行类型断言
			richMediaMessage, ok := groupReply.(*dto.RichMediaMessage)
			// 如果断言为RichMediaMessage失败
			var groupMessage *dto.MessageToCreate
			if !ok {
				// 尝试断言为MessageToCreate（local_image/base64_image 等已在生成时完成上传）
				groupMessage, ok = groupReply.(*dto.MessageToCreate)
				if !ok {
					mylog.Printf("Error: Expected RichMediaMessage type for key,value:%v", groupReply)
					return "", nil // 或其他错误处理
				}
			}
			// 如果groupMessage是nil 说明groupReply是richMediaMessage类型 如果groupMessage不是nil 说明groupReply是MessageToCreate
			if groupMessage == nil {
				// 上传图片并获取FileInfo
				fileInfo, err := uploadMediaPrivate(context.TODO(), UserID, richMediaMessage, apiv2)
				if err != nil {
					mylog.Printf("上传图片失败: %v", err)
					return "", nil // 或其他错误处理
				}
				// 图文混合消息同样需要转换 [CQ:at] 为 @用户名，与纯文本路径对齐
				// 否则 QQ 官方 API 不识别 CQ 码，会原文显示 [CQ:at,qq=数字]
				messageText = resolvePlainTextAtMentions(messageText)
				// 创建包含文本和图像信息的消息
				msgseq = echo.IncrementMappingSeq(messageID)
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
				// 已上传完成的富媒体（local_image/base64_image 等），补充文本后成为图文混合消息
				groupMessage.Content = resolvePlainTextAtMentions(messageText)
				groupMessage.MsgID = messageID
				groupMessage.EventID = eventID
				groupMessage.Timestamp = time.Now().Unix() // 设置时间戳
			}

			// 处理 [CQ:reply,id=数字] → message_reference + msg_id（私聊场景校验避免越权）
			if replyIDs, ok := foundItems["reply_msg_id"]; ok && len(replyIDs) > 0 && messageText != "" {
				applyPrivateReply(groupMessage, replyIDs, UserID)
			}

			// 发送组合消息
			resp, err = apiv2.PostC2CMessage(context.TODO(), UserID, groupMessage)
			if err != nil {
				mylog.Printf("发送组合消息失败: %v", err)
				mylog.Printf("%s", FormatQQError(err))
				// 错误保存到本地
				if config.GetSaveError() {
					mylog.ErrLogToFile("type", "PostC2CMessage")
					mylog.ErrInterfaceToFile("request", groupMessage)
					mylog.ErrLogToFile("error", err.Error())
				}
				// 22009: 主动消息超过频控限制，记录日志 (被动回复场景无需补偿)
				if IsQQError(err, 22009) {
					mylog.Printf("私聊主动消息受限(code:22009)，消息被丢弃")
					retmsg, _ = SendC2CResponse(client, err, &message, resp)
					return "", nil
				}
				// 请求参数 event_id 无效，清空后重试一次
				if IsQQError(err, 40034025) {
					groupMessage.EventID = ""
					resp, err = apiv2.PostC2CMessage(context.TODO(), UserID, groupMessage)
					if err != nil {
						mylog.Printf("发送组合消息失败 on code 40034025: %v", err)
					}
				}
				// 超时重试
				if IsDeliveryTimeout(err) {
					resp, err = postC2CMessageWithRetry(apiv2, UserID, groupMessage)
				}
			}
			// 发送成功/最终失败回执（err 体现结果，避免客户端超时）
			retmsg, _ = SendC2CResponse(client, err, &message, resp)

			delete(foundItems, imageType) // 从foundItems中删除已处理的图片项
			messageText = ""
		}

		// 优先发送文本信息
		// 如果存在 [CQ:input_notify]，先发送输入状态通知
		if notifyItems, ok := foundItems["input_notify"]; ok && len(notifyItems) > 0 {
		 var notifyData map[string]string
		 if err := json.Unmarshal([]byte(notifyItems[0]), &notifyData); err == nil {
		  inputType := 1
		  inputSecond := 60
		  if t, err := strconv.Atoi(notifyData["type"]); err == nil {
		   inputType = t
		  }
		  if s, err := strconv.Atoi(notifyData["second"]); err == nil {
		   inputSecond = s
		  }
		  notifyMsg := &dto.MessageToCreate{
		   MsgType: 6,
		   InputNotify: &dto.InputNotify{
		    InputType:   inputType,
		    InputSecond: inputSecond,
		   },
		   MsgID:  messageID,
		   MsgSeq: echo.GetMappingSeq(messageID),
		  }
		  resp, err := apiv2.PostC2CMessage(context.TODO(), UserID, notifyMsg)
		  if err != nil {
		   mylog.Printf("发送输入状态通知失败: %v", err)
		   mylog.Printf("%s", FormatQQError(err))
		  } else {
		   mylog.Printf("[CQ:input_notify] 已发送输入状态通知")
		  }
		  retmsg, _ = SendC2CResponse(client, err, &message, resp)
		  delete(foundItems, "input_notify")
		 }
		}

		// 流式消息处理 [CQ:stream]
		if streamItems, ok := foundItems["stream"]; ok && len(streamItems) > 0 {
			var streamData map[string]string
			if err := json.Unmarshal([]byte(streamItems[0]), &streamData); err == nil {
				streamType := streamData["type"]
				qq := streamData["qq"]
				delete(foundItems, "stream")

				// 从缓存读取 stream_msg_id 和 index
				type streamInfo struct {
					StreamMsgID string
					Index       int
				}
				info := &streamInfo{}
				if cached, ok := streamCache.Load(qq); ok {
					info = cached.(*streamInfo)
				}

				chunk := &dto.StreamChunk{
					ContentType: "text",
					ContentRaw:  messageText,
					MsgID:       messageID,
					MsgSeq:      echo.GetMappingSeq(messageID),
				}

				switch streamType {
				case "start":
					chunk.InputMode = "replace"
					chunk.InputState = 1
					chunk.Index = 0
					resp, err := apiv2.PostC2CStreamMessage(context.TODO(), UserID, chunk)
					if err != nil {
						mylog.Printf("流式消息首片发送失败: %v", err)
					} else if resp != nil && resp.Message != nil {
						info.StreamMsgID = resp.Message.ID
						info.Index = 0
						streamCache.Store(qq, info)
						mylog.Printf("[CQ:stream] 首片发送成功, stream_msg_id=%s", info.StreamMsgID)
					}
					retmsg, _ = SendC2CResponse(client, err, &message, resp)

				case "mid":
					if info.StreamMsgID == "" {
						mylog.Printf("[CQ:stream] 续片缺少 stream_msg_id，跳过")
					} else {
						info.Index++
						chunk.StreamMsgID = info.StreamMsgID
						chunk.InputState = 1
						chunk.Index = info.Index
						streamCache.Store(qq, info)
						resp, err := apiv2.PostC2CStreamMessage(context.TODO(), UserID, chunk)
						if err != nil {
							mylog.Printf("流式消息续片发送失败: %v", err)
						} else {
							mylog.Printf("[CQ:stream] 续片发送成功, index=%d", info.Index)
						}
						retmsg, _ = SendC2CResponse(client, err, &message, resp)
					}

				case "finish":
					if info.StreamMsgID == "" {
						mylog.Printf("[CQ:stream] 终片缺少 stream_msg_id，跳过")
					} else {
						info.Index++
						chunk.StreamMsgID = info.StreamMsgID
						chunk.InputState = 10
						chunk.Index = info.Index
						resp, err := apiv2.PostC2CStreamMessage(context.TODO(), UserID, chunk)
						if err != nil {
							mylog.Printf("流式消息终片发送失败: %v", err)
						} else {
							mylog.Printf("[CQ:stream] 终片发送成功")
						}
						retmsg, _ = SendC2CResponse(client, err, &message, resp)
					}
					streamCache.Delete(qq)
				}
			}
			// 流式消息处理完毕后返回，不再走普通文本发送路径
			return retmsg, nil
		}

		if strings.TrimSpace(messageText) != "" {
			msgseq := echo.IncrementMappingSeq(messageID)
			groupReply := generatePrivateMessage(messageID, eventID, nil, messageText, msgseq, apiv2, UserID)

			// 进行类型断言
			groupMessage, ok := groupReply.(*dto.MessageToCreate)
			if !ok {
				mylog.Println("Error: Expected MessageToCreate type.")
				return "", nil
			}

			groupMessage.Timestamp = time.Now().Unix() // 设置时间戳

			// 处理 [CQ:markdown] → 将消息类型切换为 markdown
			        var md *dto.Markdown
			        var kb *keyboard.MessageKeyboard
			        if mdItems, ok := foundItems["markdown"]; ok && len(mdItems) > 0 {
			         md, kb = parseMarkdownFromMessage(mdItems[0])
			         if md != nil && md.Content != "" {
			          md.Content = ResolveMarkdownAtMentions(md.Content)
			          md.Content = ResolveMarkdownImages(md.Content, apiv2)
			                     }
			                     if kb != nil {
			                      ResolveKeyboardImages(kb, apiv2)
			                     }
			         if md != nil {
			          groupMessage.Markdown = md
			          groupMessage.Keyboard = kb
			          groupMessage.MsgType = 2
			          groupMessage.Content = ""
			          delete(foundItems, "markdown")
			          mylog.Printf("[CQ:markdown] 将私聊消息类型切换为 markdown")
			         }
			        }

			        // 没有 markdown 时，纯文本消息转换 [CQ:at] 为 @用户名
			        if md == nil {
			         groupMessage.Content = resolvePlainTextAtMentions(groupMessage.Content)
			        }

			        // 没有内嵌 keyboard 时，处理独立 [CQ:keyboard] → 附加内嵌键盘（可与 markdown 共存）
			        if groupMessage.Keyboard == nil {
			         if kbItems, ok := foundItems["keyboard"]; ok && len(kbItems) > 0 {
			          kb, err := parseKeyboardData([]byte(kbItems[0]))
			          if err != nil || kb == nil {
			           mylog.Printf("[CQ:keyboard] 解析键盘数据失败: %v", err)
			          } else {
			           // 替换 keyboard 中 __USER_ID__ 占位符为实际用户 OpenID
			           userOpenID := idmap.ResolveOriginalID(UserID)
			           ResolvePlaceholderUserIDs(kb, userOpenID)
			           // 处理 keyboard 按钮中的本地图片路径
			           ResolveKeyboardImages(kb, apiv2)
			           groupMessage.Keyboard = kb
			           // 从 foundItems 中移除 keyboard，避免下方循环重复发送
			           delete(foundItems, "keyboard")
			           mylog.Printf("[CQ:keyboard] 私聊消息附加内嵌键盘")
			          }
			         }
			        }

			        // 处理 [CQ:reply,id=数字] → message_reference + msg_id（私聊场景校验避免越权）
			    if replyIDs, ok := foundItems["reply_msg_id"]; ok && len(replyIDs) > 0 && messageText != "" {
			        applyPrivateReply(groupMessage, replyIDs, UserID)
			    }

			    resp, err := apiv2.PostC2CMessage(context.TODO(), UserID, groupMessage)
			if err != nil {
				mylog.Printf("发送文本私聊信息失败: %v", err)
				mylog.Printf("%s", FormatQQError(err))
				// 22009: 主动消息超过频控限制，记录日志 (被动回复场景无需补偿)
    if IsQQError(err, 22009) {
					mylog.Printf("私聊主动消息受限(code:22009)，消息被丢弃: %s", messageText)
					if config.GetSaveError() {
						mylog.ErrLogToFile("type", "PostC2CMessage-22009")
						mylog.ErrInterfaceToFile("request", groupMessage)
						mylog.ErrLogToFile("error", err.Error())
					}
					//如果失败 防止进入递归
					return "", nil
				}
				// 请求参数 event_id 无效，清空后重试一次
    if IsQQError(err, 40034025) {
					groupMessage.EventID = ""
					resp, err = apiv2.PostC2CMessage(context.TODO(), UserID, groupMessage)
					if err != nil {
						mylog.Printf("发送文本私聊信息失败 on code 40034025: %v", err)
						return "", nil
					}
				}
				// 超时重试
    if IsDeliveryTimeout(err) {
					resp, err = postC2CMessageWithRetry(apiv2, UserID, groupMessage)
					if err != nil {
						return "", nil
					}
				}
			}
			//发送成功回执
			retmsg, _ = SendC2CResponse(client, err, &message, resp)
		}

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
				msgseq := echo.IncrementMappingSeq(messageID)
				groupReply := generatePrivateMessage(messageID, eventID, singleItem, "", msgseq, apiv2, UserID)
				// 进行类型断言
				richMediaMessage, ok := groupReply.(*dto.RichMediaMessage)
				if !ok {
				 // key 是 for key, urls := range foundItems { 这里的 key
				 if _, exists := sendPrivateMsgKeyMap[key]; exists {
						// 进行类型断言
						groupMessage, ok := groupReply.(*dto.MessageToCreate)
						       if !ok {
						        mylog.Println("Error: Expected MessageToCreate type.")
						        return "", nil // 或其他错误处理
						       }

						       // 将 reply 引用合并到 markdown 消息中（私聊场景校验避免越权）
						       if replyIDs, ok := foundItems["reply_msg_id"]; ok && len(replyIDs) > 0 && key == "markdown" {
						        applyPrivateReply(groupMessage, replyIDs, UserID)
						       }

						       // 将独立 [CQ:keyboard] 合并到 markdown 消息中（markdown JSON 未内嵌 keyboard 时）
						       if groupMessage.Keyboard == nil {
						        if kbItems, ok := foundItems["keyboard"]; ok && len(kbItems) > 0 {
						         kb, err := parseKeyboardData([]byte(kbItems[0]))
						         if err != nil || kb == nil {
						          mylog.Printf("[CQ:keyboard] 解析键盘数据失败: %v", err)
						         } else {
						          // 替换 keyboard 中 __USER_ID__ 占位符为实际用户 OpenID
						          userOpenID := idmap.ResolveOriginalID(UserID)
						          ResolvePlaceholderUserIDs(kb, userOpenID)
						          ResolveKeyboardImages(kb, apiv2)
						          groupMessage.Keyboard = kb
						          delete(foundItems, "keyboard")
						          mylog.Printf("[CQ:keyboard] 私聊 markdown 消息附加内嵌键盘")
						         }
						        }
						       }

						       // 首次发送私聊 MessageToCreate
						resp, err = apiv2.PostC2CMessage(context.TODO(), UserID, groupMessage)
						if err != nil {
							mylog.Printf("发送 MessageToCreate 私聊信息失败: %v", err)
							mylog.Printf("%s", FormatQQError(err))
							// 错误保存到本地
							if config.GetSaveError() {
								mylog.ErrLogToFile("type", "PostC2CMessage")
								mylog.ErrInterfaceToFile("request", groupMessage)
								mylog.ErrLogToFile("error", err.Error())
							}
						}

      if IsQQError(err, 22009) {
						 mylog.Printf("私聊主动消息受限(code:22009)，消息被丢弃")
						 if config.GetSaveError() {
						  mylog.ErrLogToFile("type", "PostC2CMessage-22009")
						  mylog.ErrInterfaceToFile("request", groupMessage)
						  mylog.ErrLogToFile("error", err.Error())
						 }

      } else if IsQQError(err, 40034025) {
							// 请求参数 event_id 无效，清空后重试一次
							groupMessage.EventID = ""
							//重新为err赋值
							resp, err = apiv2.PostC2CMessage(context.TODO(), UserID, groupMessage)
							if err != nil {
								mylog.Printf("发送 MessageToCreate 私聊信息失败 on code 40034025: %v", err)
								// 错误保存到本地
								if config.GetSaveError() {
									mylog.ErrLogToFile("type", "PostC2CMessage")
									mylog.ErrInterfaceToFile("request", groupMessage)
									mylog.ErrLogToFile("error", err.Error())
								}
							}

      } else if IsDeliveryTimeout(err) {
							// 仅对超时做有限次重试
							resp, err = postC2CMessageWithRetry(apiv2, UserID, groupMessage)
						}

						// 发送成功或最终失败后，都尝试回执（err 里能体现成功/失败）
						retmsg, _ = SendC2CResponse(client, err, &message, resp)
					}
					continue // 跳过这个项，继续下一个
				}

				// 发媒体
				message_return, err := apiv2.PostC2CMessage(context.TODO(), UserID, richMediaMessage)
				if err != nil {
					mylog.Printf("发送 %s 信息失败_send_private_msg: %v", key, err)

					// 错误保存到本地
					if config.GetSaveError() {
						mylog.ErrLogToFile("type", "PostC2CMessage")
						mylog.ErrInterfaceToFile("request", richMediaMessage)
						mylog.ErrLogToFile("error", err.Error())
					}
				}

				// 22009: 主动消息超过频控限制
    if IsQQError(err, 22009) {
					mylog.Printf("私聊富媒体主动消息受限(code:22009): %s", key)
					if config.GetSaveError() {
						mylog.ErrLogToFile("type", "PostC2CMessage-22009")
						mylog.ErrInterfaceToFile("request", richMediaMessage)
						mylog.ErrLogToFile("error", err.Error())
					}
				}

				// 仅对超时做重试，使用原始富媒体消息，不再构造错误文本消息
    if IsDeliveryTimeout(err) {
					message_return, err = postC2CRichMediaMessageWithRetry(apiv2, UserID, richMediaMessage)
				}

				if message_return != nil && message_return.MediaResponse != nil && message_return.MediaResponse.FileInfo != "" {
					msgseq := echo.IncrementMappingSeq(messageID)
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

					// 处理 [CQ:reply,id=数字] → message_reference + msg_id (富媒体消息，私聊场景校验避免越权)
					if replyIDs, ok := foundItems["reply_msg_id"]; ok && len(replyIDs) > 0 {
					 applyPrivateReply(groupMessage, replyIDs, UserID)
					}

					//重新为err赋值
					resp, err = apiv2.PostC2CMessage(context.TODO(), UserID, groupMessage)
					if err != nil {
						mylog.Printf("发送 %s 私聊信息失败: %v", key, err)
					}
				}
				//发送成功回执
				retmsg, _ = SendC2CResponse(client, err, &message, resp)
			}
		}
	default:
		mylog.Printf("Unknown message type: %s", msgType)
	}

	// 如果递归id不是10(不递归特殊值)
	if echo.GetMapping(idInt64) != 10 {
		//重置递归类型
		if echo.GetMapping(idInt64) <= 0 {
			echo.AddMsgType(config.GetAppIDStr(), idInt64, "")
		}
		echo.AddMapping(idInt64, echo.GetMapping(idInt64)-1)

		//递归3次枚举类型
		if echo.GetMapping(idInt64) > 0 {
			tryMessageTypes := []string{"group"}
			messageCopy := message // 创建message的副本
			echo.AddMsgType(config.GetAppIDStr(), idInt64, tryMessageTypes[echo.GetMapping(idInt64)-1])
			delay := config.GetSendDelay()
			time.Sleep(time.Duration(delay) * time.Millisecond)
			retmsg, _ = HandleSendPrivateMsg(client, api, apiv2, messageCopy)
		}
	}

	return retmsg, nil
}

// applyPrivateReply 将虚拟 reply ID 反查为真实 QQ API msg_id 并设置到私聊消息上。
// 关键：私聊场景只能引用私聊自身的 msg_id，若反查得到的真实 ID 归属其他场景
// （如群聊），QQ API 会返回 40034024 越权。因此反查前先用 RetrieveRowByIDv2 校验
// 虚拟 ID 的归属真实 ID 是否为当前私聊目标 UserID，不一致则跳过 reply 避免越权。
func applyPrivateReply(groupMessage *dto.MessageToCreate, replyIDs []string, privateUserID string) {
	if len(replyIDs) == 0 {
		return
	}
	// 反查前校验：虚拟 reply ID 的归属真实 ID 是否为当前私聊目标 UserID。
	// 私聊虚拟 ID 对应私聊 UserID，群聊虚拟 ID 对应群聊 GroupID，二者归属不同。
	ownerRealID, err := idmap.RetrieveRowByIDv2(replyIDs[0])
	if err != nil || ownerRealID == "" {
		mylog.Printf("[CQ:reply] 虚拟 ID %s 归属反查失败: %v", replyIDs[0], err)
		return
	}
	if ownerRealID != privateUserID {
		mylog.Printf("[CQ:reply] 跳过私聊reply：归属 %s 与私聊目标 %s 不一致（跨场景越权）",
			ownerRealID, privateUserID)
		return
	}
	// 归属校验通过，反查真实 msg_id 并设置 reply
	realReplyID, err := idmap.RetrieveRowByCachev2(replyIDs[0])
	if err != nil || realReplyID == "" {
		mylog.Printf("[CQ:reply] 虚拟 ID %s 反查失败: %v", replyIDs[0], err)
		return
	}
	// echo 缓存中的真实 ID 格式可能为 "UserID MessageID" 或纯 msg_id，取后半段
	parts := strings.Split(realReplyID, " ")
	refID := parts[len(parts)-1]
	groupMessage.MessageReference = &dto.MessageReference{
		MessageID:             ResolveReplyRefID(refID),
		IgnoreGetMessageError: false,
	}
	groupMessage.MsgID = refID
	// msg_id 与 event_id 二选一，清空 event_id
	groupMessage.EventID = ""
	mylog.Printf("[CQ:reply] 设置私聊回复消息: msg_id=%s", refID)
}

// 这个函数可以通过int类型的虚拟userid反推真实的guild_id和channel_id
func getGuildIDFromMessage(message callapi.ActionMessage) (string, string, error) {
	var userID string

	// 判断UserID的类型，并将其转换为string
	switch v := message.Params.UserID.(type) {
	case int:
		userID = strconv.Itoa(v)
	case float64:
		userID = strconv.FormatInt(int64(v), 10) // 将float64先转为int64，然后再转为string
	case string:
		userID = v
	default:
		return "", "", fmt.Errorf("unexpected type for UserID: %T", v) // 使用%T来打印具体的类型
	}
	var realUserID string
	var err error
	// 使用RetrieveRowByIDv2还原真实的UserID
	realUserID, err = idmap.RetrieveRowByIDv2(userID)
	if err != nil {
		return "", "", fmt.Errorf("error retrieving real UserID: %v", err)
	}
	// 使用realUserID作为sectionName从数据库中获取channel_id
	channelID, err := idmap.ReadConfigv2(realUserID, "channel_id")
	if err != nil {
		return "", "", fmt.Errorf("error reading channel_id: %v", err)
	}
	//使用channelID作为sectionName从数据库中获取guild_id
	guildID, err := idmap.ReadConfigv2(channelID, "guild_id")
	if err != nil {
		return "", "", fmt.Errorf("error reading guild_id: %v", err)
	}

	return guildID, channelID, nil
}

// 这个函数可以通过int类型的虚拟groupid反推真实的guild_id和channel_id
func getGuildIDFromMessagev2(message callapi.ActionMessage) (string, string, error) {
	var GroupID string
	//groupID此时是转换后的channelid

	// 判断UserID的类型，并将其转换为string
	switch v := message.Params.GroupID.(type) {
	case int:
		GroupID = strconv.Itoa(v)
	case float64:
		GroupID = strconv.FormatInt(int64(v), 10) // 将float64先转为int64，然后再转为string
	case string:
		GroupID = v
	default:
		return "", "", fmt.Errorf("unexpected type for UserID: %T", v) // 使用%T来打印具体的类型
	}

	var err error
	//使用channelID作为sectionName从数据库中获取guild_id
	guildID, err := idmap.ReadConfigv2(GroupID, "guild_id")
	if err != nil {
		return "", "", fmt.Errorf("error reading guild_id: %v", err)
	}

	return guildID, GroupID, nil
}

// uploadMedia 上传媒体并返回FileInfo
// 智能选择：小文件走 URL 直传，大文件自动切到分片上传
func uploadMediaPrivate(ctx context.Context, UserID string, richMediaMessage *dto.RichMediaMessage, apiv2 openapi.OpenAPI) (string, error) {
	// 如果有 base64 文件数据且超过软限制，走分片上传
	if richMediaMessage.FileData != "" {
		decoded, err := base64.StdEncoding.DecodeString(richMediaMessage.FileData)
		if err == nil && needsChunkedUpload(int64(len(decoded)), int(richMediaMessage.FileType)) {
			mylog.Printf("文件超过软限制，自动切换到分片上传 (type=%d, size=%d)",
				richMediaMessage.FileType, len(decoded))
			return chunkedUpload(ctx, apiv2, UserID, false, decoded, int(richMediaMessage.FileType), richMediaMessage.FileName)
		}
	}
	// URL 直传（默认路径）
	messageReturn, err := apiv2.PostC2CMessage(ctx, UserID, richMediaMessage)
	if err != nil {
		return "", err
	}
	// 返回上传后的FileInfo
	return messageReturn.MediaResponse.FileInfo, nil
}

// 私聊富媒体专用重试：针对 context deadline exceeded
func postC2CRichMediaMessageWithRetry(
	apiv2 openapi.OpenAPI,
	userID string,
	richMediaMessage *dto.RichMediaMessage,
) (resp *dto.C2CMessageResponse, err error) {
	richMediaMessage.EventID = ""
	retryCount := 3 // 设置最大重试次数为 3
	for i := 0; i < retryCount; i++ {
		resp, err = apiv2.PostC2CMessage(context.TODO(), userID, richMediaMessage)

		if err != nil && defaultRetryPolicy.ShouldRetry(err, i) {
			// 仅对超时做重试
			mylog.Printf("私聊富媒体超时重试第 %d 次: %v", i+1, err)
			if config.GetSaveError() {
				mylog.ErrLogToFile("type", "PostC2CRichMediaMessage-context-deadline-exceeded-retry-"+strconv.Itoa(i+1))
				mylog.ErrInterfaceToFile("request", richMediaMessage)
				mylog.ErrLogToFile("error", err.Error())
			}
			time.Sleep(defaultRetryPolicy.Backoff(i + 1))
			continue
		}

		// 成功 或 非超时错误，统一在这里收尾然后 break
		if config.GetSaveError() {
			logType := "PostC2CRichMediaMessage-final"
			if err == nil {
				logType = "PostC2CRichMediaMessage-retry-success-" + strconv.Itoa(i+1)
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

func postC2CMessageWithRetry(apiv2 openapi.OpenAPI, userID string, msg *dto.MessageToCreate) (resp *dto.C2CMessageResponse, err error) {
	retryCount := 3 // 设置最大重试次数为 3
	for i := 0; i < retryCount; i++ {
		// 递增 msgseq（沿用你群聊那套映射逻辑）
		msgseq := echo.NextMappingSeq(msg.MsgID)
		msg.MsgSeq = msgseq

		resp, err = apiv2.PostC2CMessage(context.TODO(), userID, msg)
		if err != nil && defaultRetryPolicy.ShouldRetry(err, i) {
			mylog.Printf("私聊超时重试第 %d 次: %v", i+1, err)
			if config.GetSaveError() {
				mylog.ErrLogToFile("type", "PostC2CMessage-context-deadline-exceeded-retry-"+strconv.Itoa(i+1))
				mylog.ErrInterfaceToFile("request", msg)
				mylog.ErrLogToFile("error", err.Error())
			}
			time.Sleep(defaultRetryPolicy.Backoff(i + 1))
			continue
		} else {
			// 成功 或 非超时错误，统一在这里收尾
			mylog.Printf("私聊超时重试第 %d 次结束: %v", i+1, err)
			if config.GetSaveError() {
				suffix := "-successed"
				if err != nil {
					suffix = "-failed"
				}
				mylog.ErrLogToFile("type", "PostC2CMessage-context-deadline-exceeded-retry-"+strconv.Itoa(i+1)+suffix)
				mylog.ErrInterfaceToFile("request", msg)
				if resp != nil {
					mylog.ErrLogToFile("msgid", resp.Message.ID)
				}
				if err != nil {
					mylog.ErrLogToFile("error", err.Error())
				}
			}
		}
		break
	}
	return resp, err
}

package handlers

import (
	"context"
	"fmt"
	"strconv"
	"time"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/internal/domain/identity"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

// WakeupResponseNotice 伪装成 Notice 的响应结构
type WakeupResponseNotice struct {
	PostType   string `json:"post_type"`
	NoticeType string `json:"notice_type"`

	UserID     int64  `json:"user_id"`                // 【修改】改成 int64，兼容 NoticeEvent
	RealUserID string `json:"real_user_id,omitempty"` // 【新增】用来放 32 位 OpenID String

	Status    string `json:"status"`
	MessageID string `json:"message_id,omitempty"`
	ErrorMsg  string `json:"error_msg,omitempty"`
	Time      int64  `json:"time"`
	SelfID    int64  `json:"self_id"`
}

func init() {
	callapi.RegisterHandler("send_private_msg_wakeup", HandleSendPrivateMsgWakeup)
}

// HandleSendPrivateMsgWakeup 处理私聊互动召回消息
// 逻辑高度复刻 HandleSendPrivateMsg，但适配 IsWakeup 参数并移除 ID 转换逻辑
func HandleSendPrivateMsgWakeup(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	// 1. 获取 UserID，支持虚拟数字 ID → 真实 OpenID 自动转换
	userID, ok := message.Params.UserID.(string)
	if !ok || userID == "" {
		mylog.Printf("send_private_msg_wakeup 错误: UserID 为空")
		return sendActionResult(client, message, "缺少 user_id 参数", sendParamRetCode)
	}

	// 如果不是 OpenID，尝试从 idmap 转换（虚拟数字 ID → 真实 OpenID）
	if !identity.IsOpenID(userID) {
		realID, err := idmap.RetrieveRowByIDv2(userID)
		if err == nil && identity.IsOpenID(realID) {
			mylog.Printf("send_private_msg_wakeup: 虚拟ID %s → 真实OpenID %s", userID, realID)
			userID = realID
		} else {
			mylog.Printf("send_private_msg_wakeup 错误: UserID 不是32位OpenID且无法转换: %s", userID)
			return sendActionResult(client, message, "无法反查用户 OpenID", sendParamRetCode)
		}
	}

	// 此时 selfID 最好从配置或 message 中获取，这里演示用 0 或 message.SelfID (如果你的ActionMessage里有)
	var selfID int64 = int64(config.GetAppID())

	// 2. 解析消息内容
	messageText, foundItems, msgPendings := parseMessageContent(message.Params, message, client, api, apiv2)
	// 修 M1：wakeup（私聊）路径显式拦截动作码（Parse 内已按 Scope 拦截，此处兜底丢弃）
	if len(msgPendings) > 0 {
		mylog.Printf("[cqparse] wakeup 私聊路径拦截 %d 个动作码,不执行不发送", len(msgPendings))
	}

	// A 包出站接缝：arch_mode=new 走统一出站服务（wakeup 语义）；shadow 执行并 diff；legacy 原逻辑。
	if handled, ret := TryOutbound(client, api, apiv2, message, messageText, foundItems, identity.TargetPrivate, true); handled {
		return ret, nil
	}

	mylog.Printf("发送互动召回消息 UserID:[%s]", userID)

	// 定义 KeyMap (对齐 HandleSendPrivateMsg)
	keyMap := map[string]bool{
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

	var singleItem = make(map[string][]string)
	var imageType, imageUrl string
	imageCount := 0
	// 是否已发出至少一条召回消息：markdown/keyboard 等键被前置装配消费后，
	// 末尾不再补发一条空的占位召回。
	sentAny := false
	// 首个失败原因：各场景跑完后统一回一条失败回执，避免多场景各回一条（同一请求只回一条 action 回执）。
	var wakeupErr error
	recordFailure := func(err error) {
		if err != nil && wakeupErr == nil {
			wakeupErr = err
		}
	}

	// 检查图片并计算数量 (复刻原逻辑)
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

	// --- 场景 A: 单图文混合消息 ---
	if imageCount == 1 && messageText != "" {
		mylog.Printf("发送召回图文混合信息")
		singleItem[imageType] = []string{imageUrl}

		// 生成消息对象 (MsgID/EventID 传空，因为是主动召回)
		groupReply := generatePrivateMessage("", "", singleItem, "", 0, apiv2, userID)

		// 类型断言
		richMediaMessage, ok := groupReply.(*dto.RichMediaMessage)
		if !ok {
			mylog.Printf("Error: Expected RichMediaMessage type for key")
			return sendActionResult(client, message, "图文消息生成失败: 消息类型不匹配", sendFailRetCode)
		}

		// 上传图片 (不需要 IsWakeup，只是为了拿 FileInfo)
		fileInfo, err := uploadMediaPrivate(context.TODO(), userID, richMediaMessage, apiv2)
		if err != nil {
			mylog.Printf("上传图片失败: %v", err)
			sendWakeupNotice(client, userID, nil, err, selfID)
			return sendActionResult(client, message, "上传图片失败: "+err.Error(), sendFailRetCode)
		}

		// 构造 MessageToCreate
		groupMessage := &dto.MessageToCreate{
			Content: messageText,
			Media: &dto.Media{
				FileInfo: fileInfo,
			},
			MsgType:  7,    // 富媒体类型
			IsWakeup: true, // [重点] 标记为召回
			MsgID:    "",   // [重点] 互斥
			EventID:  "",   // [重点] 互斥
		}

		// 发送
		resp, err := postC2CWakeupMessageWithRetry(apiv2, userID, groupMessage)
		sendWakeupNotice(client, userID, resp, err, selfID)
		recordFailure(err)
		sentAny = true

		delete(foundItems, imageType)
		messageText = ""
	}

	// [CQ:stream] 单聊流式消息：复用 send_private_msg 的流式发送（同一套解析与 stream_msg_id 分片缓存）
	if ret, present := sendStream(client, &message, "", userID, messageText, foundItems, apiv2); present {
		return ret, nil
	}

	// [CQ:card] 图文卡片为群聊专用（msg_type=8，dto.GroupCard 仅群聊可用），私聊召回无对应能力：
	// 明确丢弃并记 warn，避免静默消失
	if _, ok := foundItems["card"]; ok {
		mylog.Printf("[CQ:card] 私聊召回不支持图文卡片(msg_type=8 仅群聊)，已忽略")
		delete(foundItems, "card")
	}

	// --- 场景 B: 纯文本 / markdown / 独立 keyboard（与 send_private_msg 同款装配） ---
	if messageText != "" || len(foundItems["markdown"]) > 0 || len(foundItems["keyboard"]) > 0 {
		// 转换 [CQ:at] 为 @用户名
		messageText = resolvePlainTextAtMentions(messageText)
		// 复用 generatePrivateMessage 获取基础结构 (虽然可以直接构造，但为了保持风格一致)
		groupReply := generatePrivateMessage("", "", nil, messageText, 0, apiv2, userID)

		if groupMessage, ok := groupReply.(*dto.MessageToCreate); ok {
			// 复用私聊共用装配：markdown 内嵌 keyboard / 独立 [CQ:keyboard] 一并附加
			// （含 __USER_ID__ 占位符替换与 C2C 权限归一），并从 foundItems 消费掉
			applyPrivateMarkdownKeyboard(groupMessage, foundItems, userID, apiv2)
			// 强制覆盖为召回模式
			groupMessage.IsWakeup = true
			groupMessage.MsgID = ""
			groupMessage.EventID = ""

			resp, err := postC2CWakeupMessageWithRetry(apiv2, userID, groupMessage)
			sendWakeupNotice(client, userID, resp, err, selfID)
			recordFailure(err)
			sentAny = true
		} else {
			mylog.Println("Error: Expected MessageToCreate type for text.")
			recordFailure(fmt.Errorf("文本消息生成失败: 消息类型不匹配"))
		}
	}

	// --- 场景 C: 遍历 foundItems (核心复刻部分) ---
	for key, urls := range foundItems {
		// [CQ:active] / [CQ:wakeup] 已被消费，跳过
		if key == "active" || key == "active_type" || key == "active_sub_type" || key == "wakeup" {
			continue
		}
		for _, url := range urls {
			var singleItem = make(map[string][]string)
			singleItem[key] = []string{url}

			// 生成消息对象
			groupReply := generatePrivateMessage("", "", singleItem, "", 0, apiv2, userID)

			// 1. 尝试断言为 RichMediaMessage (通常是需要上传的媒体)
			richMediaMessage, ok := groupReply.(*dto.RichMediaMessage)
			if !ok {
				// 如果不是 RichMediaMessage，检查是否在 KeyMap 中 (Markdown, QQMusic 等)
				if _, exists := keyMap[key]; exists {
					// 断言为 MessageToCreate
					groupMessage, ok := groupReply.(*dto.MessageToCreate)
					if !ok {
						mylog.Println("Error: Expected MessageToCreate type.")
						recordFailure(fmt.Errorf("%s 消息生成失败: 消息类型不匹配", key))
						continue
					}

					// [关键修改] 适配召回消息格式
					groupMessage.IsWakeup = true
					groupMessage.MsgID = ""
					groupMessage.EventID = ""

					// 发送特殊类型的消息 (Markdown 等)
					resp, err := postC2CWakeupMessageWithRetry(apiv2, userID, groupMessage)

					// 错误处理逻辑复刻
					if err != nil {
						mylog.Printf("发送 MessageToCreate 召回信息失败: %v", err)
						mylog.Printf("%s", FormatQQError(err))
						if config.GetSaveError() {
							mylog.ErrLogToFile("type", "PostC2CWakeup-Special")
							mylog.ErrInterfaceToFile("request", groupMessage)
							mylog.ErrLogToFile("error", err.Error())
						}
					}

					// 发送通知
					sendWakeupNotice(client, userID, resp, err, selfID)
					recordFailure(err)
					sentAny = true
				} else {
					mylog.Printf("Error: Expected RichMediaMessage type for key %s.", key)
				}
				continue // 继续下一个
			}

			// 2. 如果是 RichMediaMessage，执行上传 + 发送流程
			// 上传媒体 (这里不需要 IsWakeup)
			messageReturn, err := apiv2.PostC2CMessage(context.TODO(), userID, richMediaMessage)
			if err != nil {
				mylog.Printf("发送 %s 信息失败_upload: %v", key, err)
				if config.GetSaveError() {
					mylog.ErrLogToFile("type", "PostC2CWakeup-Upload")
					mylog.ErrInterfaceToFile("request", richMediaMessage)
					mylog.ErrLogToFile("error", err.Error())
				}
			}

			// 富媒体上传超时重试 (复刻原逻辑)
   if IsDeliveryTimeout(err) {
				// 注意：这里重试的是上传接口，不是发送接口
				// postC2CRichMediaMessageWithRetry 是你原有代码中的函数，可以直接复用
				// 如果那个函数没导出，可能需要复制一份改名，这里假设能调用或者逻辑一致
				messageReturn, err = postC2CRichMediaMessageWithRetry(apiv2, userID, richMediaMessage)
			}

			// 上传成功，构造最终的 MessageToCreate 进行发送
			if messageReturn != nil && messageReturn.MediaResponse != nil && messageReturn.MediaResponse.FileInfo != "" {
				media := dto.Media{
					FileInfo: messageReturn.MediaResponse.FileInfo,
				}
				groupMessage := &dto.MessageToCreate{
					Content:  " ", // 媒体消息通常带个空格
					MsgType:  7,   // 富媒体
					Media:    &media,
					IsWakeup: true, // [重点]
					MsgID:    "",
					EventID:  "",
				}

				// 发送最终的召回消息
				resp, err := postC2CWakeupMessageWithRetry(apiv2, userID, groupMessage)
				if err != nil {
					mylog.Printf("发送 %s 召回私聊信息失败: %v", key, err)
				}

				// 发送通知
				sendWakeupNotice(client, userID, resp, err, selfID)
				recordFailure(err)
				sentAny = true
			} else {
				// 上传都失败了，发一个失败通知
				uploadErr := fmt.Errorf("media upload failed for %s: %v", key, err)
				sendWakeupNotice(client, userID, nil, uploadErr, selfID)
				recordFailure(uploadErr)
			}
		}
	}

	// 如果所有场景都跳过了（如纯 [CQ:active]/[CQ:wakeup] 无实际内容），至少发送唤醒请求
	hasRealContent := false
	for k := range foundItems {
		if k != "active" && k != "active_type" && k != "active_sub_type" && k != "wakeup" {
			hasRealContent = true
			break
		}
	}
	if messageText == "" && !hasRealContent && !sentAny {
		groupMessage := &dto.MessageToCreate{
			Content:  " ",
			IsWakeup: true,
		}
		resp, err := postC2CWakeupMessageWithRetry(apiv2, userID, groupMessage)
		sendWakeupNotice(client, userID, resp, err, selfID)
		recordFailure(err)
	}

	if wakeupErr != nil {
		return sendActionResult(client, message, "召回消息发送失败: "+wakeupErr.Error(), sendFailRetCode)
	}

	return `{"status": "ok", "retcode": 0}`, nil
}

// postC2CWakeupMessageWithRetry 召回消息专用重试逻辑
// 保持了 3 次重试和详细的错误落盘
func postC2CWakeupMessageWithRetry(apiv2 openapi.OpenAPI, userID string, msg *dto.MessageToCreate) (resp *dto.C2CMessageResponse, err error) {
	retryCount := 3
	for i := 0; i < retryCount; i++ {
		// 召回消息不需要 MsgSeq
		resp, err = apiv2.PostC2CMessage(context.TODO(), userID, msg)

  if IsDeliveryTimeout(err) {
			mylog.Printf("召回消息超时重试第 %d 次: %v", i+1, err)
			if config.GetSaveError() {
				mylog.ErrLogToFile("type", "PostC2CWakeup-Retry-"+strconv.Itoa(i+1))
				mylog.ErrInterfaceToFile("request", msg)
				mylog.ErrLogToFile("error", err.Error())
			}
			time.Sleep(3 * time.Second)
			continue
		} else {
			// 成功 或 非超时错误
			mylog.Printf("召回消息请求结束: %v", err)
			if config.GetSaveError() {
				suffix := "-successed"
				if err != nil {
					suffix = "-failed"
				}
				mylog.ErrLogToFile("type", "PostC2CWakeup-Retry-"+strconv.Itoa(i+1)+suffix)
				mylog.ErrInterfaceToFile("request", msg)
				if resp != nil {
					mylog.ErrLogToFile("msg_id", resp.Message.ID)
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

// sendWakeupNotice 将发送结果伪装成 Notice 发送给应用端
func sendWakeupNotice(client callapi.Client, userID string, resp *dto.C2CMessageResponse, err error, selfID int64) {

	// 1. 构造结构体
	notice := WakeupResponseNotice{
		PostType:   "notice",
		NoticeType: "c2c_wakeup_resp",

		UserID:     0,      // 【关键】设为 0，防止 json.Unmarshal 报错
		RealUserID: userID, // 【关键】真正的 String ID 放这里

		Time:   time.Now().Unix(),
		SelfID: selfID,
	}

	if err != nil {
		notice.Status = "failed"
		notice.ErrorMsg = err.Error()
	} else {
		notice.Status = "success"
		if resp != nil && resp.Message != nil {
			notice.MessageID = resp.Message.ID
		}
	}

	// 2. 转换为 Map
	outputMap := structToMap(notice)

	// 3. 推送给应用端
	if sendErr := client.SendMessage(outputMap); sendErr != nil {
		mylog.Printf("发送召回结果通知失败: %v", sendErr)
	}
}

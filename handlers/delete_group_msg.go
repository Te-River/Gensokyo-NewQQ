package handlers

import (
	"context"
	"encoding/json"
	"fmt"
	"strconv"
	"strings"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/echo"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

type deleteGroupMsgResponse struct {
	Data    interface{} `json:"data"`
	Message string      `json:"message"`
	RetCode int         `json:"retcode"`
	Status  string      `json:"status"`
	Echo    interface{} `json:"echo,omitempty"`
}

type deleteGroupMsgTarget struct {
	GroupOpenID string
	UserOpenID  string
	MessageID   string
	BotMessage  bool
}

var (
	deleteGroupMsgResolveOriginalID = idmap.ResolveOriginalID
	deleteGroupMsgGetLatestUser     = idmap.GetLatestMsgID
	deleteGroupMsgGetLatestBot      = idmap.GetLatestBotMsgID
	deleteGroupMsgRetract           = func(api openapi.OpenAPI, groupOpenID, messageID string) error {
		return api.RetractGroupMessage(context.TODO(), groupOpenID, messageID, openapi.RetractMessageOptionHidetip)
	}
)

func init() {
	callapi.RegisterHandler("delete_group_msg", DeleteGroupMsg)
}

// DeleteGroupMsg withdraws a QQ group message.
//
// group_id is required. A positive user_id selects that user's message and is
// required when message_id is omitted. Missing, zero, or negative user_id
// selects a message sent by the QQ Bot itself, matching delete_msg semantics.
func DeleteGroupMsg(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	target, err := resolveDeleteGroupMsgTarget(message.Params)
	if err != nil {
		return sendDeleteGroupMsgResponse(client, message.Echo, false, err.Error(), 1400)
	}

	if err := deleteGroupMsgRetract(apiv2, target.GroupOpenID, target.MessageID); err != nil {
		mylog.Printf("delete_group_msg 撤回失败: group=%s user=%s message_id=%s bot_message=%t error=%v",
			target.GroupOpenID, target.UserOpenID, target.MessageID, target.BotMessage, err)
		return sendDeleteGroupMsgResponse(client, message.Echo, false, err.Error(), 1500)
	}

	mylog.Printf("delete_group_msg 撤回成功: group=%s user=%s message_id=%s bot_message=%t",
		target.GroupOpenID, target.UserOpenID, target.MessageID, target.BotMessage)
	return sendDeleteGroupMsgResponse(client, message.Echo, true, "", 0)
}

func resolveDeleteGroupMsgTarget(params callapi.ParamsContent) (deleteGroupMsgTarget, error) {
	groupID := actionIDString(params.GroupID)
	if groupID == "" || isNonPositiveNumericID(groupID) {
		return deleteGroupMsgTarget{}, fmt.Errorf("group_id 为必填参数，且必须是有效的虚拟群ID或群OpenID")
	}
	groupOpenID, err := resolveDeleteGroupOpenID(groupID, "group_id")
	if err != nil {
		return deleteGroupMsgTarget{}, err
	}

	userID := actionIDString(params.UserID)
	botMessage := userID == "" || isNonPositiveNumericID(userID)
	var userOpenID string
	if !botMessage {
		userOpenID, err = resolveDeleteGroupOpenID(userID, "user_id")
		if err != nil {
			return deleteGroupMsgTarget{}, err
		}
	}

	messageID := actionIDString(params.MessageID)
	if messageID == "" {
		if botMessage {
			messageID, err = deleteGroupMsgGetLatestBot(groupOpenID)
			if err != nil {
				return deleteGroupMsgTarget{}, fmt.Errorf("未找到机器人在群 %s 发送的最后一条消息: %w", groupID, err)
			}
		} else {
			messageID, err = deleteGroupMsgGetLatestUser(groupOpenID, userOpenID)
			if err != nil {
				return deleteGroupMsgTarget{}, fmt.Errorf("未找到用户 %s 在群 %s 发送的最后一条消息: %w", userID, groupID, err)
			}
		}
	} else {
		messageID, err = resolveDeleteGroupMessageID(messageID)
		if err != nil {
			return deleteGroupMsgTarget{}, err
		}
	}

	if strings.TrimSpace(messageID) == "" {
		return deleteGroupMsgTarget{}, fmt.Errorf("message_id 解析结果为空")
	}
	return deleteGroupMsgTarget{
		GroupOpenID: groupOpenID,
		UserOpenID:  userOpenID,
		MessageID:   messageID,
		BotMessage:  botMessage,
	}, nil
}

func resolveDeleteGroupOpenID(value, fieldName string) (string, error) {
	resolved := strings.TrimSpace(deleteGroupMsgResolveOriginalID(value))
	if resolved == "" {
		return "", fmt.Errorf("%s 解析结果为空", fieldName)
	}
	if isNumericID(value) && resolved == value {
		return "", fmt.Errorf("%s=%s 无法从虚拟ID解析为OpenID", fieldName, value)
	}
	return resolved, nil
}

func resolveDeleteGroupMessageID(value string) (string, error) {
	value = strings.TrimSpace(value)
	if value == "" || isNonPositiveNumericID(value) {
		return "", fmt.Errorf("message_id 必须是有效的虚拟消息ID或实际消息ID")
	}
	if !isNumericID(value) {
		return value, nil
	}

	if config.GetMemoryMsgid() {
		if realID, ok := echo.GetCacheIDFromMemoryByRowID(value); ok && realID != "" {
			return realID, nil
		}
		return "", fmt.Errorf("message_id=%s 无法从内存映射解析", value)
	}
	realID, err := idmap.RetrieveRowByCachev2(value)
	if err != nil {
		return "", fmt.Errorf("message_id=%s 无法从虚拟ID解析: %w", value, err)
	}
	if realID == "" {
		return "", fmt.Errorf("message_id=%s 无法从虚拟ID解析", value)
	}
	return realID, nil
}

func actionIDString(value interface{}) string {
	switch typed := value.(type) {
	case nil:
		return ""
	case string:
		return strings.TrimSpace(typed)
	default:
		return strings.TrimSpace(fmt.Sprint(typed))
	}
}

func isNumericID(value string) bool {
	_, err := strconv.ParseInt(strings.TrimSpace(value), 10, 64)
	return err == nil
}

func isNonPositiveNumericID(value string) bool {
	number, err := strconv.ParseInt(strings.TrimSpace(value), 10, 64)
	return err == nil && number <= 0
}

func sendDeleteGroupMsgResponse(client callapi.Client, echoValue interface{}, success bool, message string, retCode int) (string, error) {
	status := "failed"
	if success {
		status = "ok"
	}
	response := deleteGroupMsgResponse{
		Data:    nil,
		Message: message,
		RetCode: retCode,
		Status:  status,
		Echo:    echoValue,
	}
	outputMap := structToMap(response)
	if err := client.SendMessage(outputMap); err != nil {
		return "", err
	}
	encoded, err := json.Marshal(response)
	if err != nil {
		return "", err
	}
	return string(encoded), nil
}

func rememberLatestBotGroupMessage(message *callapi.ActionMessage, response *dto.GroupMessageResponse) {
	if message == nil {
		return
	}
	rememberLatestBotGroupMessageInGroup(actionIDString(message.Params.GroupID), response)
}

func rememberLatestBotGroupMessageInGroup(groupID string, response *dto.GroupMessageResponse) {
	if response == nil || response.Message == nil || response.Message.ID == "" {
		return
	}
	if groupID == "" {
		return
	}
	groupOpenID := strings.TrimSpace(idmap.ResolveOriginalID(groupID))
	if groupOpenID == "" || (isNumericID(groupID) && groupOpenID == groupID) {
		return
	}
	idmap.StoreLatestBotMsgID(groupOpenID, response.Message.ID)
}

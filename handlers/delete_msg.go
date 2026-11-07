package handlers

import (
	"context"
	"encoding/json"
	"fmt"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/echo"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/internal/domain/identity"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/openapi"
)

func init() {
	callapi.RegisterHandler("delete_msg", DeleteMsg)
}

func DeleteMsg(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	var RealMsgID string
	var err error

	// message_id 为外部输入，缺失或类型不符时无法定位消息，直接返回错误
	msgID, ok := message.Params.MessageID.(string)
	if !ok || msgID == "" {
		mylog.Printf("delete_msg: message_id 缺失或类型错误，跳过撤回: %#v", message.Params.MessageID)
		return "", fmt.Errorf("delete_msg: message_id 缺失或类型错误")
	}

	// 不是stringob才需要转换
	if !config.GetStringOb11() {
		// 如果从内存取
		if config.GetMemoryMsgid() {
			//还原msgid
			RealMsgID, _ = echo.GetCacheIDFromMemoryByRowID(msgID)
		} else {
			//还原msgid
			RealMsgID, err = idmap.RetrieveRowByCachev2(msgID)
			if err != nil {
				mylog.Printf("error retrieving real RChannelID: %v", err)
			}
		}
	} else {
		RealMsgID = msgID
	}

	//重新赋值
	message.Params.MessageID = RealMsgID

	//撤回群信息
	if message.Params.GroupID != nil && message.Params.GroupID != "" {
		// 判断是否是原始id
		if !identity.IsOpenID(message.Params.GroupID.(string)) {
			var originalGroupID string
			originalGroupID, err := idmap.RetrieveRowByIDv2(message.Params.GroupID.(string))
			if err != nil {
				mylog.Printf("Error retrieving original GroupID: %v", err)
			}
			message.Params.GroupID = originalGroupID
			err = api.RetractGroupMessage(context.TODO(), message.Params.GroupID.(string), message.Params.MessageID.(string), openapi.RetractMessageOptionHidetip)
			if err != nil {
				fmt.Println("Error retracting group message:", err)
			}
		} else {
			err = api.RetractGroupMessage(context.TODO(), message.Params.GroupID.(string), message.Params.MessageID.(string), openapi.RetractMessageOptionHidetip)
			if err != nil {
				fmt.Println("Error retracting group message:", err)
			}
		}
	}

	//撤回C2C私信消息列表
	if message.Params.UserID != nil && message.Params.UserID != "" {
		var UserID string
		//还原真实的userid
		UserID, err := idmap.RetrieveRowByIDv2(message.Params.UserID.(string))
		if err != nil {
			mylog.Printf("Error reading config: %v", err)
			return "", nil
		}
		message.Params.UserID = UserID
		err = api.RetractC2CMessage(context.TODO(), message.Params.UserID.(string), message.Params.MessageID.(string), openapi.RetractMessageOptionHidetip)
		if err != nil {
			fmt.Println("Error retracting C2C message:", err)
		}

	}

	var response GetStatusResponse
	response.Message = ""
	response.RetCode = 0
	response.Status = "ok"
	response.Echo = message.Echo

	outputMap := structToMap(response)

	mylog.Printf("delete_msg: %+v\n", outputMap)

	err = client.SendMessage(outputMap)
	if err != nil {
		mylog.Printf("Error sending message via client: %v", err)
	}
	//把结果从struct转换为json
	result, err := json.Marshal(response)
	if err != nil {
		mylog.Printf("Error marshaling data: %v", err)
		//todo 符合onebotv11 ws返回的错误码
		return "", nil
	}
	return string(result), nil
}

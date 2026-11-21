package handlers

import (
	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/openapi"
)

type MarkThisMessageAsReadAPIResponse struct {
	Status  string      `json:"status"`
	Data    interface{} `json:"data"` // 空实现：零值 nil 序列化为 data:null（不暴露内部类型字段）
	Msg     string      `json:"msg"`
	Wording string      `json:"wording"`
	RetCode int64       `json:"retcode"`
	Echo    interface{} `json:"echo"`
}

func init() {
	callapi.RegisterHandler("mark_msg_as_read", MarkThisMessageAsRead)
}

func MarkThisMessageAsRead(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {

	var response MarkThisMessageAsReadAPIResponse

	// QQ官方API无已读上报接口,空实现:Data/Msg 保持零值,仅回执 ok
	response.RetCode = 0
	response.Status = "ok"
	response.Echo = message.Echo

	// Convert the members slice to a map
	outputMap := structToMap(response)

	mylog.Printf("mark_msg_as_read: %+v\n", outputMap)

	err := client.SendMessage(outputMap)
	if err != nil {
		mylog.Printf("Error sending message via client: %v", err)
	} else {
		mylog.Printf("响应mark_msg_as_read: %+v", outputMap)
	}
	//把结果从struct转换为json
	result, err := callapi.MarshalNoEscape(response)
	if err != nil {
		mylog.Printf("Error marshaling data: %v", err)
		// 已回执（上方 SendMessage）
		return "", nil
	}
	return string(result), nil
}

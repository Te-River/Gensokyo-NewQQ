package handlers

import (
	"encoding/json"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/openapi"
)

func init() {
	callapi.RegisterHandler("set_group_card", SetGroupCard)
}

// SetGroupCard mock implementation for OneBot 11 compatibility
func SetGroupCard(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	mylog.Printf("[MOCK] set_group_card API called, params: %+v", message.Params)

	response := struct {
		Message string      `json:"message"`
		RetCode int         `json:"retcode"`
		Status  string      `json:"status"`
		Echo    interface{} `json:"echo,omitempty"`
	}{
		Message: "",
		RetCode: 0,
		Status:  "ok",
		Echo:    message.Echo,
	}

	outputMap := structToMap(response)
	if client != nil {
		err := client.SendMessage(outputMap)
		if err != nil {
			mylog.Printf("[MOCK] Error sending message via client for set_group_card: %v", err)
		}
	}

	result, err := json.Marshal(response)
	if err != nil {
		mylog.Printf("[MOCK] Error marshaling response for set_group_card: %v", err)
		return "", err
	}

	return string(result), nil
}

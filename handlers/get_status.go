package handlers

import (
	"encoding/json"

	"github.com/hoshinonyaruko/gensokyo/botstats"
	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/openapi"
)

type GetStatusResponse struct {
	Data    StatusData  `json:"data"`
	Message string      `json:"message"`
	RetCode int         `json:"retcode"`
	Status  string      `json:"status"`
	Echo    interface{} `json:"echo"`
}

type StatusData struct {
	AppInitialized bool       `json:"app_initialized"`
	AppEnabled     bool       `json:"app_enabled"`
	PluginsGood    bool       `json:"plugins_good"`
	AppGood        bool       `json:"app_good"`
	Online         bool       `json:"online"`
	Good           bool       `json:"good"`
	Stat           Statistics `json:"stat"`
}

type Statistics struct {
	PacketReceived  uint64 `json:"packet_received"`
	PacketSent      uint64 `json:"packet_sent"`
	PacketLost      uint32 `json:"packet_lost"`
	MessageReceived int    `json:"message_received"`
	MessageSent     int    `json:"message_sent"`
	DisconnectTimes uint32 `json:"disconnect_times"`
	LostTimes       uint32 `json:"lost_times"`
	LastMessageTime int64  `json:"last_message_time"`
}

func init() {
	callapi.RegisterHandler("get_status", GetStatus)
}

func GetStatus(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {

	var response GetStatusResponse

	messageReceived, messageSent, lastMessageTime, err := botstats.GetStats()
	if err != nil {
		mylog.Printf("get_status错误,获取机器人发信状态错误:%v", err)
	}
	response.Data = StatusData{
		AppInitialized: true,
		AppEnabled:     true,
		PluginsGood:    true,
		AppGood:        true,
		Online:         true, // 进程存活即在线(中性值,官方无心跳状态查询接口)
		Good:           true, // 进程存活即在线(中性值,官方无心跳状态查询接口)
		Stat: Statistics{
			PacketReceived:  0,               // 官方API无包收发统计,诚实置0
			PacketSent:      0,               // 官方API无包收发统计,诚实置0
			PacketLost:      0,               // 官方API无包收发统计,诚实置0
			MessageReceived: messageReceived, // botstats 真实数据
			MessageSent:     messageSent,     // botstats 真实数据
			DisconnectTimes: 0,               // 官方API无断线统计且 wsclient 无导出连接计数,诚实置0
			LostTimes:       0,               // 官方API无丢包统计,诚实置0
			LastMessageTime: lastMessageTime, // botstats 真实数据
		},
	}
	response.Message = ""
	response.RetCode = 0
	response.Status = "ok"
	response.Echo = message.Echo

	outputMap := structToMap(response)

	mylog.Printf("get_status: %+v\n", outputMap)

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

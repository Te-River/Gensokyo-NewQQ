package handlers

import (
	"encoding/json"
	"strconv"
	"time"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/mylog"

	"github.com/tencent-connect/botgo/openapi"
)

func init() {
	callapi.RegisterHandler("get_group_info", HandleGetGroupInfo)
}

type OnebotGroupInfo struct {
	Data    GroupInfo   `json:"data"`
	Message string      `json:"message"`
	RetCode int         `json:"retcode"`
	Status  string      `json:"status"`
	Echo    interface{} `json:"echo"`
}

type GroupInfo struct {
	GroupID         int64  `json:"group_id"`
	GroupName       string `json:"group_name"`
	GroupMemo       string `json:"group_memo"`
	GroupCreateTime int32  `json:"group_create_time"`
	GroupLevel      int32  `json:"group_level"`
	MemberCount     int32  `json:"member_count"`
	MaxMemberCount  int32  `json:"max_member_count"`
}

func HandleGetGroupInfo(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	var groupInfo *OnebotGroupInfo
	var groupid int64
	groupid, _ = strconv.ParseInt(message.Params.GroupID.(string), 10, 64)
	groupCreateTime := time.Now().Unix()
	// 创建 GroupInfo 实例
	groupInfo1 := &GroupInfo{
		GroupID:         groupid,
		GroupName:       "测试群",
		GroupMemo:       "这是一个测试群",
		GroupCreateTime: int32(groupCreateTime),
		GroupLevel:      0,
		MemberCount:     500,
		MaxMemberCount:  1000,
	}
	// 创建 OnebotGroupInfo 实例并嵌入 GroupInfo
	groupInfo = &OnebotGroupInfo{
		Data:    *groupInfo1, // 将 groupInfo 添加到 Data 切片中
		Message: "success",
		RetCode: 0,
		Status:  "ok",
	}
	if message.Echo == "" {
		groupInfo.Echo = "0"
	} else {
		groupInfo.Echo = message.Echo
	}
	groupInfoMap := structToMap(groupInfo)

	// 打印groupInfoMap的内容
	mylog.Printf("groupInfoMap: %+v\n", groupInfoMap)

	err := client.SendMessage(groupInfoMap) //发回去
	if err != nil {
		mylog.Printf("error sending group info via wsclient: %v", err)
	}
	//把结果从struct转换为json
	result, err := json.Marshal(groupInfo)
	if err != nil {
		mylog.Printf("Error marshaling data: %v", err)
		//todo 符合onebotv11 ws返回的错误码
		return "", nil
	}
	return string(result), nil
}

// 将结构体转换为 map[string]interface{}
func structToMap(obj interface{}) map[string]interface{} {
	out := make(map[string]interface{})
	j, _ := json.Marshal(obj)
	json.Unmarshal(j, &out)
	return out
}

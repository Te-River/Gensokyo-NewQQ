package handlers

import (
	"encoding/json"
	"fmt"
	"regexp"
	"strconv"
	"time"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/openapi"
)

func init() {
	callapi.RegisterHandler("get_group_list", GetGroupList)
}

type Group struct {
	GroupCreateTime int32  `json:"group_create_time"`
	GroupID         int64  `json:"group_id"`
	GroupLevel      int32  `json:"group_level"`
	MaxMemberCount  int32  `json:"max_member_count"`
	MemberCount     int32  `json:"member_count"`
	GroupMemo       string `json:"group_memo"`
	GroupName       string `json:"group_name"`
}

type GroupString struct {
	GroupCreateTime int32  `json:"group_create_time"`
	GroupID         string `json:"group_id"`
	GroupLevel      int32  `json:"group_level"`
	MaxMemberCount  int32  `json:"max_member_count"`
	MemberCount     int32  `json:"member_count"`
	GroupMemo       string `json:"group_memo"`
	GroupName       string `json:"group_name"`
}

type GroupList struct {
	Data    []Group `json:"data"`
	Message string  `json:"message"`
	RetCode int     `json:"retcode"`
	Status  string  `json:"status"`
	Echo    interface{} `json:"echo"`
}

type GroupListString struct {
	Data    []GroupString `json:"data"`
	Message string        `json:"message"`
	RetCode int           `json:"retcode"`
	Status  string        `json:"status"`
	Echo    interface{}   `json:"echo"`
}

func GetGroupList(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	var groupList GroupList
	var groupListString GroupListString
	var outputMap map[string]interface{}

	// 初始化 groupList.Data 为一个空数组
	groupList.Data = []Group{}

	//从idmaps数据库找群,组合成群列表需要的格式
	groupIDs, err := idmap.FindKeysBySubAndType("group", "type")
	if err != nil {
		mylog.Printf("Error FindKeysBySubAndType %s", err)
	}
	// 当前时间的 10 位 Unix 时间戳
	currentTimestamp := int32(time.Now().Unix())

	// 判断是否string返回
	if !config.GetStringOb11() {
		for _, idStr := range groupIDs {
			groupID, err := strconv.ParseInt(idStr, 10, 64)
			if err != nil {
				mylog.Printf("Error converting group ID %s to int64: %v", idStr, err)
				continue
			}

			group := Group{
				GroupCreateTime: currentTimestamp, // 使用当前时间的时间戳
				GroupID:         groupID,
				GroupLevel:      0,
				GroupMemo:       "",
				GroupName:       "",
				MaxMemberCount:  0,
				MemberCount:     0,
			}

			groupList.Data = append(groupList.Data, group)
		}
		groupList.Message = ""
		groupList.RetCode = 0
		groupList.Status = "ok"

		if message.Echo == "" {
			groupList.Echo = "0"
		} else {
			groupList.Echo = message.Echo
		}
		outputMap = structToMap(groupList)
	} else {
		// 检查字符串是否仅包含数字 将数字形式的interactionID转换为真实的形式
		isNumeric := func(s string) bool {
			return regexp.MustCompile(`^\d+$`).MatchString(s)
		}
		for _, idStr := range groupIDs {
			var originalGroupID string
			if isNumeric(idStr) {
				continue
			} else {
				originalGroupID = idStr
			}
			group := GroupString{
				GroupCreateTime: currentTimestamp, // 使用当前时间的时间戳
				GroupID:         originalGroupID,
				GroupLevel:      0,
				GroupMemo:       "",
				GroupName:       "",
				MaxMemberCount:  0,
				MemberCount:     0,
			}
			groupListString.Data = append(groupListString.Data, group)
		}
		groupListString.Message = ""
		groupListString.RetCode = 0
		groupListString.Status = "ok"

		if message.Echo == "" {
			groupListString.Echo = "0"
		} else {
			groupListString.Echo = message.Echo
		}
		outputMap = structToMap(groupListString)
	}

	//fmt.Printf("getGroupList(频道): %+v\n", outputMap)
	fmt.Printf("getGroupList(数量): %+v\n", len(outputMap["data"].([]interface{})))

	err = client.SendMessage(outputMap)
	if err != nil {
		mylog.Printf("error sending group info via wsclient: %v", err)
	}

	var result []byte
	if !config.GetStringOb11() {
		result, err = json.Marshal(groupList)
		if err != nil {
			mylog.Printf("Error marshaling data: %v", err)
			return "", nil
		}
	} else {
		result, err = json.Marshal(groupListString)
		if err != nil {
			mylog.Printf("Error marshaling data: %v", err)
			return "", nil
		}
	}

	//mylog.Printf("get_group_list: %s", result)
	return string(result), nil
}

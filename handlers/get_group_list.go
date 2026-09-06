package handlers

import (
	"context"
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
	// 逐群节流间隔(毫秒),防超官方 QPM
	delay := config.GetGroupListDelay()

	// 判断是否string返回
	if !config.GetStringOb11() {
		for i, idStr := range groupIDs {
			groupID, err := strconv.ParseInt(idStr, 10, 64)
			if err != nil {
				mylog.Printf("Error converting group ID %s to int64: %v", idStr, err)
				continue
			}

			group := Group{
				GroupCreateTime: 0, // 官方 API 无群创建时间字段,诚实置 0
				GroupID:         groupID,
				GroupLevel:      0, // 官方无对应字段,保留 0
				GroupMemo:       "",
				GroupName:       "",
				MaxMemberCount:  0, // 官方无对应字段,保留 0
				MemberCount:     0,
			}
			// 逐群拉取真实信息,失败该群字段留空并日志,继续下一群
			group.GroupName, group.GroupMemo, group.MemberCount = fetchGroupBriefInfo(apiv2, idStr)

			groupList.Data = append(groupList.Data, group)
			// 逐群节流,最后一群后不等待
			if i < len(groupIDs)-1 && delay > 0 {
				time.Sleep(time.Duration(delay) * time.Millisecond)
			}
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
		count := 0
		for _, idStr := range groupIDs {
			var originalGroupID string
			if isNumeric(idStr) {
				continue
			} else {
				originalGroupID = idStr
			}
			group := GroupString{
				GroupCreateTime: 0, // 官方 API 无群创建时间字段,诚实置 0
				GroupID:         originalGroupID,
				GroupLevel:      0, // 官方无对应字段,保留 0
				GroupMemo:       "",
				GroupName:       "",
				MaxMemberCount:  0, // 官方无对应字段,保留 0
				MemberCount:     0,
			}
			// 逐群拉取真实信息,失败该群字段留空并日志,继续下一群
			group.GroupName, group.GroupMemo, group.MemberCount = fetchGroupBriefInfo(apiv2, idStr)

			groupListString.Data = append(groupListString.Data, group)
			// 逐群节流,最后一群后不等待
			count++
			if count < len(groupIDs) && delay > 0 {
				time.Sleep(time.Duration(delay) * time.Millisecond)
			}
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

// fetchGroupBriefInfo 拉取单个群的真实信息（群名/群简介/成员数）。
// API 失败返回空值由调用方留空处理，不中断其余群的拉取。
func fetchGroupBriefInfo(apiv2 openapi.OpenAPI, idStr string) (string, string, int32) {
	groupOpenID, err := resolveGroupOpenID(idStr)
	if err != nil {
		mylog.Printf("get_group_list: 反查 group_openid 失败 group=%s: %v", idStr, err)
		return "", "", 0
	}
	info, err := apiv2.GroupInfo(context.TODO(), groupOpenID)
	if err != nil {
		mylog.Printf("get_group_list: 获取群信息失败 group=%s: %v", idStr, err)
		return "", "", 0
	}
	return info.GroupName, info.GroupFingerMemo, int32(info.GroupMemberNum)
}

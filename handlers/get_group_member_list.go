package handlers

import (
	"strconv"
	"time"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/openapi"
)

type Response struct {
	Retcode int          `json:"retcode"`
	Status  string       `json:"status"`
	Data    []MemberList `json:"data"`
	Echo    interface{}  `json:"echo"` // 使用 interface{} 类型以容纳整数或文本
}

// Member Onebot 群成员
type MemberList struct {
	GroupID         uint64 `json:"group_id"`
	UserID          uint64 `json:"user_id"`
	Nickname        string `json:"nickname"`
	Card            string `json:"card"`
	Sex             string `json:"sex"`
	Age             int32  `json:"age"`
	Area            string `json:"area"`
	JoinTime        int32  `json:"join_time"`
	LastSentTime    int32  `json:"last_sent_time"`
	Level           string `json:"level"`
	Role            string `json:"role"`
	Unfriendly      bool   `json:"unfriendly"`
	Title           string `json:"title"`
	TitleExpireTime int64  `json:"title_expire_time"`
	CardChangeable  bool   `json:"card_changeable"`
	ShutUpTimestamp int64  `json:"shut_up_timestamp"`
}

func init() {
	callapi.RegisterHandler("get_group_member_list", GetGroupMemberList)
}

func GetGroupMemberList(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {

	msgType, err := idmap.ReadConfigv2(message.Params.GroupID.(string), "type")
	if err != nil {
		mylog.Printf("Error reading config: %v", err)
		return "", nil
	}

	switch msgType {
	case "group":
		mylog.Printf("getGroupMemberList(group): 开始从本地获取群成员列表(请在config打开idmap-pro以缓存群成员列表)")
		// 实现的功能
		var members []MemberList

		// 使用 message.Params.GroupID.(string) 作为 id 来调用 FindSubKeysById
		userIDs, err := idmap.FindSubKeysByIdPro(message.Params.GroupID.(string))
		if err != nil {
			mylog.Printf("Error retrieving user IDs: %v", err)
			return "", nil // 或者处理错误
		}

		// 获取当前时间的前一天，并转换为10位时间戳
		yesterday := time.Now().AddDate(0, 0, -1).Unix()

		for _, userID := range userIDs {
			userIDInt, err := strconv.ParseUint(userID, 10, 64)
			if err != nil {
				mylog.Printf("Error ParseInt73: %v", err)
			}
			groupIDInt, err := strconv.ParseUint(message.Params.GroupID.(string), 10, 64)
			if err != nil {
				mylog.Printf("Error ParseInt76: %v", err)
			}
			joinTimeInt := int32(yesterday)
			member := MemberList{
				UserID:          userIDInt,
				GroupID:         groupIDInt,
				Nickname:        "主人",
				Card:            "主人",
				Sex:             "0",
				Age:             0,
				Area:            "0",
				JoinTime:        joinTimeInt,
				LastSentTime:    0,
				Level:           "0",
				Role:            "member",
				Unfriendly:      false,
				Title:           "0",
				TitleExpireTime: 0,
				CardChangeable:  false,
				ShutUpTimestamp: 0,
			}
			members = append(members, member)
		}
		mylog.Printf("member message.Echors: %+v\n", message.Echo)

		responseJSON := buildResponse(members, message.Echo)
		mylog.Printf("getGroupMemberList(群): %s\n", responseJSON)

		err = client.SendMessage(responseJSON)
		if err != nil {
			mylog.Printf("Error sending message via client: %v", err)
		}
		result, err := ConvertMapToJSONString(responseJSON)
		if err != nil {
			mylog.Printf("Error marshaling data: %v", err)
			//todo 符合onebotv11 ws返回的错误码
			return "", nil
		}
		return string(result), nil
	case "private":
		mylog.Printf("getGroupMemberList(private): 目前暂未适配私聊虚拟群场景获取虚拟群列表能力")
		return "", nil
	default:
		mylog.Printf("Unknown msgType: %s", msgType)
	}
	return "", nil
}

func buildResponse(members []MemberList, echoValue interface{}) map[string]interface{} {
	data := make([]map[string]interface{}, len(members))

	for i, member := range members {
		memberMap := map[string]interface{}{
			"user_id":           member.UserID,
			"group_id":          member.GroupID,
			"nickname":          member.Nickname,
			"card":              member.Card,
			"sex":               member.Sex,
			"age":               member.Age,
			"area":              member.Area,
			"join_time":         member.JoinTime,
			"last_sent_time":    member.LastSentTime,
			"level":             member.Level,
			"role":              member.Role,
			"unfriendly":        member.Unfriendly,
			"title":             member.Title,
			"title_expire_time": member.TitleExpireTime,
			"card_changeable":   member.CardChangeable,
			"shut_up_timestamp": member.ShutUpTimestamp,
		}
		data[i] = memberMap
	}

	response := map[string]interface{}{
		"retcode": 0,
		"status":  "ok",
		"data":    data,
	}

	// Set echo based on the type of echoValue
	switch v := echoValue.(type) {
	case int:
		mylog.Printf("Setting echo as int: %d", v)
		response["echo"] = v
	case string:
		mylog.Printf("Setting echo as string: %s", v)
		response["echo"] = v
	case []interface{}:
		mylog.Printf("Setting echo as array: %v", v)
		response["echo"] = v
	case map[string]interface{}:
		mylog.Printf("Setting echo as object: %v", v)
		response["echo"] = v
	default:
		mylog.Printf("Unknown type for echo: %T", v)
	}

	return response
}

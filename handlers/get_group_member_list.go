package handlers

import (
	"context"
	"strconv"
	"time"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/config"
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

// groupMemberListMaxPages 分页安全上限(单页≤30,100页=3000人),防止异常响应导致死循环
const groupMemberListMaxPages = 100

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
		groupIDStr := message.Params.GroupID.(string)
		var members []MemberList

		// 反查真实群 OpenID，调用官方成员列表分页接口
		groupOpenID, err := resolveGroupOpenID(groupIDStr)
		if err != nil {
			mylog.Printf("get_group_member_list(group): 反查群 OpenID 失败,回退本地成员列表: %v", err)
			members = fetchFallbackMembers(groupIDStr)
		} else {
			members = fetchMembersByAPI(apiv2, groupOpenID, groupIDStr)
		}

		mylog.Printf("member message.Echors: %+v\n", message.Echo)

		responseJSON := buildResponse(members, message.Echo)
		mylog.Printf("getGroupMemberList(群): 共 %d 名成员\n", len(members))

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

// fetchMembersByAPI 通过官方 v2 成员列表接口游标分页拉取全量成员。
// 首次调用失败回退本地缓存路径；中途页失败返回已收集数据（不静默丢整表）。
func fetchMembersByAPI(apiv2 openapi.OpenAPI, groupOpenID, groupIDStr string) []MemberList {
	var members []MemberList
	groupIDInt, err := strconv.ParseUint(groupIDStr, 10, 64)
	if err != nil {
		// 群 OpenID 形态时无法回填虚拟群号,置 0 由应用端自行处理
		groupIDInt = 0
	}

	cursor := ""
	for page := 1; page <= groupMemberListMaxPages; page++ {
		list, err := apiv2.GroupMemberList(context.TODO(), groupOpenID, cursor, 30)
		if err != nil {
			if page == 1 {
				// 首页失败：官方接口不可用（如无权限/未开放），回退本地缓存路径
				mylog.Printf("get_group_member_list(group): 官方接口调用失败,回退本地成员列表: %v", err)
				return fetchFallbackMembers(groupIDStr)
			}
			// 中途页失败：返回已收集数据，避免整表丢失
			mylog.Printf("get_group_member_list(group): 第 %d 页拉取失败,返回已收集的 %d 名成员: %v", page, len(members), err)
			return members
		}

		for _, member := range list.Members {
			// 真实 openid → 虚拟 user_id（陌生成员自动生成新虚拟 ID）
			userID, err := idmap.StoreIDv2(member.MemberOpenID)
			if err != nil {
				mylog.Printf("get_group_member_list(group): 成员 openid 转虚拟 ID 失败,跳过: %v", err)
				continue
			}
			members = append(members, MemberList{
				UserID:   uint64(userID),
				GroupID:  groupIDInt,
				Nickname: member.Username,
				Card:     member.Username,
				Sex:      "unknown", // 官方无性别字段,中性值
				JoinTime: parseRFC3339ToInt32(member.JoinedAt, "joined_at"),
				Role:     normalizeMemberRole(member.MemberRole),
				// Age/Area/LastSentTime/Level/Title 等官方无对应字段,保留中性值
			})
		}

		if list.NextCursor == "" {
			break // 末页
		}
		// 页间节流,防止大群连续请求触发官方 QPM 限制(delay=0 时不休眠)
		if delay := config.GetGroupListDelay(); delay > 0 {
			time.Sleep(time.Duration(delay) * time.Millisecond)
		}
		cursor = list.NextCursor
	}
	if len(members) == 0 {
		// 官方返回空列表时回退本地缓存,保持旧行为可用
		mylog.Printf("get_group_member_list(group): 官方接口返回空列表,回退本地成员列表")
		return fetchFallbackMembers(groupIDStr)
	}
	return members
}

// fetchFallbackMembers 官方接口不可用时的本地缓存回退路径（原 FindSubKeysByIdPro 逻辑）。
// 基于 idmap-pro 缓存的成员虚拟 ID 列表构造成员条目。
func fetchFallbackMembers(groupIDStr string) []MemberList {
	var members []MemberList

	// 使用 groupID 作为 id 来调用 FindSubKeysByIdPro
	userIDs, err := idmap.FindSubKeysByIdPro(groupIDStr)
	if err != nil {
		mylog.Printf("Error retrieving user IDs: %v", err)
		return nil
	}

	// 获取当前时间的前一天，并转换为10位时间戳
	yesterday := time.Now().AddDate(0, 0, -1).Unix()

	for _, userID := range userIDs {
		userIDInt, err := strconv.ParseUint(userID, 10, 64)
		if err != nil {
			mylog.Printf("Error ParseInt73: %v", err)
		}
		groupIDInt, err := strconv.ParseUint(groupIDStr, 10, 64)
		if err != nil {
			mylog.Printf("Error ParseInt76: %v", err)
		}
		joinTimeInt := int32(yesterday)
		member := MemberList{
			UserID:          userIDInt,
			GroupID:         groupIDInt,
			Nickname:        "未知", // 本地缓存无用户名,中性占位
			Card:            "未知",
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
	return members
}

// parseRFC3339ToInt32 解析 RFC3339 时间字符串为 Unix 秒（int32）。
// 空串/非法格式置 0 并日志，不中断列表。
func parseRFC3339ToInt32(value, field string) int32 {
	if value == "" {
		return 0
	}
	t, err := time.Parse(time.RFC3339, value)
	if err != nil {
		// 中性前缀:本函数被 get_group_member_list 与 get_group_member_info 共用,不归属单一 action
		mylog.Printf("parseRFC3339ToInt32: %s 解析失败(%s),置 0: %v", field, value, err)
		return 0
	}
	return int32(t.Unix())
}

// normalizeMemberRole 官方 member_role 枚举(member|owner|admin)与 OneBot role 直通,未知值保底 member
func normalizeMemberRole(role string) string {
	switch role {
	case "owner", "admin", "member":
		return role
	}
	return "member"
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

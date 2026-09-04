package handlers

import (
	"context"
	"strconv"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/openapi"
)

// 初始化handler，在程序启动时会被调用
func init() {
	callapi.RegisterHandler("get_group_member_info", GetGroupMemberInfo)
}

// 成员信息的结构定义
type MemberInfo struct {
	UserID          int64  `json:"user_id"`
	GroupID         int64  `json:"group_id"`
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

// 构建单个成员的响应数据
func buildResponseForSingleMember(memberInfo *MemberInfo, echoValue interface{}) map[string]interface{} {
	// 构建成员数据的映射
	memberMap := map[string]interface{}{
		"group_id":          memberInfo.GroupID,
		"user_id":           memberInfo.UserID,
		"nickname":          memberInfo.Nickname,
		"card":              memberInfo.Card,
		"sex":               memberInfo.Sex,
		"age":               memberInfo.Age,
		"area":              memberInfo.Area,
		"join_time":         memberInfo.JoinTime,
		"last_sent_time":    memberInfo.LastSentTime,
		"level":             memberInfo.Level,
		"role":              memberInfo.Role,
		"unfriendly":        memberInfo.Unfriendly,
		"title":             memberInfo.Title,
		"title_expire_time": memberInfo.TitleExpireTime,
		"card_changeable":   memberInfo.CardChangeable,
		"shut_up_timestamp": memberInfo.ShutUpTimestamp,
	}

	// 构建完整的响应映射
	response := map[string]interface{}{
		"retcode": 0,
		"status":  "ok",
		"data":    memberMap,
		"echo":    echoValue,
	}

	return response
}

// GetGroupMemberInfo 是处理获取群成员信息的函数。
// 优先调用官方 v2 单成员接口取真实数据；接口失败时回退中性值兜底 body（不伪造成员信息）。
func GetGroupMemberInfo(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	groupID, _ := message.Params.GroupID.(string)
	userID, _ := message.Params.UserID.(string)

	memberInfo := fetchMemberInfoByAPI(apiv2, groupID, userID)

	// 构建响应JSON
	responseJSON := buildResponseForSingleMember(memberInfo, message.Echo)
	mylog.Printf("get_group_member_info: %s\n", responseJSON)

	// 发送响应回去
	err := client.SendMessage(responseJSON)
	if err != nil {
		mylog.Printf("发送消息时出错: %v", err)
	}
	result, err := ConvertMapToJSONString(responseJSON)
	if err != nil {
		mylog.Printf("Error marshaling data: %v", err)
		//todo 符合onebotv11 ws返回的错误码
		return "", nil
	}
	return string(result), nil
}

// fetchMemberInfoByAPI 调用官方单成员接口构造 MemberInfo，失败时返回中性值兜底。
func fetchMemberInfoByAPI(apiv2 openapi.OpenAPI, groupID, userID string) *MemberInfo {
	// 反查真实 OpenID（32 位原生 OpenID 直接使用）
	groupOpenID, err := resolveGroupOpenID(groupID)
	if err != nil {
		mylog.Printf("get_group_member_info: 反查 group_openid 失败,使用中性值兜底: %v", err)
		return buildFallbackMemberInfo(groupID, userID)
	}
	memberOpenID, err := resolveMemberOpenID(userID)
	if err != nil {
		mylog.Printf("get_group_member_info: 反查 member_openid 失败,使用中性值兜底: %v", err)
		return buildFallbackMemberInfo(groupID, userID)
	}

	member, err := apiv2.GroupMemberInfo(context.TODO(), groupOpenID, memberOpenID)
	if err != nil {
		mylog.Printf("get_group_member_info: 官方接口调用失败,使用中性值兜底: %v", err)
		return buildFallbackMemberInfo(groupID, userID)
	}

	// 虚拟 ID 输出：参数可解析则直接用，否则经 StoreIDv2 反查
	userIDInt, err := strconv.ParseInt(userID, 10, 64)
	if err != nil {
		userIDInt, _ = idmap.StoreIDv2(memberOpenID)
	}
	groupIDInt, err := strconv.ParseInt(groupID, 10, 64)
	if err != nil {
		groupIDInt, _ = idmap.StoreIDv2(groupOpenID)
	}

	return &MemberInfo{
		UserID:   userIDInt,
		GroupID:  groupIDInt,
		Nickname: member.Username,
		Card:     member.Username,
		Sex:      "unknown", // 官方无性别字段,中性值
		JoinTime: parseRFC3339ToInt32(member.JoinedAt, "joined_at"),
		Role:     normalizeMemberRole(member.MemberRole),
		// Age/Area/LastSentTime/Level/Title 等官方无对应字段,保留中性值
	}
}

// buildFallbackMemberInfo 官方接口失败时的中性值兜底 body（保留原 body 结构,不再伪造具体成员信息）。
func buildFallbackMemberInfo(groupID, userID string) *MemberInfo {
	userIDInt, err := strconv.ParseInt(userID, 10, 64)
	if err != nil {
		userIDInt = 0
	}
	groupIDInt, err := strconv.ParseInt(groupID, 10, 64)
	if err != nil {
		groupIDInt = 0
	}
	return &MemberInfo{
		UserID:  userIDInt,
		GroupID: groupIDInt,
		Sex:     "unknown",
		Role:    "member",
		Level:   "0",
	}
}

package handlers

import (
	"context"
	"encoding/json"
	"strconv"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/echo"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/mylog"

	"github.com/tencent-connect/botgo/dto"
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

func ConvertGuildToGroupInfo(guild *dto.Guild, GroupId string, message callapi.ActionMessage) *OnebotGroupInfo {
	// 使用idmap.StoreIDv2映射GroupId到一个int64的值
	groupid64, err := idmap.StoreIDv2(GroupId)
	if err != nil {
		mylog.Printf("Error storing GroupID: %v", err)
		return nil
	}

	ts, err := guild.JoinedAt.Time()
	if err != nil {
		mylog.Printf("转换JoinedAt失败: %v", err)
		return nil
	}
	groupCreateTime := int32(ts.Unix())

	groupInfo := &GroupInfo{
		GroupID:         groupid64,
		GroupName:       guild.Name,
		GroupMemo:       guild.Desc,
		GroupCreateTime: groupCreateTime,
		GroupLevel:      0,
		MemberCount:     int32(guild.MemberCount),
		MaxMemberCount:  int32(guild.MaxMembers),
	}

	// 创建 OnebotGroupInfo 实例并填充数据
	onebotGroupInfo := &OnebotGroupInfo{
		Data:    *groupInfo,
		Message: "success",
		RetCode: 0,
		Status:  "ok",
	}
	if message.Echo == "" {
		onebotGroupInfo.Echo = "0"
	} else {
		onebotGroupInfo.Echo = message.Echo
	}

	return onebotGroupInfo
}

func HandleGetGroupInfo(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	params := message.Params
	// 使用 message.Echo 作为key来获取消息类型
	var msgType string
	var groupInfo *OnebotGroupInfo
	var err error
	if echoStr, ok := message.Echo.(string); ok {
		// 当 message.Echo 是字符串类型时执行此块
		msgType = echo.GetMsgTypeByKey(echoStr)
	}
	if msgType == "" {
		msgType = GetMessageTypeByGroupid(config.GetAppIDStr(), message.Params.GroupID)
	}
	if msgType == "" {
		msgType = GetMessageTypeByGroupidV2(message.Params.GroupID)
	}
	switch msgType {
	case "guild", "guild_private":
		//用GroupID给ChannelID赋值,因为我们是把频道虚拟成了群
		ChannelID := params.GroupID
		// 使用RetrieveRowByIDv2还原真实的ChannelID
		mylog.Printf("测试:%v", ChannelID.(string))
		RChannelID, err := idmap.RetrieveRowByIDv2(ChannelID.(string))
		if err != nil {
			mylog.Printf("error retrieving real ChannelID: %v", err)
		}
		//读取ini 通过ChannelID取回之前储存的guild_id
		value, err := idmap.ReadConfigv2(RChannelID, "guild_id")
		if err != nil {
			mylog.Printf("handleGetGroupInfo:Error reading config: %v\n", err)
			return "", nil
		}
		//最后获取到guildID
		guildID := value
		mylog.Printf("调试,准备groupInfoMap(频道)guildID:%v", guildID)
		guild, err := api.Guild(context.TODO(), guildID)
		if err != nil {
			mylog.Printf("获取频道信息失败: %v", err)
			return "", nil
		}
		groupInfo = ConvertGuildToGroupInfo(guild, guildID, message)
	default:
		// 真实群聊：通过官方群信息 API 获取真实群信息
		groupID := message.Params.GroupID.(string)
		// 反查真实 OpenID（32 位原生 OpenID 直接使用）
		groupOpenID := groupID
		if len(groupID) != 32 {
			realGroupID, err := idmap.RetrieveRowByIDv2(groupID)
			if err != nil || realGroupID == "" {
				mylog.Printf("get_group_info: 反查 group_openid 失败: %v", err)
				return sendGroupInfoError(client, message, "无法反查群 OpenID")
			}
			groupOpenID = realGroupID
		}

		info, err := apiv2.GroupInfo(context.TODO(), groupOpenID)
		if err != nil {
			mylog.Printf("get_group_info: 获取群信息失败: %v", err)
			return sendGroupInfoError(client, message, err.Error())
		}

		groupid, _ := strconv.ParseInt(groupID, 10, 64)
		groupInfo = &OnebotGroupInfo{
			Data: GroupInfo{
				GroupID:     groupid,
				GroupName:   info.GroupName,
				GroupMemo:   info.GroupFingerMemo,
				MemberCount: int32(info.GroupMemberNum),
			},
			Message: "success",
			RetCode: 0,
			Status:  "ok",
		}
		if message.Echo == "" {
			groupInfo.Echo = "0"
		} else {
			groupInfo.Echo = message.Echo
		}
	}
	groupInfoMap := structToMap(groupInfo)

	// 打印groupInfoMap的内容
	mylog.Printf("groupInfoMap(频道): %+v\n", groupInfoMap)

	err = client.SendMessage(groupInfoMap) //发回去
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

// sendGroupInfoError 构造并回传错误响应
func sendGroupInfoError(client callapi.Client, message callapi.ActionMessage, errMsg string) (string, error) {
	response := struct {
		Message string      `json:"message"`
		RetCode int         `json:"retcode"`
		Status  string      `json:"status"`
		Echo    interface{} `json:"echo,omitempty"`
	}{
		Message: errMsg,
		RetCode: 100,
		Status:  "failed",
		Echo:    message.Echo,
	}
	outputMap := structToMap(response)
	if client != nil {
		if err := client.SendMessage(outputMap); err != nil {
			mylog.Printf("get_group_info: 发送错误响应失败: %v", err)
		}
	}
	result, err := json.Marshal(response)
	if err != nil {
		return "", err
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

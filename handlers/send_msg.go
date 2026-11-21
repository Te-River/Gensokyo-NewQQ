package handlers

import (
	"fmt"
	"strconv"
	"time"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/echo"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/internal/domain/identity"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/openapi"
)

func init() {
	callapi.RegisterHandler("send_msg", HandleSendMsg)
}

func HandleSendMsg(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	// 使用 message.Echo 作为key来获取消息类型
	var msgType string
	var retmsg string
	if echoStr, ok := message.Echo.(string); ok {
		// 当 message.Echo 是字符串类型时执行此块
		msgType = echo.GetMsgTypeByKey(echoStr)
	}
	// 检查GroupID是否为0
	checkZeroGroupID := func(id interface{}) bool {
		switch v := id.(type) {
		case int:
			return v != 0
		case int64:
			return v != 0
		case string:
			return v != "0" // 检查字符串形式的0
		default:
			return true // 如果不是int、int64或string，假定它不为0
		}
	}

	// 检查UserID是否为0
	checkZeroUserID := func(id interface{}) bool {
		switch v := id.(type) {
		case int:
			return v != 0
		case int64:
			return v != 0
		case string:
			return v != "0" // 同样检查字符串形式的0
		default:
			return true // 如果不是int、int64或string，假定它不为0
		}
	}

	// 内部直接构造的 action 可能不带 id（nil）或带非 string：统一安全取值，避免类型断言 panic
	groupIDStr := paramsID(message.Params.GroupID)
	userIDStr := paramsID(message.Params.UserID)

	if !identity.IsOpenID(groupIDStr) {
		if msgType == "" && message.Params.GroupID != nil && checkZeroGroupID(message.Params.GroupID) {
			msgType = GetMessageTypeByGroupid(config.GetAppIDStr(), message.Params.GroupID)
		}
		if msgType == "" && message.Params.UserID != nil && checkZeroUserID(message.Params.UserID) {
			msgType = GetMessageTypeByUserid(config.GetAppIDStr(), message.Params.UserID)
		}
		if msgType == "" && message.Params.GroupID != nil && checkZeroGroupID(message.Params.GroupID) {
			msgType = GetMessageTypeByGroupidV2(message.Params.GroupID)
		}
		if msgType == "" && message.Params.UserID != nil && checkZeroUserID(message.Params.UserID) {
			msgType = GetMessageTypeByUseridV2(message.Params.UserID)
		}
	}

	// New checks for UserID and GroupID being nil or 0
	if (message.Params.UserID == nil || !checkZeroUserID(message.Params.UserID)) &&
		(message.Params.GroupID == nil || !checkZeroGroupID(message.Params.GroupID)) {
		mylog.Printf("send_group_msgs接收到错误action: %v", message)
		return sendActionResult(client, message, "user_id 或 group_id 至少提供一个有效值", sendParamRetCode)
	}

	var idInt64 int64
	var err error

	if identity.IsOpenID(groupIDStr) {
		if groupIDStr != "" {
			idInt64, err = idmap.GenerateRowID(groupIDStr, 9)
		} else if userIDStr != "" {
			idInt64, err = idmap.GenerateRowID(userIDStr, 9)
		}
		// 临时的
		msgType = "group"
	} else {
		if groupIDStr != "" {
			idInt64, err = ConvertToInt64(groupIDStr)
		} else if userIDStr != "" {
			idInt64, err = ConvertToInt64(userIDStr)
		}
	}

	//设置递归 对直接向gsk发送action时有效果
	// 递归必须可推进：id 可转成正数、转换无错、且该 id 尚未被探测过（GetMapping 为 0），
	// 否则注入的 msgType 永远读不回，会无限自调用直到 fatal error: stack overflow。
	if msgType == "" {
		if err != nil {
			mylog.Printf("错误：无法转换 ID %v\n", err)
		} else if idInt64 > 0 && echo.GetMapping(idInt64) == 0 {
			// 递归1次（枚举剩余消息类型，当前仅 group）
			echo.AddMapping(idInt64, 2)
			// 递归调用handleSendMsg，使用设置的消息类型
			echo.AddMsgType(config.GetAppIDStr(), idInt64, "group_private")
			retmsg, _ = HandleSendMsg(client, api, apiv2, message)
		}
	} else if echo.GetMapping(idInt64) <= 0 {
		// 特殊值代表不递归
		echo.AddMapping(idInt64, 10)
	}

	// 递归一次后仍无法确定类型（注入的类型没被读回 / id 不可转数字）：回执 1400，绝不二次递归。
	if msgType == "" {
		mylog.Printf("send_msg无法确定消息类型: %v", message)
		return sendActionResult(client, message, "无法确定消息类型: group_id/user_id 无效", sendParamRetCode)
	}

	switch msgType {
	case "group":
		//复用处理逻辑
		retmsg, _ = HandleSendGroupMsg(client, api, apiv2, message)
	case "group_private":
		//私聊信息
		retmsg, _ = HandleSendPrivateMsg(client, api, apiv2, message)
	default:
		mylog.Printf("1Unknown message type: %s", msgType)
	}

	// 如果递归id不是10(不递归特殊值)
	if echo.GetMapping(idInt64) != 10 {
		//重置递归类型
		if echo.GetMapping(idInt64) <= 0 {
			echo.AddMsgType(config.GetAppIDStr(), idInt64, "")
		}
		echo.AddMapping(idInt64, echo.GetMapping(idInt64)-1)

		//递归1次枚举类型（当前仅 group）
		if echo.GetMapping(idInt64) > 0 {
			tryMessageTypes := []string{"group"}
			messageCopy := message // 创建message的副本
			echo.AddMsgType(config.GetAppIDStr(), idInt64, tryMessageTypes[echo.GetMapping(idInt64)-1])
			delay := config.GetSendDelay()
			time.Sleep(time.Duration(delay) * time.Millisecond)
			retmsg, _ = HandleSendMsg(client, api, apiv2, messageCopy)
		}
	}

	return retmsg, nil
}

// 通过user_id获取messageID
func GetMessageIDByUseridOrGroupid(appID string, userID interface{}) string {
	// 从appID和userID生成key
	var userIDStr string
	switch u := userID.(type) {
	case int:
		userIDStr = strconv.Itoa(u)
	case int64:
		userIDStr = strconv.FormatInt(u, 10)
	case float64:
		userIDStr = strconv.FormatFloat(u, 'f', 0, 64)
	case string:
		userIDStr = u
	default:
		// 可能需要处理其他类型或报错
		return ""
	}
	//将真实id转为int
	userid64, err := idmap.StoreIDv2(userIDStr)
	if err != nil {
		mylog.Printf("Error storing ID 241: %v", err)
		return ""
	}
	key := appID + "_" + fmt.Sprint(userid64)
	mylog.Printf("GetMessageIDByUseridOrGroupid_key:%v", key)
	messageid := echo.GetMsgIDByKey(key)
	if messageid == "" {
		key := appID + "_" + userIDStr
		mylog.Printf("GetMessageIDByUseridOrGroupid_key_2:%v", key)
		messageid = echo.GetMsgIDByKey(key)
	}
	return messageid
}

// 通过user_id获取messageID
func GetMessageIDByUseridOrGroupidSP(appID string, userID interface{}) string {
	// 从appID和userID生成key
	var userIDStr string
	switch u := userID.(type) {
	case int:
		userIDStr = strconv.Itoa(u)
	case int64:
		userIDStr = strconv.FormatInt(u, 10)
	case float64:
		userIDStr = strconv.FormatFloat(u, 'f', 0, 64)
	case string:
		userIDStr = u
	default:
		// 可能需要处理其他类型或报错
		return ""
	}

	messageid := echo.GetMsgIDv3(appID, userIDStr)
	return messageid
}

// 通过user_id获取EventID 私聊,群,频道,通用 userID可以是三者之一 这是不需要区分群+用户的 只需要精准到群 私聊只需要精准到用户 idmap不开启的用户使用
func GetEventIDByUseridOrGroupid(appID string, userID interface{}) string {
	// 从appID和userID生成key
	var userIDStr string
	switch u := userID.(type) {
	case int:
		userIDStr = strconv.Itoa(u)
	case int64:
		userIDStr = strconv.FormatInt(u, 10)
	case float64:
		userIDStr = strconv.FormatFloat(u, 'f', 0, 64)
	case string:
		userIDStr = u
	default:
		// 可能需要处理其他类型或报错
		return ""
	}
	//将真实id转为int 这是非idmap-pro的方式
	userid64, err := idmap.StoreIDv2(userIDStr)
	if err != nil {
		mylog.Printf("Error storing ID 241: %v", err)
		return ""
	}
	key := appID + "_" + fmt.Sprint(userid64)
	mylog.Printf("GetEventIDByUseridOrGroupid_key:%v", key)
	eventid := echo.GetEventIDByKey(key)
	if eventid == "" {
		// 用原始id获取,这个分支应该是没有用的.
		key := appID + "_" + userIDStr
		mylog.Printf("GetEventIDByUseridOrGroupid_key_2:%v", key)
		eventid = echo.GetEventIDByKey(key)
	}
	return eventid
}

// 通过user_id获取EventID 私聊,群,频道,通用 userID可以是三者之一 这是不需要区分群+用户的 只需要精准到群 私聊只需要精准到用户 idmap不开启的用户使用
func GetEventIDByUseridOrGroupidv2(appID string, userID interface{}) string {
	// 从appID和userID生成key
	var userIDStr string
	switch u := userID.(type) {
	case int:
		userIDStr = strconv.Itoa(u)
	case int64:
		userIDStr = strconv.FormatInt(u, 10)
	case float64:
		userIDStr = strconv.FormatFloat(u, 'f', 0, 64)
	case string:
		userIDStr = u
	default:
		// 可能需要处理其他类型或报错
		return ""
	}

	key := appID + "_" + userIDStr
	mylog.Printf("GetEventIDByUseridOrGroupid_key:%v", key)
	eventid := echo.GetEventIDByKey(key)

	return eventid
}

// 通过user_id获取messageID
func GetMessageIDByUseridAndGroupid(appID string, userID interface{}, groupID interface{}) string {
	// 从appID和userID生成key
	var userIDStr string
	switch u := userID.(type) {
	case int:
		userIDStr = strconv.Itoa(u)
	case int64:
		userIDStr = strconv.FormatInt(u, 10)
	case float64:
		userIDStr = strconv.FormatFloat(u, 'f', 0, 64)
	case string:
		userIDStr = u
	default:
		// 可能需要处理其他类型或报错
		return ""
	}
	// 从appID和userID生成key
	var GroupIDStr string
	switch u := groupID.(type) {
	case int:
		GroupIDStr = strconv.Itoa(u)
	case int64:
		GroupIDStr = strconv.FormatInt(u, 10)
	case float64:
		GroupIDStr = strconv.FormatFloat(u, 'f', 0, 64)
	case string:
		GroupIDStr = u
	default:
		// 可能需要处理其他类型或报错
		return ""
	}
	var userid64, groupid64 int64
	var err error
	if config.GetIdmapPro() {
		//将真实id转为int userid64
		groupid64, userid64, err = idmap.StoreIDv2Pro(GroupIDStr, userIDStr)
		if err != nil {
			mylog.Errorf("Error storing ID 210: %v", err)
		}
	} else {
		//将真实id转为int
		userid64, err = idmap.StoreIDv2(userIDStr)
		if err != nil {
			mylog.Errorf("Error storing ID 241: %v", err)
			return ""
		}
		//将真实id转为int
		groupid64, err = idmap.StoreIDv2(GroupIDStr)
		if err != nil {
			mylog.Errorf("Error storing ID 256: %v", err)
			return ""
		}
	}
	key := appID + "_" + fmt.Sprint(groupid64) + "_" + fmt.Sprint(userid64)
	mylog.Printf("GetMessageIDByUseridAndGroupid_key:%v", key)
	return echo.GetMsgIDByKey(key)
}

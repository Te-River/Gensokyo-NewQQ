package handlers

import (
	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/mylog"
)

// checkZeroGroupID 检查 GroupID 是否有效（不为 0 或空）
func checkZeroGroupID(id interface{}) bool {
	switch v := id.(type) {
	case int:
		return v != 0
	case int64:
		return v != 0
	case string:
		return v != "0"
	default:
		return true
	}
}

// checkZeroUserID 检查 UserID 是否有效（不为 0 或空）
func checkZeroUserID(id interface{}) bool {
	switch v := id.(type) {
	case int:
		return v != 0
	case int64:
		return v != 0
	case string:
		return v != "0"
	default:
		return true
	}
}

// GetMessageTypeByUseridAndGroupid 组合用户 ID 和群 ID 获取消息类型
// 返回第一个非空的消息类型，如果都为空则返回 ""
func GetMessageTypeByUseridAndGroupid(userid, groupid interface{}) string {
	if userid != nil && checkZeroUserID(userid) {
		if mt := GetMessageTypeByUserid(config.GetAppIDStr(), userid); mt != "" {
			return mt
		}
		if mt := GetMessageTypeByUseridV2(userid); mt != "" {
			return mt
		}
	}
	if groupid != nil && checkZeroGroupID(groupid) {
		if mt := GetMessageTypeByGroupid(config.GetAppIDStr(), groupid); mt != "" {
			return mt
		}
		if mt := GetMessageTypeByGroupidV2(groupid); mt != "" {
			return mt
		}
	}
	return ""
}

// ValidateGroupOrUserIDs 验证 GroupID 和 UserID 至少有一个有效
// 如果都无效，返回错误日志并返回 false
func ValidateGroupOrUserIDs(groupid, userid interface{}) bool {
	if (userid == nil || !checkZeroUserID(userid)) &&
	   (groupid == nil || !checkZeroGroupID(groupid)) {
		mylog.Printf("send_group_msgs接收到错误action: GroupID and UserID are both invalid")
		return false
	}
	return true
}
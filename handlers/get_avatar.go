package handlers

import (
	"fmt"
	"strconv"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/internal/domain/identity"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/openapi"
)

type AvatarData struct {
	Message string `json:"message"`
	UserID  int64  `json:"user_id"`
}

type GetAvatarResponse struct {
	Data    AvatarData  `json:"data"`
	Message string      `json:"message"`
	RetCode int         `json:"retcode"`
	Status  string      `json:"status"`
	Echo    interface{} `json:"echo"`
}

func init() {
	callapi.RegisterHandler("get_avatar", GetAvatar)
}

func GetAvatar(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {

	var response GetAvatarResponse
	var originalUserID string
	var err error

	// 内部直接构造的 action 可能不带 id（nil）或带非 string：统一安全取值，避免类型断言 panic
	groupIDStr := paramsID(message.Params.GroupID)
	userIDStr := paramsID(message.Params.UserID)

	if identity.IsOpenID(userIDStr) {
		// 32 位原生 openid 直通，不反查（与 get_user_info / get_group_info 同口径）
		originalUserID = userIDStr
	} else if config.GetIdmapPro() {
		// 如果user_id可用且配置为使用Pro版本，则调用RetrieveRowByIDv2Pro
		_, originalUserID, err = idmap.RetrieveRowByIDv2Pro(groupIDStr, userIDStr)
		if err != nil {
			mylog.Printf("Error1 retrieving original GroupID: %v", err)
			_, originalUserID, err = idmap.RetrieveRowByIDv2Pro("690426430", userIDStr)
			if err != nil {
				mylog.Printf("Error reading private originalUserID: %v", err)
			}
		}
	} else {
		originalUserID, err = idmap.RetrieveRowByIDv2(userIDStr)
		if err != nil {
			mylog.Printf("Error retrieving original UserID: %v", err)
		}
	}

	avatarurl, _ := GenerateAvatarURLV2(originalUserID)
	if err != nil || originalUserID == "" {
		mylog.Printf("get_avatar: 无法反查 user_openid: %v", err)
		return sendActionResult(client, message, "无法反查用户 OpenID", sendParamRetCode)
	}

	userid64, _ := strconv.ParseInt(userIDStr, 10, 64)

	response.Data = AvatarData{
		Message: avatarurl,
		UserID:  userid64,
	}
	response.Message = ""
	response.RetCode = 0
	response.Status = "ok"
	response.Echo = message.Echo

	outputMap := structToMap(response)

	mylog.Printf("get_avatar: %+v\n", outputMap)

	err = client.SendMessage(outputMap)
	if err != nil {
		mylog.Printf("Error sending message via client: %v", err)
	}
	//把结果从struct转换为json
	result, err := callapi.MarshalNoEscape(response)
	if err != nil {
		mylog.Printf("Error marshaling data: %v", err)
		// 已回执（上方 SendMessage）
		return "", nil
	}
	return string(result), nil
}

// GenerateAvatarURLV2 生成根据32位ID 和 Appid 组合的 新QQ 头像 URL
func GenerateAvatarURLV2(openid string) (string, error) {

	appidstr := config.GetAppIDStr()
	// 构建并返回 URL
	return fmt.Sprintf("https://q.qlogo.cn/qqapp/%s/%s/640", appidstr, openid), nil
}

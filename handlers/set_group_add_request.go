package handlers

import (
	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/openapi"
)

func init() {
	callapi.RegisterHandler("set_group_add_request", SetGroupAddRequest)
}

// SetGroupAddRequest 审批入群申请
// params: group_id(虚拟/真实群ID), user_id(虚拟/真实用户ID), flag(join_request_id), approve(bool)
// 扩展参数: reason(拒绝理由), add_to_member_blacklist(是否同时拉黑)
// 内部复用 set_group_helpers.go 共享底层，与 [CQ:set_group,action=add_request] 行为一致
func SetGroupAddRequest(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	groupID, _ := message.Params.GroupID.(string)
	userID, _ := message.Params.UserID.(string)

	// 反查真实 OpenID（32 位原生 OpenID 直接使用）
	groupOpenID, err := resolveGroupOpenID(groupID)
	if err != nil {
		mylog.Printf("set_group_add_request: 反查 group_openid 失败: %v", err)
		return sendActionResult(client, message, "无法反查群 OpenID", 100)
	}
	memberOpenID, err := resolveMemberOpenID(userID)
	if err != nil {
		mylog.Printf("set_group_add_request: 反查 member_openid 失败: %v", err)
		return sendActionResult(client, message, "无法反查用户 OpenID", 100)
	}

	var blacklist *bool
	if message.Params.AddToMemberBlacklist {
		b := true
		blacklist = &b
	}
	if err := approveJoinRequest(apiv2, groupOpenID, memberOpenID, message.Params.Flag, message.Params.Approve, message.Params.Reason, blacklist); err != nil {
		mylog.Printf("set_group_add_request: 审批失败: %v", err)
		return sendActionResult(client, message, err.Error(), 100)
	}

	op := "decline"
	if message.Params.Approve {
		op = "approve"
	}
	mylog.Printf("set_group_add_request: 审批成功 group=%s user=%s op=%s", groupOpenID, memberOpenID, op)
	return sendActionResult(client, message, "", 0)
}

// 回执码口径（与 Rust 侧对齐）：1400 参数/协议错误（含 id 反查失败），1500 上游/内部失败。
// 上游失败复用 message_parser.go 的 sendFailRetCode；历史 handler 仍沿用旧口径 100，新增分支请用这两个常量。
const sendParamRetCode = 1400

// paramsID 安全读取 Params 里的 id 字段（user_id / group_id / message_id）：nil 与非 string 一律视为空串。
//
// JSON 入参路径本来就是安全的——ParamsContent.UnmarshalJSON 会把缺失字段归一为 ""（非 nil），
// 所以解码出来的 id 一定是 string。但内部代码（CQ 解析、事件转 action、webhook 转发）会直接
// 构造 callapi.ActionMessage，此时这些字段可能仍是 nil 或其他类型；旧写法
// message.Params.UserID.(string) 在这种输入上直接 panic，RecoverPanic 只兜住不至于崩溃，
// 却不回执，客户端只能等到超时。发送/撤回类入口统一走这里取 id。
func paramsID(v interface{}) string {
	s, ok := v.(string)
	if !ok {
		return ""
	}
	return s
}

// sendActionResult 构造并回传 OneBot action 结果响应（retcode 0=成功；失败时 data 序列化为 null）
func sendActionResult(client callapi.Client, message callapi.ActionMessage, errMsg string, retCode int) (string, error) {
	status := "ok"
	if retCode != 0 {
		status = "failed"
	}
	response := struct {
		Data    interface{} `json:"data"`
		Message string      `json:"message"`
		RetCode int         `json:"retcode"`
		Status  string      `json:"status"`
		Echo    interface{} `json:"echo"`
	}{
		Data:    nil,
		Message: errMsg,
		RetCode: retCode,
		Status:  status,
		Echo:    message.Echo,
	}
	outputMap := structToMap(response)
	if client != nil {
		if err := client.SendMessage(outputMap); err != nil {
			mylog.Printf("sendActionResult: 发送响应失败: %v", err)
		}
	}
	result, err := callapi.MarshalNoEscape(response)
	if err != nil {
		mylog.Printf("sendActionResult: 序列化响应失败: %v", err)
		return "", err
	}
	return string(result), nil
}

// sendActionResultWithData 构造并回传带 data 的成功响应
func sendActionResultWithData(client callapi.Client, message callapi.ActionMessage, data interface{}) (string, error) {
	response := struct {
		Data    interface{} `json:"data"`
		Message string      `json:"message"`
		RetCode int         `json:"retcode"`
		Status  string      `json:"status"`
		Echo    interface{} `json:"echo,omitempty"`
	}{
		Data:    data,
		Message: "",
		RetCode: 0,
		Status:  "ok",
		Echo:    message.Echo,
	}
	outputMap := structToMap(response)
	if client != nil {
		if err := client.SendMessage(outputMap); err != nil {
			mylog.Printf("set_group_add_request: 发送响应失败: %v", err)
		}
	}
	result, err := callapi.MarshalNoEscape(response)
	if err != nil {
		mylog.Printf("set_group_add_request: 序列化响应失败: %v", err)
		return "", err
	}
	return string(result), nil
}

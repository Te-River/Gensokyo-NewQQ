package handlers

import (
	"encoding/base64"
	"strconv"
	"time"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/handlers/cqparse"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/openapi"
)

func init() {
	callapi.RegisterHandler("send_group_forward_msg", HandleSendGroupForwardMsg)
}

func HandleSendGroupForwardMsg(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {
	nodes, ok := message.Params.Messages.([]interface{})
	if !ok {
		mylog.Printf("send_group_forward_msg: Messages 不是 []interface{} 类型")
		return sendActionResult(client, message, "messages 参数必须是消息段数组", sendParamRetCode)
	}
	var retmsg string
	forwardMsgLimit := config.GetForwardMsgLimit() // 获取消息发送条数上限
	count := 0

	for _, nodeInterface := range nodes {
		if count > forwardMsgLimit {
			break // 如果消息数量已达上限，则停止发送
		}

		nodeMap, ok := nodeInterface.(map[string]interface{})
		if !ok {
			continue
		}

		nodeData, ok := nodeMap["data"].(map[string]interface{})
		if !ok {
			continue
		}

		var messageText string
		var foundItems map[string][]string

		// 检查 content 的类型
		content, ok := nodeData["content"].([]interface{})
		if ok {
			// 处理 segment 类型的 content
			var intercepted []cqparse.PendingAction
			messageText, foundItems, intercepted = parseMessageContent(callapi.ParamsContent{Message: content}, message, client, api, apiv2)
			// 修 M1：转发节点显式拦截动作码（Parse 内已按 Scope 拦截，此处兜底丢弃）
			if len(intercepted) > 0 {
				mylog.Printf("[cqparse] 转发路径拦截 %d 个动作码,不执行不发送", len(intercepted))
			}
		} else {
			// 处理直接包含的文本内容
			contentString, ok := nodeData["content"].(string)
			if ok {
				messageText = contentString
			}
		}

		// 节点署名：官方群消息出站请求体无作者字段，合并转发也无官方出站接口
		// （botgo 仅有 /v2/groups/{id}/messages 单条发送），本 handler 一直是逐条降级下发，
		// uin/name 只能并入正文署名，否则整段丢弃。
		if author := forwardNodeAuthor(nodeData); author != "" {
			messageText = author + ": " + messageText
		}

		// 节点内 [CQ:keyboard]：逐条下发只带正文会丢掉键盘，这里还原为等价的 base64 CQ 码
		// 随正文重放，由 send_group_msg 的既有解析（parseMDData → ResolveKeyboardVirtualIDs）
		// 附加到该节点，不另做一套解析。
		if kbItems, ok := foundItems["keyboard"]; ok && len(kbItems) > 0 {
			messageText += "[CQ:keyboard,data=base64://" + base64.StdEncoding.EncodeToString([]byte(kbItems[0])) + "]"
		}

		// 创建新的 ActionMessage 实例并发送
		newMessage := callapi.ActionMessage{
			Action: "send_group_msg",
			Params: callapi.ParamsContent{
				GroupID: message.Params.GroupID,
				Message: messageText,
			},
		}

		HandleSendGroupMsg(client, api, apiv2, newMessage)
		count++
		time.Sleep(500 * time.Millisecond) // 每条消息之间的延时
	}
	retmsg, _ = SendResponse(client, nil, &message, nil, api, apiv2)
	return retmsg, nil
}

// forwardNodeAuthor 取节点署名：go-cqhttp/TRSS 用 uin/name，OneBot v11 标准用 user_id/nickname。
// 两者都给时以「昵称(uin)」呈现，保证两个字段都不丢失。
func forwardNodeAuthor(nodeData map[string]interface{}) string {
	uin := nodeDataString(nodeData, "uin", "user_id")
	name := nodeDataString(nodeData, "name", "nickname")
	switch {
	case name != "" && uin != "":
		return name + "(" + uin + ")"
	case name != "":
		return name
	}
	return uin
}

// nodeDataString 按顺序返回第一个非空字段；JSON 数值形态（float64）统一转十进制字符串。
func nodeDataString(nodeData map[string]interface{}, keys ...string) string {
	for _, k := range keys {
		switch v := nodeData[k].(type) {
		case string:
			if v != "" {
				return v
			}
		case float64:
			return strconv.FormatFloat(v, 'f', -1, 64)
		case int:
			return strconv.Itoa(v)
		}
	}
	return ""
}

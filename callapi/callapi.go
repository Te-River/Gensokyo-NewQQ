package callapi

import (
	"encoding/json"
	"fmt"
	"strconv"

	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/openapi"
)

// onebot发来的action调用信息
type ActionMessage struct {
	Action      string        `json:"action"`
	Params      ParamsContent `json:"params"`
	Echo        interface{}   `json:"echo,omitempty"`
	PostType    string        `json:"post_type,omitempty"`
	MessageType string        `json:"message_type,omitempty"`
}

func (a *ActionMessage) UnmarshalJSON(data []byte) error {
	type Alias ActionMessage

	var rawEcho json.RawMessage
	temp := &struct {
		*Alias
		Echo *json.RawMessage `json:"echo,omitempty"`
	}{
		Alias: (*Alias)(a),
		Echo:  &rawEcho,
	}

	if err := json.Unmarshal(data, &temp); err != nil {
		return err
	}

	if rawEcho != nil {
		var lastErr error

		var intValue int
		if lastErr = json.Unmarshal(rawEcho, &intValue); lastErr == nil {
			a.Echo = intValue
			return nil
		}

		var strValue string
		if lastErr = json.Unmarshal(rawEcho, &strValue); lastErr == nil {
			a.Echo = strValue
			return nil
		}

		var arrValue []interface{}
		if lastErr = json.Unmarshal(rawEcho, &arrValue); lastErr == nil {
			a.Echo = arrValue
			return nil
		}

		var objValue map[string]interface{}
		if lastErr = json.Unmarshal(rawEcho, &objValue); lastErr == nil {
			a.Echo = objValue
			return nil
		}

		return fmt.Errorf("unable to unmarshal echo: %v", lastErr)
	}

	return nil
}

// params类型
type ParamsContent struct {
	BotQQ     string      `json:"botqq,omitempty"`
	ChannelID interface{} `json:"channel_id,omitempty"`
	GuildID   interface{} `json:"guild_id,omitempty"`
	GroupID   interface{} `json:"group_id,omitempty"`   // 每一种onebotv11实现的字段类型都可能不同
	MessageID interface{} `json:"message_id,omitempty"` // 用于撤回信息
	Message   interface{} `json:"message,omitempty"`    // 这里使用interface{}因为它可能是多种类型
	Messages  interface{} `json:"messages,omitempty"`   // 坑爹转发信息
	UserID    interface{} `json:"user_id,omitempty"`    // 这里使用interface{}因为它可能是多种类型
	Duration  int         `json:"duration,omitempty"`   // 可选的整数
	Enable    bool        `json:"enable,omitempty"`     // 可选的布尔值
	// set_group_add_request 入群申请审批
	Approve              bool   `json:"approve,omitempty"`                 // 是否同意入群申请
	Flag                 string `json:"flag,omitempty"`                    // 申请标识(join_request_id)
	Reason               string `json:"reason,omitempty"`                  // 拒绝理由(扩展参数)
	AddToMemberBlacklist bool   `json:"add_to_member_blacklist,omitempty"` // 是否同时拉黑(扩展参数)
	// 群聊管理扩展 action 参数
	NextIndex      int      `json:"next_index,omitempty"`      // 入群申请列表分页游标
	Cursor         string   `json:"cursor,omitempty"`          // 策略列表分页游标
	Limit          int      `json:"limit,omitempty"`           // 策略列表单页数量
	StrategyID     string   `json:"strategy_id,omitempty"`     // 策略 ID
	GroupOpenIDs   StringList `json:"group_openids,omitempty"`   // 关联群 openid 列表
	GroupIDs       []uint64 `json:"group_ids,omitempty"`       // 关联 QQ 群号列表
	IsEnable       string   `json:"is_enable,omitempty"`       // 策略启用状态 on/off
	ExpireAt       string   `json:"expire_at,omitempty"`       // 策略过期时间(RFC3339)
	Remark         string   `json:"remark,omitempty"`          // 策略备注
	Op             string   `json:"op,omitempty"`              // 白名单/关联群操作 add/del
	WhitelistUsers []string `json:"whitelist_users,omitempty"` // 白名单 QQ 号码列表
	// 菜单/面板与成员管理扩展 action 参数
	Scope        string      `json:"scope,omitempty"`         // 面板 scope (c2c/group/channel/dm)
	TargetType   string      `json:"target_type,omitempty"`   // 面板对象范围 all|specific(仅 c2c/group 支持 specific)
	PanelID      string      `json:"panel_id,omitempty"`      // 面板 ID
	UserIDs      StringList  `json:"user_ids,omitempty"`      // 批量踢人/拉黑成员列表(虚拟 ID 数组,≤20)
	UserOpenIDs  StringList  `json:"user_openids,omitempty"`  // 面板创建/关联对象列表(虚拟 ID 数组)
	AddBlacklist bool        `json:"add_blacklist,omitempty"` // kick 时移出同时拉黑
	Menu         interface{} `json:"menu,omitempty"`          // 菜单原始对象(handler 内 Marshal→Unmarshal 成 dto.Menu)
	Panel        interface{} `json:"panel,omitempty"`         // 面板原始对象(同上,透传官方校验)
	// handle quick operation
	Context      Context   `json:"context,omitempty"`       // context 字段
	Operation    Operation `json:"operation,omitempty"`     // operation 字段
	CallbackData string    `json:"callback_data,omitempty"` // 新增: 用于接收 GenerateURLLink 的参数
}

// StringList 柔性字符串数组:OneBot 客户端对 user_id 类字段习惯发数字数组(JSON 数值),
// 固定 []string 会在整请求 Unmarshal 阶段报错、消息被 wsclient 静默丢弃(客户端收不到任何回执)。
// 元素级归一化:字符串直通、数字转十进制字符串、其他类型跳过。
type StringList []string

// UnmarshalJSON 兼容 ["123"] / [123] / 混合 / null 四种输入
func (s *StringList) UnmarshalJSON(b []byte) error {
	var raw []interface{}
	if err := json.Unmarshal(b, &raw); err != nil {
		return err
	}
	for _, v := range raw {
		switch x := v.(type) {
		case string:
			*s = append(*s, x)
		case float64:
			*s = append(*s, strconv.FormatFloat(x, 'f', -1, 64))
		}
	}
	return nil
}

// Context 结构体用于存储 context 字段相关信息
type Context struct {
	Avatar      string `json:"avatar,omitempty"`       // 用户头像链接
	Font        int    `json:"font,omitempty"`         // 字体（假设是整数类型）
	MessageID   int    `json:"message_id,omitempty"`   // 消息 ID
	MessageSeq  int    `json:"message_seq,omitempty"`  // 消息序列号
	MessageType string `json:"message_type,omitempty"` // 消息类型
	PostType    string `json:"post_type,omitempty"`    // 帖子类型
	SubType     string `json:"sub_type,omitempty"`     // 子类型
	Time        int64  `json:"time,omitempty"`         // 时间戳
	UserID      int    `json:"user_id,omitempty"`      // 用户 ID
	GroupID     int    `json:"group_id,omitempty"`     // 群号
}

// Operation 结构体用于存储 operation 字段相关信息
type Operation struct {
	Reply    string `json:"reply,omitempty"`     // 回复内容
	AtSender bool   `json:"at_sender,omitempty"` // 是否 @ 发送者
}

// 自定义一个ParamsContent的UnmarshalJSON 让GroupID同时兼容str和int
func (p *ParamsContent) UnmarshalJSON(data []byte) error {
	type Alias ParamsContent
	aux := &struct {
		GroupID   interface{} `json:"group_id"`
		UserID    interface{} `json:"user_id"`
		MessageID interface{} `json:"message_id"`
		ChannelID interface{} `json:"channel_id"`
		GuildID   interface{} `json:"guild_id"`
		*Alias
	}{
		Alias: (*Alias)(p),
	}
	if err := json.Unmarshal(data, &aux); err != nil {
		return err
	}

	switch v := aux.GroupID.(type) {
	case nil: // 当GroupID不存在时
		p.GroupID = ""
	case float64: // JSON的数字默认被解码为float64
		p.GroupID = fmt.Sprintf("%.0f", v) // 将其转换为字符串，忽略小数点后的部分
	case string:
		p.GroupID = v
	default:
		return fmt.Errorf("GroupID has unsupported type")
	}

	switch v := aux.UserID.(type) {
	case nil: // 当UserID不存在时
		p.UserID = ""
	case float64: // JSON的数字默认被解码为float64
		p.UserID = fmt.Sprintf("%.0f", v) // 将其转换为字符串，忽略小数点后的部分
	case string:
		p.UserID = v
	default:
		return fmt.Errorf("UserID has unsupported type")
	}

	switch v := aux.MessageID.(type) {
	case nil: // 当UserID不存在时
		p.MessageID = ""
	case float64: // JSON的数字默认被解码为float64
		p.MessageID = fmt.Sprintf("%.0f", v) // 将其转换为字符串，忽略小数点后的部分
	case string:
		p.MessageID = v
	default:
		return fmt.Errorf("MessageID has unsupported type")
	}

	switch v := aux.ChannelID.(type) {
	case nil: // 当ChannelID不存在时
		p.ChannelID = ""
	case float64: // JSON的数字默认被解码为float64
		p.ChannelID = fmt.Sprintf("%.0f", v) // 将其转换为字符串，忽略小数点后的部分
	case string:
		p.ChannelID = v
	default:
		return fmt.Errorf("MessageID has unsupported type")
	}

	switch v := aux.GuildID.(type) {
	case nil: // 当GuildID不存在时
		p.GuildID = ""
	case float64: // JSON的数字默认被解码为float64
		p.GuildID = fmt.Sprintf("%.0f", v) // 将其转换为字符串，忽略小数点后的部分
	case string:
		p.GuildID = v
	default:
		return fmt.Errorf("MessageID has unsupported type")
	}

	return nil
}

// Message represents a standardized structure for the incoming messages.
type Message struct {
	Action string                 `json:"action"`
	Params map[string]interface{} `json:"params"`
	Echo   interface{}            `json:"echo,omitempty"`
}

// 这是一个接口,在wsclient传入client但不需要引用wsclient包,避免循环引用,复用wsserver和client逻辑
type Client interface {
	SendMessage(message map[string]interface{}) error
}

// 为了解决processor和server循环依赖设计的接口
type WebSocketServerClienter interface {
	SendMessage(message map[string]interface{}) error
	Close() error
}

// 根据action订阅handler处理api
type HandlerFunc func(client Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, messgae ActionMessage) (string, error)

var handlers = make(map[string]HandlerFunc)

// RegisterHandler registers a new handler for a specific action.
func RegisterHandler(action string, handler HandlerFunc) {
	handlers[action] = handler
}

// CallAPIFromDict 处理信息 by calling the 对应的 handler.
func CallAPIFromDict(client Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message ActionMessage) string {
	handler, ok := handlers[message.Action]
	if !ok {
		mylog.Println("Unsupported action:", message.Action)
		return ""
	}

	jsonString, err := handler(client, api, apiv2, message)
	if err != nil {
		// 处理错误
		mylog.Println("Error handling action:", message.Action, "Error:", err)
		return ""
	}

	return jsonString
}

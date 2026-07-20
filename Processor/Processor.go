// 处理收到的信息事件
package Processor

import (
	"fmt"
	"reflect"
	"strings"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/structs"
	"github.com/hoshinonyaruko/gensokyo/wsclient"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

// Processor 结构体用于处理消息
type Processors struct {
	Api             openapi.OpenAPI                   // API 类型
	Apiv2           openapi.OpenAPI                   //群的API
	Settings        *structs.Settings                 // 使用指针
	Wsclient        []*wsclient.WebSocketClient       // 指针的切片
	WsServerClients []callapi.WebSocketServerClienter //ws server被连接的客户端
}

type Sender struct {
	Nickname string `json:"nickname"`
	TinyID   string `json:"tiny_id"`
	UserID   int64  `json:"user_id"`
	Role     string `json:"role,omitempty"`
	Card     string `json:"card,omitempty"`
	Sex      string `json:"sex,omitempty"`
	Age      int32  `json:"age,omitempty"`
	Area     string `json:"area,omitempty"`
	Level    string `json:"level,omitempty"`
	Title    string `json:"title,omitempty"`
}

// 频道信息事件
type OnebotChannelMessage struct {
	ChannelID       string      `json:"channel_id"`
	GuildID         string      `json:"guild_id"`
	Message         interface{} `json:"message"`
	MessageID       string      `json:"message_id"`
	MessageType     string      `json:"message_type"`
	PostType        string      `json:"post_type"`
	SelfID          int64       `json:"self_id"`
	SelfTinyID      string      `json:"self_tiny_id"`
	Sender          Sender      `json:"sender"`
	SubType         string      `json:"sub_type"`
	Time            int64       `json:"time"`
	Avatar          string      `json:"avatar,omitempty"`
	UserID          int64       `json:"user_id"`
	RawMessage      string      `json:"raw_message"`
	Echo            string      `json:"echo,omitempty"`
	RealMessageType string      `json:"real_message_type,omitempty"` //当前信息的真实类型 表情表态
}

// 群信息事件
type OnebotGroupMessage struct {
	RawMessage      string      `json:"raw_message"`
	MessageID       int         `json:"message_id"`
	GroupID         int64       `json:"group_id"` // Can be either string or int depending on p.Settings.CompleteFields
	MessageType     string      `json:"message_type"`
	PostType        string      `json:"post_type"`
	SelfID          int64       `json:"self_id"` // Can be either string or int
	Sender          Sender      `json:"sender"`
	SubType         string      `json:"sub_type"`
	Time            int64       `json:"time"`
	Avatar          string      `json:"avatar,omitempty"`
	Echo            string      `json:"echo,omitempty"`
	Message         interface{} `json:"message"` // For array format
	MessageSeq      int         `json:"message_seq"`
	Font            int         `json:"font"`
	UserID          int64       `json:"user_id"`
	ToMe            bool        `json:"to_me,omitempty"`              //消息是否@了机器人
	RealMessageType string      `json:"real_message_type,omitempty"`  //当前信息的真实类型 group group_private guild guild_private
	RealUserID      string      `json:"real_user_id,omitempty"`       //当前真实uid
	RealGroupID     string      `json:"real_group_id,omitempty"`      //当前真实gid
	IsBindedGroupId bool        `json:"is_binded_group_id,omitempty"` //当前群号是否是binded后的
	IsBindedUserId  bool        `json:"is_binded_user_id,omitempty"`  //当前用户号号是否是binded后的
	IsFullGroupMessage bool    `json:"is_full_group_message,omitempty"` //消息是否来自全量（非@）群聊
	Platform        string      `json:"platform,omitempty"`            //平台类型
}

type OnebotGroupMessageS struct {
	RawMessage      string      `json:"raw_message"`
	MessageID       string      `json:"message_id"`
	GroupID         string      `json:"group_id"` // Can be either string or int depending on p.Settings.CompleteFields
	MessageType     string      `json:"message_type"`
	PostType        string      `json:"post_type"`
	SelfID          int64       `json:"self_id"` // Can be either string or int
	Sender          Sender      `json:"sender"`
	SubType         string      `json:"sub_type"`
	Time            int64       `json:"time"`
	Avatar          string      `json:"avatar,omitempty"`
	Echo            string      `json:"echo,omitempty"`
	Message         interface{} `json:"message"` // For array format
	MessageSeq      int         `json:"message_seq"`
	Font            int         `json:"font"`
	UserID          string      `json:"user_id"`
	ToMe            bool        `json:"to_me,omitempty"`              //消息是否@了机器人
	RealMessageType string      `json:"real_message_type,omitempty"`  //当前信息的真实类型 group group_private guild guild_private
	RealUserID      string      `json:"real_user_id,omitempty"`       //当前真实uid
	RealGroupID     string      `json:"real_group_id,omitempty"`      //当前真实gid
	IsBindedGroupId bool        `json:"is_binded_group_id,omitempty"` //当前群号是否是binded后的
	IsBindedUserId  bool        `json:"is_binded_user_id,omitempty"`  //当前用户号号是否是binded后的
	IsFullGroupMessage bool    `json:"is_full_group_message,omitempty"` //消息是否来自全量（非@）群聊
	Platform        string      `json:"platform,omitempty"`            //平台类型
}

// 私聊信息事件
type OnebotPrivateMessage struct {
	RawMessage      string        `json:"raw_message"`
	MessageID       int           `json:"message_id"` // Can be either string or int depending on logic
	MessageType     string        `json:"message_type"`
	PostType        string        `json:"post_type"`
	SelfID          int64         `json:"self_id"` // Can be either string or int depending on logic
	Sender          PrivateSender `json:"sender"`
	SubType         string        `json:"sub_type"`
	Time            int64         `json:"time"`
	Avatar          string        `json:"avatar,omitempty"`
	Echo            string        `json:"echo,omitempty"`
	Message         interface{}   `json:"message"`                     // For array format
	MessageSeq      int           `json:"message_seq"`                 // Optional field
	Font            int           `json:"font"`                        // Optional field
	UserID          int64         `json:"user_id"`                     // Can be either string or int depending on logic
	RealMessageType string        `json:"real_message_type,omitempty"` //当前信息的真实类型 group group_private guild guild_private
	RealUserID      string        `json:"real_user_id,omitempty"`      //当前真实uid
	IsBindedUserId  bool          `json:"is_binded_user_id,omitempty"` //当前用户号号是否是binded后的
}

// onebotv11标准扩展
type OnebotInteractionNotice struct {
	GroupID     int64                  `json:"group_id,omitempty"`
	NoticeType  string                 `json:"notice_type,omitempty"`
	PostType    string                 `json:"post_type,omitempty"`
	SelfID      int64                  `json:"self_id,omitempty"`
	SubType     string                 `json:"sub_type,omitempty"`
	Time        int64                  `json:"time,omitempty"`
	UserID      int64                  `json:"user_id,omitempty"`
	Data        *dto.WSInteractionData `json:"data,omitempty"`
	RealUserID  string                 `json:"real_user_id,omitempty"`  //当前真实uid
	RealGroupID string                 `json:"real_group_id,omitempty"` //当前真实gid
}

// onebotv11标准扩展
type OnebotGroupRejectNotice struct {
	GroupID    int64                    `json:"group_id,omitempty"`
	NoticeType string                   `json:"notice_type,omitempty"`
	PostType   string                   `json:"post_type,omitempty"`
	SelfID     int64                    `json:"self_id,omitempty"`
	SubType    string                   `json:"sub_type,omitempty"`
	Time       int64                    `json:"time,omitempty"`
	UserID     int64                    `json:"user_id,omitempty"`
	Data       *dto.GroupMsgRejectEvent `json:"data,omitempty"`
}

// onebotv11标准扩展
type OnebotGroupReceiveNotice struct {
	GroupID    int64                     `json:"group_id,omitempty"`
	NoticeType string                    `json:"notice_type,omitempty"`
	PostType   string                    `json:"post_type,omitempty"`
	SelfID     int64                     `json:"self_id,omitempty"`
	SubType    string                    `json:"sub_type,omitempty"`
	Time       int64                     `json:"time,omitempty"`
	UserID     int64                     `json:"user_id,omitempty"`
	Data       *dto.GroupMsgReceiveEvent `json:"data,omitempty"`
}

type PrivateSender struct {
	Nickname string `json:"nickname"`
	UserID   int64  `json:"user_id"`
	TinyID   string `json:"tiny_id"`
}

func PrintStructWithFieldNames(v interface{}) {
	t := reflect.TypeOf(v)
	if t.Kind() == reflect.Ptr {
		t = t.Elem()
	}
	fmt.Printf("Type: %s\n", t.Name())
	val := reflect.ValueOf(v)
	if val.Kind() == reflect.Ptr {
		val = val.Elem()
	}
	for i := 0; i < t.NumField(); i++ {
		field := t.Field(i)
		value := val.Field(i)
		fmt.Printf("%s: %v\n", field.Name, value.Interface())
	}
}

func structToMap(obj interface{}) map[string]interface{} {
	result := make(map[string]interface{})
	val := reflect.ValueOf(obj)
	if val.Kind() == reflect.Ptr {
		val = val.Elem()
	}
	typ := val.Type()
	for i := 0; i < val.NumField(); i++ {
		field := typ.Field(i)
		fieldValue := val.Field(i)
		jsonTag := field.Tag.Get("json")
		if jsonTag != "" {
			jsonName := strings.SplitN(jsonTag, ",", 2)[0]
			if jsonName != "" {
				result[jsonName] = fieldValue.Interface()
			}
		}
	}
	return result
}

func applyOpUserIDType(message map[string]interface{}) {
	opUserIDType := config.GetOpUserIDType()
	sender, ok := message["sender"].(map[string]interface{})
	if !ok {
		return
	}
	switch opUserIDType {
	case "raw":
		if userID, exists := message["user_id"]; exists {
			sender["user_id"] = userID
		}
	case "ruin":
		if realUserID, exists := message["real_user_id"]; exists {
			sender["user_id"] = realUserID
		}
	case "vuin":
		if userID, exists := message["user_id"]; exists {
			sender["user_id"] = userID
		}
	}
}

// 修改函数的返回类型为 *Processor
func NewProcessor(api openapi.OpenAPI, apiv2 openapi.OpenAPI, settings *structs.Settings, wsclient []*wsclient.WebSocketClient) *Processors {
	return &Processors{
		Api:      api,
		Apiv2:    apiv2,
		Settings: settings,
		Wsclient: wsclient,
	}
}

func NewProcessorV2(api openapi.OpenAPI, apiv2 openapi.OpenAPI, settings *structs.Settings) *Processors {
	return &Processors{
		Api:      api,
		Apiv2:    apiv2,
		Settings: settings,
	}
}
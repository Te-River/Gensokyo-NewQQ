package dto

import "github.com/tencent-connect/botgo/dto/keyboard"

// SendType 消息类型
type SendType int

const (
	Text      SendType = 1 // 文字消息
	RichMedia SendType = 2 // 富媒体类消息
)

// APIMessage 消息结构接口
type APIMessage interface {
	GetEventID() string
	GetSendType() SendType
}

// RichMediaMessage 富媒体消息
type RichMediaMessage struct {
	EventID    string `json:"event_id,omitempty"`  // 要回复的事件id, 逻辑同MsgID
	FileType   uint64 `json:"file_type,omitempty"` // 业务类型，图片，文件，语音，视频 文件类型，取值:1图片,2视频,3语音,4文件
	URL        string `json:"url,omitempty"`
	FileData   string `json:"file_data,omitempty"` //没有base64头的base64
	SrvSendMsg bool   `json:"srv_send_msg,omitempty"`
	Content    string `json:"content,omitempty"`
	FileName   string `json:"file_name,omitempty"` // 文件名（仅文件类型有效）
}

// Meida message内的富媒体消息
type Media struct {
	FileInfo string `json:"file_info"`
}

// GetEventID 事件ID
func (msg RichMediaMessage) GetEventID() string {
	return msg.EventID
}

// GetSendType 消息类型
func (msg RichMediaMessage) GetSendType() SendType {
	return RichMedia
}

// MessageToCreate 发送消息结构体定义
type MessageToCreate struct {
	Content string `json:"content,omitempty"`
	MsgType int    `json:"msg_type,omitempty"` //消息类型: 0:文字消息, 2: md消息
	Embed   *Embed `json:"embed,omitempty"`
	Ark     *Ark   `json:"ark,omitempty"`
	Image   string `json:"image,omitempty"`
	Media   *Media `json:"media,omitempty"`
	// 要回复的消息id，为空是主动消息，公域机器人会异步审核，不为空是被动消息，公域机器人会校验语料
	MsgID            string                    `json:"msg_id,omitempty"`
	MessageReference *MessageReference         `json:"message_reference,omitempty"`
	Markdown         *Markdown                 `json:"markdown,omitempty"`
	Keyboard         *keyboard.MessageKeyboard `json:"keyboard,omitempty"`  // 消息按钮组件
	EventID          string                    `json:"event_id,omitempty"`  // 要回复的事件id, 逻辑同MsgID
	Timestamp        int64                     `json:"timestamp,omitempty"` //TODO delete this
	MsgSeq           int                       `json:"msg_seq,omitempty"`   //回复消息的序号，与 msg_id 联合使用，避免相同消息id回复重复发送，不填默认是1。相同的 msg_id + msg_seq 重复发送会失败。

	// [新增] 互动召回消息标记
	// 指明发送消息为互动召回消息，与 msg_id，event_id 互斥使用
	IsWakeup bool `json:"is_wakeup,omitempty"`

	// Card 卡片消息（msg_type=8），群聊专用
	// type="tuwen" 时含 title/description/pic_url/url
	Card *GroupCard `json:"card,omitempty"`

	// InputNotify 输入状态通知（msg_type=6），仅单聊可用
	InputNotify *InputNotify `json:"input_notify,omitempty"`
}

// FourmToCreate 发送帖子结构体定义
type FourmToCreate struct {
	Title   string `json:"title,omitempty"`
	Content string `json:"content,omitempty"`
	Format  uint32 `json:"format,omitempty"` //消息类型: 1:文字消息 2:html信息 3: md消息 4:json信息
}

// GetEventID 事件ID
func (msg MessageToCreate) GetEventID() string {
	return msg.EventID
}

// GetSendType 消息类型
func (msg MessageToCreate) GetSendType() SendType {
	return Text
}

// MessageReference 引用消息
type MessageReference struct {
	MessageID             string `json:"message_id"`               // 消息 id
	IgnoreGetMessageError bool   `json:"ignore_get_message_error"` // 是否忽律获取消息失败错误
}

// GetEventID 事件ID
func (msg MessageReference) GetEventID() string {
	return msg.MessageID
}

// GetSendType 消息类型
func (msg MessageReference) GetSendType() SendType {
	return Text
}

// Markdown markdown 消息
type Markdown struct {
	TemplateID       int               `json:"template_id,omitempty"`        // 模版 id
	CustomTemplateID string            `json:"custom_template_id,omitempty"` // 模版 id 群
	Params           []*MarkdownParams `json:"params,omitempty"`             // 模版参数
	Content          string            `json:"content,omitempty"`            // 原生 markdown
	// ForceVerifyImageResource 官方强校验图片转存结果,由 config 开关控制注入(默认 false 不序列化)
	ForceVerifyImageResource bool `json:"force_verify_image_resource,omitempty"`
}

// MarkdownParams markdown 模版参数 键值对
type MarkdownParams struct {
	Key    string   `json:"key"`
	Values []string `json:"values"`
}

// SettingGuideToCreate 发送引导消息的结构体
type SettingGuideToCreate struct {
	Content      string        `json:"content,omitempty"`       // 频道内发引导消息可以带@
	SettingGuide *SettingGuide `json:"setting_guide,omitempty"` // 设置引导
}

// SettingGuide 设置引导
type SettingGuide struct {
	// 频道ID, 当通过私信发送设置引导消息时，需要指定guild_id
	GuildID string `json:"guild_id"`
}

// GroupCard 群聊卡片消息（msg_type=8）
// type="tuwen" 时发送图文卡片，含标题、描述、图片和跳转链接
type GroupCard struct {
	Type    string         `json:"type"`
	Content *GroupCardContent `json:"content,omitempty"`
}

// GroupCardContent 卡片消息正文
type GroupCardContent struct {
	Title       string `json:"title,omitempty"`
	Description string `json:"description,omitempty"`
	PicURL      string `json:"pic_url,omitempty"`
	URL         string `json:"url,omitempty"`
}

// InputNotify 输入状态通知（msg_type=6），仅单聊可用
type InputNotify struct {
	InputType   int `json:"input_type"`
	InputSecond int `json:"input_second"`
}

// ---------- 文件分片上传结构体 ----------

// UploadPrepareRequest 分片上传预上传请求
type UploadPrepareRequest struct {
	FileType int    `json:"file_type,omitempty"`
	FileSize string `json:"file_size,omitempty"`
	FileName string `json:"file_name,omitempty"`
	MD5      string `json:"md5,omitempty"`
	SHA1     string `json:"sha1,omitempty"`
	MD510m   string `json:"md5_10m,omitempty"`
}

// UploadPrepareResponse 分片上传预上传响应
type UploadPrepareResponse struct {
	UploadID    string            `json:"upload_id"`
	BlockSize   string            `json:"block_size"`
	Parts       []UploadPart      `json:"parts"`
	UploadCfg   *UploadConfig     `json:"upload_config,omitempty"`
}

// UploadPart 分片信息
type UploadPart struct {
	Index        int    `json:"index"`
	PresignedURL string `json:"presigned_url"`
	BlockSize    string `json:"block_size"`
}

// UploadConfig 上传配置
type UploadConfig struct {
	Concurrency   int `json:"concurrency"`
	RetryTimeout  int `json:"retry_timeout"`
	RetryDelay    int `json:"retry_delay"`
}

// UploadPartFinishRequest 分片上传完成请求
type UploadPartFinishRequest struct {
	UploadID   string `json:"upload_id,omitempty"`
	PartIndex  int    `json:"part_index,omitempty"`
	BlockSize  string `json:"block_size,omitempty"`
	MD5        string `json:"md5,omitempty"`
}

// FileUploadRequest 文件上传合并请求（用于分片上传最后一步：合并）
type FileUploadRequest struct {
	FileType    int    `json:"file_type,omitempty"`
	FileName    string `json:"file_name,omitempty"`
	SrvSendMsg  bool   `json:"srv_send_msg,omitempty"`
	UploadID    string `json:"upload_id,omitempty"`
}

// StreamChunk 流式消息分片
type StreamChunk struct {
	InputMode    string `json:"input_mode,omitempty"`
	InputState   int    `json:"input_state"`
	Index        int    `json:"index"`
	ContentType  string `json:"content_type,omitempty"`
	ContentRaw   string `json:"content_raw,omitempty"`
	StreamMsgID  string `json:"stream_msg_id,omitempty"`
	MsgID        string `json:"msg_id,omitempty"`
	EventID      string `json:"event_id,omitempty"`
	MsgSeq       int    `json:"msg_seq,omitempty"`
	IsWakeup     bool   `json:"is_wakeup,omitempty"`
}

// 仅供测试

type MessageSSE struct {
	MsgType        int              `json:"msg_type,omitempty"`
	Markdown       *MarkdownSSE     `json:"markdown,omitempty"`
	MsgID          string           `json:"msg_id,omitempty"`
	MsgSeq         int              `json:"msg_seq,omitempty"`
	Stream         *StreamSSE       `json:"stream,omitempty"`
	PromptKeyboard *KeyboardSSE     `json:"prompt_keyboard,omitempty"`
	ActionButton   *ActionButtonSSE `json:"action_button,omitempty"`
}

// GetEventID 事件ID
func (msg MessageSSE) GetEventID() string {
	return ""
}

// GetSendType 消息类型
func (msg MessageSSE) GetSendType() SendType {
	return 1
}

type MarkdownSSE struct {
	Content string `json:"content"`
}

type StreamSSE struct {
	State int    `json:"state"`
	Index int    `json:"index"`
	ID    string `json:"id,omitempty"`
}

type KeyboardSSE struct {
	KeyboardContentSSE `json:"keyboard"`
}

type KeyboardContentSSE struct {
	Content ContentSSE `json:"content"`
}

type ContentSSE struct {
	Rows []RowSSE `json:"rows"`
}

type RowSSE struct {
	Buttons []ButtonSSE `json:"buttons"`
}

type ButtonSSE struct {
	RenderData RenderDataSSE `json:"render_data"`
	Action     ActionSSE     `json:"action"`
}

type RenderDataSSE struct {
	Label string `json:"label"`
	Style int    `json:"style"`
}

type ActionSSE struct {
	Type int `json:"type"`
}

type ActionButtonSSE struct {
	TemplateID   int    `json:"template_id"`
	CallbackData string `json:"callback_data"`
}

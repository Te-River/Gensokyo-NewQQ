package openapi

import (
	"context"
	"time"

	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/token"
)

// RetractMessageOption 撤回消息可选参数
type RetractMessageOption int

const (
	// RetractMessageOptionHidetip 撤回消息隐藏小灰条可选参数
	RetractMessageOptionHidetip RetractMessageOption = 1
)

// OpenAPI openapi 完整实现
type OpenAPI interface {
	Base
	WebsocketAPI
	UserAPI
	MessageAPI
	WebhookAPI
	InteractionAPI
}

// Base 基础能力接口
type Base interface {
	Version() APIVersion
	Setup(token *token.Token, inSandbox bool) OpenAPI
	// WithTimeout 设置请求接口超时时间
	WithTimeout(duration time.Duration) OpenAPI
	// Transport 透传请求，如果 sdk 没有及时跟进新的接口的变更，可以使用该方法进行透传，openapi 实现时可以按需选择是否实现该接口
	Transport(ctx context.Context, method, url string, body interface{}) ([]byte, error)
	// TraceID 返回上一次请求的 trace id
	TraceID() string
}

// WebsocketAPI websocket 接入地址
type WebsocketAPI interface {
	WS(ctx context.Context, params map[string]string, body string) (*dto.WebsocketAP, error)
}

// UserAPI 用户相关接口
type UserAPI interface {
	Me(ctx context.Context) (*dto.User, error)
}

// MessageAPI 消息相关接口
type MessageAPI interface {
	RetractGroupMessage(ctx context.Context, groupID, msgID string, options ...RetractMessageOption) error
	RetractC2CMessage(ctx context.Context, userID, msgID string, options ...RetractMessageOption) error

	// PostGroupMessage 发送群消息
	PostGroupMessage(ctx context.Context, groupID string, msg dto.APIMessage) (*dto.GroupMessageResponse, error)
	// PostC2CMessage 发送C2C消息
	PostC2CMessage(ctx context.Context, userID string, msg dto.APIMessage) (*dto.C2CMessageResponse, error)
	// PostC2CMessage 发送C2CSSE消息
	PostC2CMessageSSE(ctx context.Context, userID string, msg dto.APIMessage) (*dto.C2CMessageResponse, error)

	// PostC2CStreamMessage 发送C2C流式消息（stream_messages）
	PostC2CStreamMessage(ctx context.Context, userID string, chunk *dto.StreamChunk) (*dto.C2CMessageResponse, error)

	// 如果你有 UserAPI interface，加在这里；如果没有，加在你汇总的 API interface 里
	// GenerateURLLink 获取机器人资料页分享链接
	GenerateURLLink(ctx context.Context, params *dto.GenerateURLLinkToCreate) (*dto.GenerateURLLink, error)

	// FileUploadPrepare 单聊/群聊文件分片上传 — 预上传
	FileUploadPrepare(ctx context.Context, id string, isGroup bool, req *dto.UploadPrepareRequest) (*dto.UploadPrepareResponse, error)
	// FileUploadPartFinish 单聊/群聊文件分片上传 — 通知分片完成
	FileUploadPartFinish(ctx context.Context, id string, isGroup bool, req *dto.UploadPartFinishRequest) error
	// FileUploadMerge 单聊/群聊文件分片上传 — 合并分片（返回 file_info）
	FileUploadMerge(ctx context.Context, id string, isGroup bool, req *dto.FileUploadRequest) (*dto.MediaResponse, error)
}


// InteractionAPI 互动接口
type InteractionAPI interface {
	// PutInteraction 更新互动信息
	PutInteraction(ctx context.Context, interactionID string, body string) error
}

// WebhookAPI http 事件网关相关接口
type WebhookAPI interface {
	CreateSession(ctx context.Context, identity dto.HTTPIdentity) (*dto.HTTPReady, error)
	CheckSessions(ctx context.Context) ([]*dto.HTTPSession, error)
	SessionList(ctx context.Context) ([]*dto.HTTPSession, error)
	RemoveSession(ctx context.Context, sessionID string) error
}

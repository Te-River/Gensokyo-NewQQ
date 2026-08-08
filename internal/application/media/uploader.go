package media

import (
	"context"
)

// UploadedMedia 上传结果。
type UploadedMedia struct {
	// URL 公开可访问地址。
	URL string
}

// MediaUploader 图床/COS/OSS 统一上传接口。
// Handler 不应知道具体云 SDK，只依赖本接口。
type MediaUploader interface {
	Upload(ctx context.Context, media *PreparedMedia) (UploadedMedia, error)
}

// AudioTranscoder 音频转码边界（FFmpeg / go-silk 封装在此 Adapter 内）。
type AudioTranscoder interface {
	// Transcode 把输入转为 QQ 兼容格式；返回新 PreparedMedia（使用后 Close）。
	Transcode(ctx context.Context, media *PreparedMedia) (*PreparedMedia, error)
}

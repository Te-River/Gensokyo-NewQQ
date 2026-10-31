package dto

// 素材管理（media-images）DTO
// 官方文档：/server-inter/message/material-management.html（页脚 2026-10-21）
// 链路：prepare（申请预签名）→ PUT presigned_url（COS，不带 Authorization）→ complete（换 image_id/url）
// markdown 中引用形式：![说明 #宽px #高px](qqbot-media://image/v1/<token>)

// MediaImagePrepareRequest 申请上传
type MediaImagePrepareRequest struct {
	FileName    string `json:"file_name"`    // 文件名（含扩展名，如 kline.png）
	FileSize    uint64 `json:"file_size"`    // 文件大小（字节）
	ContentType string `json:"content_type"` // image/png | image/jpeg | image/gif | image/webp
	MD5         string `json:"md5"`          // 内容 MD5，32 位十六进制
	SHA1        string `json:"sha1"`         // 内容 SHA1，40 位十六进制
}

// MediaImagePrepareResponse 申请上传的响应
type MediaImagePrepareResponse struct {
	UploadID     string `json:"upload_id"`     // 上传任务 ID，4 小时有效
	PresignedURL string `json:"presigned_url"` // 直传 COS 的预签名 URL
	ExpiresIn    uint32 `json:"expires_in"`    // 预签名 URL 有效期（秒）
}

// MediaImageCompleteRequest 完成上传
type MediaImageCompleteRequest struct {
	UploadID string `json:"upload_id"`
	MD5      string `json:"md5"` // PUT 响应头 ETag 去引号后的值
}

// MediaImageCompleteResponse 完成上传的响应
type MediaImageCompleteResponse struct {
	ImageID    string `json:"image_id"`
	URL        string `json:"url"`         // qqbot-media://image/v1/<token>
	TTL        uint32 `json:"ttl"`         // 滑动有效期（秒），官方 7776000 = 90 天
	PreviewURL string `json:"preview_url"` // COS 预签名短链，禁止持久化
	ExpiresAt  int64  `json:"expires_at,omitempty"`
}

// MediaImage 素材图片（列表项）
type MediaImage struct {
	ImageID    string `json:"image_id"`
	URL        string `json:"url"`
	FileName   string `json:"file_name"`
	PreviewURL string `json:"preview_url"`
	CreatedAt  int64  `json:"created_at"` // Unix 秒
	ExpiresAt  int64  `json:"expires_at"` // Unix 秒，每次被消息引用后顺延
}

// MediaImageList 素材图片列表
type MediaImageList struct {
	Images     []*MediaImage `json:"images"`
	NextCursor string        `json:"next_cursor"`
	Total      int           `json:"total"` // 与单机器人 100 张上限同口径
}

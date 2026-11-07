package dto

import (
	"bytes"
	"encoding/json"
	"fmt"
	"strconv"
)

// 素材管理（media-images）DTO
// 官方文档：/server-inter/message/material-management.html（页脚 2026-10-21）
// 链路：prepare（申请预签名）→ PUT presigned_url（COS，不带 Authorization）→ complete（换 image_id/url）
// markdown 中引用形式：![说明 #宽px #高px](qqbot-media://image/v1/<token>)

// flexInt64 数值字段的宽容类型：官方列表接口实发会把 created_at/expires_at/total 等数值
// 以 JSON 字符串下发（issue #56：total 为串导致整个响应 unmarshal 失败 → 配额清理永不
// 重试 → 白白降级外链），这里同时接受 number 与十进制数字字符串，null 与缺省保持零值；
// 其余非法形态（对象/布尔/含非数字的串）仍返回错误。
// MarshalJSON 仍输出 number，与普通 int64 的序列化语义一致。
type flexInt64 int64

// UnmarshalJSON 接受 number、数字字符串与 null 三种形态
func (f *flexInt64) UnmarshalJSON(data []byte) error {
	trimmed := bytes.TrimSpace(data)
	if len(trimmed) == 0 || bytes.Equal(trimmed, []byte("null")) {
		return nil
	}
	if trimmed[0] == '"' {
		var s string
		if err := json.Unmarshal(trimmed, &s); err != nil {
			return err
		}
		n, err := strconv.ParseInt(s, 10, 64)
		if err != nil {
			return fmt.Errorf("flexInt64: 无法解析 %q 为 int64: %w", s, err)
		}
		*f = flexInt64(n)
		return nil
	}
	var n int64
	if err := json.Unmarshal(trimmed, &n); err != nil {
		return err
	}
	*f = flexInt64(n)
	return nil
}

// MarshalJSON 输出 number，保持与 int64 字段一致的序列化形态
func (f flexInt64) MarshalJSON() ([]byte, error) {
	return []byte(strconv.FormatInt(int64(f), 10)), nil
}

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

// UnmarshalJSON 宽容解析 expires_at（flexInt64，issue #56）。
// expires_at 带 omitempty、调用方区分「字段缺失」与「有值」，故按 key 是否出现决定回填：
// 缺失与显式 null 都保持零值，与原 int64 解码语义一致。
// ttl/expires_in 等其余数值字段官方样例与实发均为 number、不在本缺陷链路，保持严格解码不扩大面。
func (c *MediaImageCompleteResponse) UnmarshalJSON(data []byte) error {
	type rawCompleteResponse MediaImageCompleteResponse // 类型别名剥离本方法，避免递归
	var raw struct {
		rawCompleteResponse
		ExpiresAt *flexInt64 `json:"expires_at"`
	}
	if err := json.Unmarshal(data, &raw); err != nil {
		return err
	}
	if raw.ExpiresAt != nil {
		raw.rawCompleteResponse.ExpiresAt = int64(*raw.ExpiresAt)
	}
	*c = MediaImageCompleteResponse(raw.rawCompleteResponse)
	return nil
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

// UnmarshalJSON 宽容解析 created_at/expires_at（flexInt64，issue #56：官方列表实发为字符串数字）。
// 其余字段经别名类型走标准解码，非法形态（对象/布尔/非数字串）仍报错。
func (m *MediaImage) UnmarshalJSON(data []byte) error {
	type rawMediaImage MediaImage
	var raw struct {
		rawMediaImage
		CreatedAt flexInt64 `json:"created_at"`
		ExpiresAt flexInt64 `json:"expires_at"`
	}
	if err := json.Unmarshal(data, &raw); err != nil {
		return err
	}
	raw.rawMediaImage.CreatedAt = int64(raw.CreatedAt)
	raw.rawMediaImage.ExpiresAt = int64(raw.ExpiresAt)
	*m = MediaImage(raw.rawMediaImage)
	return nil
}

// MediaImageList 素材图片列表
type MediaImageList struct {
	Images     []*MediaImage `json:"images"`
	NextCursor string        `json:"next_cursor"`
	Total      int           `json:"total"` // 与单机器人 100 张上限同口径
}

// UnmarshalJSON 宽容解析 total（flexInt64，issue #56：线上实发 "total":"36" 即本缺陷原文）。
// int 在所有受支持平台上为 64 位，100 上限口径下截断风险为零。
func (l *MediaImageList) UnmarshalJSON(data []byte) error {
	type rawMediaImageList MediaImageList
	var raw struct {
		rawMediaImageList
		Total flexInt64 `json:"total"`
	}
	if err := json.Unmarshal(data, &raw); err != nil {
		return err
	}
	raw.rawMediaImageList.Total = int(raw.Total)
	*l = MediaImageList(raw.rawMediaImageList)
	return nil
}

// Package media 是 application/media 的适配层：把 PreparedMedia 上传到
// imagehosting/oss 后端，返回公开 URL。Handler 只依赖 application 层接口。
package media

import (
	"context"
	"fmt"
	"os"

	appmedia "github.com/hoshinonyaruko/gensokyo/internal/application/media"
	"github.com/hoshinonyaruko/gensokyo/imagehosting"
)

// Uploader 实现 application/media.MediaUploader。
type Uploader struct{}

// NewUploader 创建默认上传器。
func NewUploader() *Uploader { return &Uploader{} }

// Upload 把 PreparedMedia 上传到图床，返回公开 URL。
func (u *Uploader) Upload(ctx context.Context, m *appmedia.PreparedMedia) (appmedia.UploadedMedia, error) {
	if m == nil {
		return appmedia.UploadedMedia{}, fmt.Errorf("media: nil prepared media")
	}
	data := m.Data
	if m.TempPath != "" {
		b, err := os.ReadFile(m.TempPath)
		if err != nil {
			return appmedia.UploadedMedia{}, fmt.Errorf("media: read temp file: %w", err)
		}
		data = b
	}
	url, err := imagehosting.UploadBytes(data, "")
	if err != nil {
		return appmedia.UploadedMedia{}, fmt.Errorf("media: upload: %w", err)
	}
	return appmedia.UploadedMedia{URL: url}, nil
}

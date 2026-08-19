package media

import (
	"bytes"
	"fmt"
	"image"
	_ "image/gif"
	_ "image/jpeg"
	_ "image/png"
)

// ValidateImage 解码并校验图片尺寸/像素数，防止解压炸弹。
// 返回图片尺寸；data 原样保留（不在此压缩，压缩由调用方决定）。
func ValidateImage(data []byte, policy MediaPolicy) (int, int, error) {
	cfg, _, err := image.DecodeConfig(bytes.NewReader(data))
	if err != nil {
		return 0, 0, fmt.Errorf("media: image decode config: %w", err)
	}
	if policy.MaxWidth > 0 && cfg.Width > policy.MaxWidth {
		return 0, 0, fmt.Errorf("media: image width %d exceeds limit %d", cfg.Width, policy.MaxWidth)
	}
	if policy.MaxHeight > 0 && cfg.Height > policy.MaxHeight {
		return 0, 0, fmt.Errorf("media: image height %d exceeds limit %d", cfg.Height, policy.MaxHeight)
	}
	if policy.MaxPixels > 0 && cfg.Width*cfg.Height > policy.MaxPixels {
		return 0, 0, fmt.Errorf("media: image pixels %d exceed limit %d", cfg.Width*cfg.Height, policy.MaxPixels)
	}
	return cfg.Width, cfg.Height, nil
}

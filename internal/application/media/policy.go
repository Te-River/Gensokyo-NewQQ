// Package media 提供统一的媒体管线：MediaSource → MediaService → PreparedMedia。
//
// 目标：Handler 不再直接实现 HTTP 下载 / 图片压缩 / FFmpeg / go-silk，
// 所有媒体大小存在硬上限，临时文件生命周期可预测。
package media

// MediaPolicy 媒体处理策略（硬上限，防 OOM / 解压炸弹 / 任意文件读取）。
type MediaPolicy struct {
	// MaxEncodedBytes base64 编码串长度上限（decode 前检查，防 OOM）。
	MaxEncodedBytes int64
	// MaxDecodedBytes base64 解码后字节上限。
	MaxDecodedBytes int64
	// MaxBytes 媒体字节硬上限（URL/本地文件）。
	MaxBytes int64
	// MaxWidth / MaxHeight / MaxPixels 图片尺寸上限（防解压炸弹）。
	MaxWidth  int
	MaxHeight int
	MaxPixels int
	// AllowedDirs 允许读取的本地目录前缀；为空表示不限制本地路径（不推荐）。
	AllowedDirs []string
	// AllowedExtensions 允许的本地文件扩展名；为空表示不限制。
	AllowedExtensions []string
}

// DefaultPolicy 返回保守默认值。
func DefaultPolicy() MediaPolicy {
	return MediaPolicy{
		MaxEncodedBytes: 16 << 20, // 16MB base64
		MaxDecodedBytes: 12 << 20, // 12MB decoded
		MaxBytes:        20 << 20, // 20MB raw
		MaxWidth:        4096,
		MaxHeight:       4096,
		MaxPixels:       40_000_000, // 4000万像素
	}
}

package media

import (
	"os"
)

// PreparedMedia 是媒体管线产物，可能是内存数据或临时文件。
// 使用方必须调用 Close 释放资源（尤其是 TempPath 临时文件）。
type PreparedMedia struct {
	// Data 内存数据（当 TempPath 为空时）。
	Data []byte
	// TempPath 临时文件路径（大媒体落盘，避免内存翻倍）；为空表示无临时文件。
	TempPath string
	// MIME 探测到的媒体类型。
	MIME string
	// Size 字节大小。
	Size int64

	cleanup func()
}

// Close 释放资源（删除临时文件）。可安全重复调用。
func (p *PreparedMedia) Close() error {
	if p == nil {
		return nil
	}
	if p.cleanup != nil {
		p.cleanup()
		p.cleanup = nil
	}
	return nil
}

// newPrepared 构造并绑定清理函数。
func newPrepared(data []byte, mime string, size int64) *PreparedMedia {
	return &PreparedMedia{Data: data, MIME: mime, Size: size}
}

// newPreparedFromTempFile 构造临时文件媒体，Close 时删除文件。
func newPreparedFromTempFile(path, mime string, size int64) *PreparedMedia {
	p := &PreparedMedia{TempPath: path, MIME: mime, Size: size}
	p.cleanup = func() { _ = os.Remove(path) }
	return p
}

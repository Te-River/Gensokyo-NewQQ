package media

import (
	"context"
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"strings"

	"github.com/hoshinonyaruko/gensokyo/internal/domain/message"
)

// MediaService 统一媒体准备入口。
type MediaService interface {
	// Prepare 按来源与策略准备媒体，返回 PreparedMedia（使用后必须 Close）。
	Prepare(ctx context.Context, source message.MediaSource, policy MediaPolicy) (*PreparedMedia, error)
}

// 大媒体内存阈值：超过则落临时文件，避免 ReadAll→Base64 重复大缓冲。
const maxInMemoryBytes int64 = 1 << 20 // 1MB

type mediaService struct {
	fetcher *Fetcher
}

// NewService 创建默认 MediaService 实现。
func NewService(fetcher *Fetcher) MediaService {
	if fetcher == nil {
		fetcher = NewFetcher(FetcherOptions{})
	}
	return &mediaService{fetcher: fetcher}
}

func (s *mediaService) Prepare(ctx context.Context, source message.MediaSource, policy MediaPolicy) (*PreparedMedia, error) {
	switch source.Kind {
	case message.MediaRemoteURL:
		return s.fetch(ctx, source.URL, policy)
	case message.MediaLocalFile:
		return prepareLocalFile(source.Path, policy)
	case message.MediaBase64:
		return prepareBase64(string(source.Data), policy)
	case message.MediaBytes:
		return validateBytes(source.Data, policy)
	default:
		return nil, fmt.Errorf("media: unsupported source kind %d", source.Kind)
	}
}

func (s *mediaService) fetch(ctx context.Context, rawURL string, policy MediaPolicy) (*PreparedMedia, error) {
	if policy.MaxBytes > 0 && policy.MaxBytes < maxInMemoryBytes {
		// 上限较小时直接读内存
		data, mime, err := s.fetcher.Fetch(ctx, rawURL, policy.MaxBytes)
		if err != nil {
			return nil, err
		}
		return newPrepared(data, mime, int64(len(data))), nil
	}

	// 大媒体走临时文件
	path, mime, size, err := s.fetcher.FetchToTempFile(ctx, rawURL, policy.MaxBytes, maxInMemoryBytes)
	if err != nil {
		return nil, err
	}
	return newPreparedFromTempFile(path, mime, size), nil
}

// validateBytes 校验字节媒体的大小上限。
func validateBytes(data []byte, policy MediaPolicy) (*PreparedMedia, error) {
	if policy.MaxBytes > 0 && int64(len(data)) > policy.MaxBytes {
		return nil, fmt.Errorf("media: bytes exceed limit %d", policy.MaxBytes)
	}
	return newPrepared(data, sniffMIME(data), int64(len(data))), nil
}

func sniffMIME(data []byte) string {
	if len(data) == 0 {
		return "application/octet-stream"
	}
	return http.DetectContentType(data)
}

// prepareLocalFile 校验并读取本地文件。
func prepareLocalFile(path string, policy MediaPolicy) (*PreparedMedia, error) {
	abs, err := filepath.Abs(path)
	if err != nil {
		return nil, err
	}

	// 允许目录校验（防任意文件读取）
	if len(policy.AllowedDirs) > 0 {
		allowed := false
		for _, dir := range policy.AllowedDirs {
			d, _ := filepath.Abs(dir)
			if strings.HasPrefix(abs, filepath.Clean(d)+string(os.PathSeparator)) || abs == filepath.Clean(d) {
				allowed = true
				break
			}
		}
		if !allowed {
			return nil, fmt.Errorf("media: path %q outside allowed directories", path)
		}
	}

	// 扩展名校验
	if len(policy.AllowedExtensions) > 0 {
		ext := strings.ToLower(filepath.Ext(abs))
		ok := false
		for _, e := range policy.AllowedExtensions {
			if ext == strings.ToLower(e) {
				ok = true
				break
			}
		}
		if !ok {
			return nil, fmt.Errorf("media: extension %q not allowed", ext)
		}
	}

	info, err := os.Stat(abs)
	if err != nil {
		return nil, err
	}
	if !info.Mode().IsRegular() {
		return nil, fmt.Errorf("media: %q is not a regular file", path)
	}
	if policy.MaxBytes > 0 && info.Size() > policy.MaxBytes {
		return nil, fmt.Errorf("media: file size %d exceeds limit %d", info.Size(), policy.MaxBytes)
	}

	data, err := os.ReadFile(abs)
	if err != nil {
		return nil, err
	}
	return newPrepared(data, sniffMIME(data), info.Size()), nil
}

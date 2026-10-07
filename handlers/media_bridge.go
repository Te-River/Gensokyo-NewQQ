package handlers

import (
	"context"
	"os"

	appmedia "github.com/hoshinonyaruko/gensokyo/internal/application/media"
	"github.com/hoshinonyaruko/gensokyo/internal/domain/message"
	"github.com/hoshinonyaruko/gensokyo/mylog"
)

// 本文件是 A 包的媒体桥：把 handlers 的媒体分支接到 application/media 的
// MediaService（arch_mode=new 时启用），legacy 分支保持原样。

var (
	mediaSvc    appmedia.MediaService
	mediaPolicy appmedia.MediaPolicy
)

// SetMediaService 注入统一媒体服务与策略（main.go bootstrap 调用）。
func SetMediaService(s appmedia.MediaService, p appmedia.MediaPolicy) {
	mediaSvc = s
	mediaPolicy = p
}

// MediaService 返回当前注入的媒体服务（诊断/测试用）。
func MediaService() appmedia.MediaService { return mediaSvc }

// PrepareMedia 在 arch_mode=new 时走统一媒体管线；否则返回 (nil,false) 由 legacy 继续。
func PrepareMedia(ctx context.Context, source message.MediaSource) (*appmedia.PreparedMedia, bool) {
	if ArchMode() == "legacy" || mediaSvc == nil {
		return nil, false
	}
	pm, err := mediaSvc.Prepare(ctx, source, mediaPolicy)
	if err != nil {
		mylog.Printf("[media] Prepare 失败: %v", err)
		return nil, false
	}
	return pm, true
}

// readLocalMedia 读取本地媒体字节：arch_mode=new 时经 MediaService.Prepare，
// 否则（或 Prepare 失败时）回退 os.ReadFile，保证 legacy 逐字节不变。
func readLocalMedia(path string) ([]byte, error) {
	if pm, ok := PrepareMedia(context.Background(), message.MediaSource{Kind: message.MediaLocalFile, Path: path}); ok {
		defer pm.Close()
		if pm.TempPath != "" {
			return os.ReadFile(pm.TempPath)
		}
		return pm.Data, nil
	}
	return os.ReadFile(path)
}

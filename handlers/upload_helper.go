package handlers

import (
	"bytes"
	"context"
	"crypto/md5"
	"encoding/hex"
	"fmt"
	"io"
	"net/http"
	"os"
	"time"

	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

// ---------- 软限制（超过此限制的文件自动走分片上传） ----------

var fileTypeSoftLimits = map[int]int64{
	1: 20 * 1024 * 1024,   // 图片：20 MB
	2: 30 * 1024 * 1024,   // 视频：30 MB
	3: 20 * 1024 * 1024,   // 语音：20 MB
	4: 200 * 1024 * 1024,  // 文件：200 MB（软限制 == 硬限制）
}

const defaultBlockSize = 5 * 1024 * 1024 // 5MB（备用，优先用服务端下发的 block_size）

// needsChunkedUpload 判断文件是否需要走分片上传
func needsChunkedUpload(fileSize int64, fileType int) bool {
	limit, ok := fileTypeSoftLimits[fileType]
	if !ok {
		limit = 200 * 1024 * 1024
	}
	return fileSize >= limit
}

// chunkedUpload 完整的分片上传流程
// 参数：
//
//	id    — 用户 OpenID（单聊）或群 OpenID（群聊）
//	isGroup — true=群聊, false=单聊
//	data  — 文件原始字节
//	fileType — 1=图片, 2=视频, 3=语音, 4=文件
//	fileName — 文件名（可选）
//
// 返回：file_info（用于 msg_type=7 的 media.file_info 字段）
func chunkedUpload(ctx context.Context, apiv2 openapi.OpenAPI, id string, isGroup bool, data []byte, fileType int, fileName string) (string, error) {
	fileSize := len(data)
	if fileName == "" {
		fileName = fmt.Sprintf("upload_%d", time.Now().Unix())
	}

	// 1. 预上传
	md5Hash := computeMD5(data)
	md510m := computeMD510m(data)
	prepareReq := &dto.UploadPrepareRequest{
		FileType: fileType,
		FileSize: fmt.Sprintf("%d", fileSize),
		FileName: fileName,
		MD5:      md5Hash,
		MD510m:   md510m,
	}

	prepareResp, err := apiv2.FileUploadPrepare(ctx, id, isGroup, prepareReq)
	if err != nil {
		return "", fmt.Errorf("upload prepare failed: %w", err)
	}

	if prepareResp == nil || len(prepareResp.Parts) == 0 {
		return "", fmt.Errorf("upload prepare returned no parts")
	}

	mylog.Printf("分片上传准备完成: upload_id=%s, parts=%d", prepareResp.UploadID, len(prepareResp.Parts))

	// 确定分片大小（优先使用服务端下发的）
	blockSize := defaultBlockSize
	if prepareResp.BlockSize != "" {
		if s, err := fmt.Sscanf(prepareResp.BlockSize, "%d", &blockSize); err != nil || s != 1 {
			blockSize = defaultBlockSize
		}
	}

	// 2. 逐片上传
	client := &http.Client{Timeout: 300 * time.Second}
	for _, part := range prepareResp.Parts {
		start := part.Index * blockSize
		end := start + blockSize
		if end > fileSize {
			end = fileSize
		}
		chunkData := data[start:end]

		// 2a. PUT 到预签名 URL
		partMD5 := computeMD5(chunkData)
		req, err := http.NewRequest("PUT", part.PresignedURL, bytes.NewReader(chunkData))
		if err != nil {
			return "", fmt.Errorf("创建分片 PUT 请求失败 (part %d): %w", part.Index, err)
		}
		req.Header.Set("Content-Type", "application/octet-stream")

		resp, err := client.Do(req)
		if err != nil {
			return "", fmt.Errorf("分片 PUT 失败 (part %d): %w", part.Index, err)
		}
		resp.Body.Close()
		if resp.StatusCode != http.StatusOK && resp.StatusCode != http.StatusCreated && resp.StatusCode != http.StatusNoContent {
			return "", fmt.Errorf("分片 PUT 返回非预期状态码 (part %d): %d", part.Index, resp.StatusCode)
		}

		// 2b. 通知服务端分片完成
		partFinishReq := &dto.UploadPartFinishRequest{
			UploadID:  prepareResp.UploadID,
			PartIndex: part.Index,
			BlockSize: fmt.Sprintf("%d", len(chunkData)),
			MD5:       partMD5,
		}
		if err := apiv2.FileUploadPartFinish(ctx, id, isGroup, partFinishReq); err != nil {
			return "", fmt.Errorf("分片完成通知失败 (part %d): %w", part.Index, err)
		}

		mylog.Printf("分片 %d/%d 上传完成", part.Index+1, len(prepareResp.Parts))
	}

	// 3. 合并分片
	mergeReq := &dto.FileUploadRequest{
		FileType:   fileType,
		FileName:   fileName,
		SrvSendMsg: false,
		UploadID:   prepareResp.UploadID,
	}
	mergeResp, err := apiv2.FileUploadMerge(ctx, id, isGroup, mergeReq)
	if err != nil {
		return "", fmt.Errorf("分片合并失败: %w", err)
	}
	if mergeResp == nil || mergeResp.FileInfo == "" {
		return "", fmt.Errorf("分片合并返回空 file_info")
	}

	mylog.Printf("分片上传合并完成: file_info=%s, ttl=%d", mergeResp.FileInfo, mergeResp.TTL)
	return mergeResp.FileInfo, nil
}

// tryChunkedUpload 尝试分片上传；如果文件太小或不满足条件则回退到 URL 直传
// 返回 (fileInfo, shouldFallback, error)
// shouldFallback=true 时调用方应使用原有的 URL 直传逻辑
func tryChunkedUpload(ctx context.Context, apiv2 openapi.OpenAPI, id string, isGroup bool, data []byte, fileType int, fileName string) (string, error) {
	if !needsChunkedUpload(int64(len(data)), fileType) {
		return "", fmt.Errorf("文件大小未超过软限制，使用 URL 直传")
	}
	return chunkedUpload(ctx, apiv2, id, isGroup, data, fileType, fileName)
}

// ---------- 辅助函数 ----------

func computeMD5(data []byte) string {
	h := md5.New()
	h.Write(data)
	return hex.EncodeToString(h.Sum(nil))
}

// computeMD510m 计算文件前 10002432 字节（约 10MB）的 MD5
func computeMD510m(data []byte) string {
	const limit = 10002432
	h := md5.New()
	if len(data) > limit {
		h.Write(data[:limit])
	} else {
		h.Write(data)
	}
	return hex.EncodeToString(h.Sum(nil))
}

// getFileSize 获取文件大小（字节）
// 为兼容 Go 1.25 的 os.Root 语义，使用 os.Stat
func getFileSize(path string) (int64, error) {
	info, err := os.Stat(path)
	if err != nil {
		return 0, err
	}
	return info.Size(), nil
}

// readFileWithLimit 安全地读取文件，最大读取 hardLimit（200MB）
func readFileWithLimit(path string, hardLimit int64) ([]byte, error) {
	size, err := getFileSize(path)
	if err != nil {
		return nil, err
	}
	if size > hardLimit {
		return nil, fmt.Errorf("文件大小 %d 超过硬限制 %d", size, hardLimit)
	}
	f, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer f.Close()
	return io.ReadAll(io.LimitReader(f, hardLimit))
}

// getHardLimit 返回硬限制（200MB），超过此值会上传失败
func getHardLimit() int64 {
	return 200 * 1024 * 1024
}

// isChunkedUploadEnabled 分片上传功能始终启用，由文件大小自动判断
func isChunkedUploadEnabled() bool {
	return true
}

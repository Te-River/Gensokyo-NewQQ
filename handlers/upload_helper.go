package handlers

import (
	"bytes"
	"context"
	"crypto/md5"
	"encoding/base64"
	"encoding/hex"
	"fmt"
	"io"
	"net/http"
	"os"
	"time"

	"github.com/hoshinonyaruko/gensokyo/images"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

// maxDirectFileDataBytes 官方 file_data 直传上限：超过 5MB 必须改用 URL 上传或分片上传。
// 来源：POST /v2/groups/{group_openid}/files 与 POST /v2/users/{user_openid}/files 现文档。
// 官方 files 页（2026-10-09）只写"5M / 数据大小不能超过 5M"，未定义十进制或二进制；
// 取保守十进制 5,000,000 字节，超过即走分片上传，结果仍正确。
const maxDirectFileDataBytes = 5 * 1000 * 1000

// fileTypeSoftLimits 各类型的平台软限制：分片上传合并后，超过软限制平台会把内容降级为文件类型下发，
// 超过硬限制（getHardLimit）则报错。仅用于日志提示，不再作为是否走 file_data 直传的判据。
var fileTypeSoftLimits = map[int]int64{
	1: 20 * 1024 * 1024,  // 图片：20 MB
	2: 30 * 1024 * 1024,  // 视频：30 MB
	3: 20 * 1024 * 1024,  // 语音：20 MB
	4: 200 * 1024 * 1024, // 文件：200 MB（软限制 == 硬限制）
}

const defaultBlockSize = 5 * 1024 * 1024 // 5MB（备用，优先用服务端下发的 block_size）

// needsChunkedUpload 判断 base64/二进制直传是否必须改走分片上传。
// 判据只有一条：超过官方 file_data 直传上限（5MB）。
// fileType 仅用于日志提示：超过该类型软限制时，平台会把内容降级为文件类型下发。
func needsChunkedUpload(fileSize int64, fileType int) bool {
	if fileSize <= maxDirectFileDataBytes {
		return false
	}
	if limit, ok := fileTypeSoftLimits[fileType]; ok && fileSize > limit {
		mylog.Printf("文件大小 %d 超过类型 %d 的软限制 %d，平台侧将降级为文件类型下发", fileSize, fileType, limit)
	}
	return true
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
// 返回 (fileInfo, error)：error 非 nil 时调用方应使用原有的 URL 直传逻辑
func tryChunkedUpload(ctx context.Context, apiv2 openapi.OpenAPI, id string, isGroup bool, data []byte, fileType int, fileName string) (string, error) {
	if !needsChunkedUpload(int64(len(data)), fileType) {
		return "", fmt.Errorf("文件大小未超过 file_data 直传上限(5MB)，使用 URL 直传")
	}
	return chunkedUpload(ctx, apiv2, id, isGroup, data, fileType, fileName)
}

// mediaMessageToCreate 构造 msg_type=7 的富媒体消息，字段与 images 包保持一致
func mediaMessageToCreate(fileInfo, content, eventID, messageID string, msgseq int) *dto.MessageToCreate {
	return &dto.MessageToCreate{
		Content: content,
		Media:   &dto.Media{FileInfo: fileInfo},
		MsgID:   messageID,
		EventID: eventID,
		MsgSeq:  msgseq,
		MsgType: 7,
	}
}

// createAndUploadMediaMessage 群聊富媒体上传：base64 直传超过 5MB 时改走分片上传。
// images 包不能反向调用 handlers 的 chunkedUpload（handlers → images 单向依赖），
// 因此直传上限的判定放在调用侧；≤5MB 仍交给 images 包走 file_data 直传。
func createAndUploadMediaMessage(ctx context.Context, base64EncodedData, eventID string, fileType uint64, srvSendMsg bool, content string, groupID string, messageID string, msgseq int, apiv2 openapi.OpenAPI) (*dto.MessageToCreate, error) {
	if decoded, err := base64.StdEncoding.DecodeString(base64EncodedData); err == nil && needsChunkedUpload(int64(len(decoded)), int(fileType)) {
		mylog.Printf("base64 数据超过 file_data 直传上限(5MB)，自动切换到分片上传 (type=%d, size=%d)", fileType, len(decoded))
		fileInfo, err := chunkedUpload(ctx, apiv2, groupID, true, decoded, int(fileType), content)
		if err != nil {
			return nil, err
		}
		return mediaMessageToCreate(fileInfo, content, eventID, messageID, msgseq), nil
	}
	return images.CreateAndUploadMediaMessage(ctx, base64EncodedData, eventID, fileType, srvSendMsg, content, groupID, messageID, msgseq, apiv2)
}

// createAndUploadMediaMessagePrivate 单聊版本，语义同 createAndUploadMediaMessage
func createAndUploadMediaMessagePrivate(ctx context.Context, base64EncodedData, eventID string, fileType uint64, srvSendMsg bool, content string, userID string, messageID string, msgseq int, apiv2 openapi.OpenAPI) (*dto.MessageToCreate, error) {
	if decoded, err := base64.StdEncoding.DecodeString(base64EncodedData); err == nil && needsChunkedUpload(int64(len(decoded)), int(fileType)) {
		mylog.Printf("base64 数据超过 file_data 直传上限(5MB)，自动切换到分片上传 (type=%d, size=%d)", fileType, len(decoded))
		fileInfo, err := chunkedUpload(ctx, apiv2, userID, false, decoded, int(fileType), content)
		if err != nil {
			return nil, err
		}
		return mediaMessageToCreate(fileInfo, content, eventID, messageID, msgseq), nil
	}
	return images.CreateAndUploadMediaMessagePrivate(ctx, base64EncodedData, eventID, fileType, srvSendMsg, content, userID, messageID, msgseq, apiv2)
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

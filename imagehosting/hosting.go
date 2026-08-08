// Package imagehosting 提供统一的图床上传接口。
//
// 本包是 oss_type 的后端实现之一，**不再由用户同时启用多个图床**。
// 具体使用哪个后端由配置项 oss_type 决定：
//   4 = COS（腾讯云对象存储，自签）
//   5 = Bilibili
//   6 = QQ频道
//   7 = ChatGLM
//   8 = Ukaka
//   9 = 星野
//  10 = Nature
//
// 使用方式:
//
//	url, err := imagehosting.UploadProvider("chatglm", imageBytes, "image.png")
//
package imagehosting

import (
	"bytes"
	"encoding/base64"
	"fmt"
	"image"
	_ "image/gif"
	_ "image/jpeg"
	_ "image/png"
	"io"
	"net/http"
	"strings"
	"time"

	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/mylog"
)

const maxProviderResponseBytes int64 = 4 << 20

var providerHTTPClient = &http.Client{Timeout: 30 * time.Second}

// UploadProvider 按指定 provider 上传图片。
// provider 名称与 config.GetOssTypeName(config.OssTypeXXX) 保持一致。
func UploadProvider(provider string, imageData []byte, filename string) (string, error) {
	mylog.Printf("图床上传 provider=%s", provider)
	switch provider {
	case "cos":
		return tryCOS(imageData, filename)
	case "bilibili":
		return tryBilibili(imageData, filename)
	case "qq_channel":
		return tryQQChannel(imageData, filename)
	case "chatglm":
		return tryChatGLM(imageData, filename)
	case "ukaka":
		return tryUkaka(imageData, filename)
	case "xingye":
		return tryXingye(imageData, filename)
	case "nature":
		return tryNature(imageData, filename)
	default:
		return "", fmt.Errorf("未知或不支持的图床 provider: %s", provider)
	}
}

// UploadBase64Provider 解码 base64 后按指定 provider 上传。
func UploadBase64Provider(provider string, base64Data string, filename string) (string, error) {
	imageData, err := base64.StdEncoding.DecodeString(base64Data)
	if err != nil {
		return "", fmt.Errorf("base64 解码失败: %w", err)
	}
	return UploadProvider(provider, imageData, filename)
}

// Upload 兼容旧接口，按当前 oss_type 名称上传。
// 注意：oss_type 为 0~3 时不会走到本包，请优先使用 UploadProvider。
func Upload(base64Data string, filename string) (string, error) {
	return UploadBase64Provider(config.GetOssTypeName(config.GetOssType()), base64Data, filename)
}

// UploadBytes 兼容旧接口，按当前 oss_type 名称上传。
// 注意：oss_type 为 0~3 时不会走到本包，请优先使用 UploadProvider。
func UploadBytes(imageData []byte, filename string) (string, error) {
	return UploadProvider(config.GetOssTypeName(config.GetOssType()), imageData, filename)
}

// ---------- 辅助函数 ----------

// detectMIME 从图片 bytes 检测 MIME 类型
func detectMIME(data []byte) string {
	if len(data) < 12 {
		return "image/jpeg"
	}
	switch {
	case bytes.HasPrefix(data, []byte{0x89, 0x50, 0x4E, 0x47}):
		return "image/png"
	case bytes.HasPrefix(data, []byte{0xFF, 0xD8, 0xFF}):
		return "image/jpeg"
	case bytes.HasPrefix(data, []byte("GIF87a")) || bytes.HasPrefix(data, []byte("GIF89a")):
		return "image/gif"
	case len(data) > 12 && string(data[8:12]) == "WEBP":
		return "image/webp"
	default:
		return "image/jpeg"
	}
}

// detectExt 从图片 bytes 检测扩展名
func detectExt(data []byte) string {
	switch detectMIME(data) {
	case "image/png":
		return "png"
	case "image/jpeg":
		return "jpg"
	case "image/gif":
		return "gif"
	case "image/webp":
		return "webp"
	default:
		return "jpg"
	}
}

// getImageDimensions 从 bytes 读取图片尺寸
func getImageDimensions(data []byte) (int, int) {
	img, _, err := image.DecodeConfig(bytes.NewReader(data))
	if err != nil {
		return 0, 0
	}
	return img.Width, img.Height
}

// httpPost 简化的 HTTP POST 请求
func httpPost(url, contentType string, body io.Reader, header map[string]string) (*http.Response, error) {
	req, err := http.NewRequest("POST", url, body)
	if err != nil {
		return nil, err
	}
	if contentType != "" {
		req.Header.Set("Content-Type", contentType)
	}
	for k, v := range header {
		req.Header.Set(k, v)
	}
	req.Header.Set("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
	return providerHTTPClient.Do(req)
}

// httpPut 简化的 HTTP PUT 请求
func httpPut(url, contentType string, body io.Reader, header map[string]string) (*http.Response, error) {
	req, err := http.NewRequest("PUT", url, body)
	if err != nil {
		return nil, err
	}
	if contentType != "" {
		req.Header.Set("Content-Type", contentType)
	}
	for k, v := range header {
		req.Header.Set(k, v)
	}
	req.Header.Set("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
	return providerHTTPClient.Do(req)
}

// readClose 读取并关闭响应体
func readClose(resp *http.Response) ([]byte, error) {
	if resp == nil || resp.Body == nil {
		return nil, fmt.Errorf("HTTP response body is nil")
	}
	defer resp.Body.Close()
	body, err := io.ReadAll(io.LimitReader(resp.Body, maxProviderResponseBytes+1))
	if err != nil {
		return nil, err
	}
	if int64(len(body)) > maxProviderResponseBytes {
		return nil, fmt.Errorf("HTTP response body exceeds %d bytes", maxProviderResponseBytes)
	}
	return body, nil
}

// ensureExt 确保文件名有正确的扩展名
func ensureExt(filename string, data []byte) string {
	ext := detectExt(data)
	if strings.HasSuffix(strings.ToLower(filename), "."+ext) {
		return filename
	}
	// 去掉旧后缀
	if idx := strings.LastIndex(filename, "."); idx >= 0 {
		filename = filename[:idx]
	}
	return filename + "." + ext
}

// Bilibili 图床 — 利用 B 站开放平台图片上传接口。
// 需要配置 Cookie (SESSDATA + bili_jct)。
package imagehosting

import (
	"bytes"
	"fmt"
	"mime/multipart"
	"net/http"
	"strings"

	"github.com/hoshinonyaruko/gensokyo/config"
)

func tryBilibili(data []byte, filename string) (string, error) {
	cfg := config.GetImageHostingBilibili()
	if cfg.Sessdata == "" || cfg.CSRFToken == "" {
		return "", fmt.Errorf("Bilibili 未配置（请填写 csrf_token 和 sessdata）")
	}

	filename = ensureExt(filename, data)
	body := &bytes.Buffer{}
	writer := multipart.NewWriter(body)
	part, err := writer.CreateFormFile("file", filename)
	if err != nil {
		return "", fmt.Errorf("创建 form 失败: %w", err)
	}
	if _, err := part.Write(data); err != nil {
		return "", fmt.Errorf("写入图片失败: %w", err)
	}
	if err := writer.Close(); err != nil {
		return "", fmt.Errorf("关闭 multipart writer 失败: %w", err)
	}

	bucket := cfg.Bucket
	if bucket == "" {
		bucket = "openplatform"
	}

	req, err := http.NewRequest(http.MethodPost, "https://api.bilibili.com/x/upload/web/image", body)
	if err != nil {
		return "", fmt.Errorf("创建请求失败: %w", err)
	}
	req.Header.Set("Content-Type", writer.FormDataContentType())
	req.Header.Set("User-Agent", "Gensokyo-NewQQ/imagehosting")
	req.Header.Set("Cookie", fmt.Sprintf("SESSDATA=%s; bili_jct=%s", cfg.Sessdata, cfg.CSRFToken))

	query := req.URL.Query()
	query.Add("bucket", bucket)
	query.Add("csrf", cfg.CSRFToken)
	req.URL.RawQuery = query.Encode()

	resp, err := providerHTTPClient.Do(req)
	if err != nil {
		return "", fmt.Errorf("上传请求失败: %w", err)
	}
	bodyBytes, readErr := readClose(resp)
	if readErr != nil {
		return "", fmt.Errorf("读取 Bilibili 响应失败: %w", readErr)
	}

	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return "", fmt.Errorf("Bilibili 返回 HTTP %d: %s", resp.StatusCode, string(bodyBytes))
	}

	var result struct {
		Code    int    `json:"code"`
		Message string `json:"message"`
		Data    struct {
			Location string `json:"location"`
		} `json:"data"`
	}
	if err := jsonUnmarshal(bodyBytes, &result); err != nil {
		return "", fmt.Errorf("解析响应失败: %w", err)
	}
	if result.Code != 0 {
		return "", fmt.Errorf("Bilibili 业务错误: code=%d msg=%s", result.Code, result.Message)
	}
	if result.Data.Location == "" {
		return "", fmt.Errorf("Bilibili 返回成功但 location 为空")
	}

	imageURL := strings.TrimSpace(result.Data.Location)
	switch {
	case strings.HasPrefix(imageURL, "//"):
		imageURL = "https:" + imageURL
	case strings.HasPrefix(imageURL, "http://"):
		imageURL = "https://" + strings.TrimPrefix(imageURL, "http://")
	}
	if err := requireHTTPSURL(imageURL); err != nil {
		return "", fmt.Errorf("Bilibili 返回无效图片 URL: %w", err)
	}
	return imageURL, nil
}


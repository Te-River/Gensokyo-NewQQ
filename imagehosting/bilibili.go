// Bilibili 图床 — 利用 B 站开放平台图片上传接口
// 需要配置 Cookie (SESSDATA + bili_jct)
package imagehosting

import (
	"bytes"
	"fmt"
	"mime/multipart"
	"net/http"

	"github.com/hoshinonyaruko/gensokyo/config"
)

func tryBilibili(data []byte, filename string) (string, error) {
	cfg := config.GetImageHostingBilibili()
	if cfg.Sessdata == "" || cfg.CSRFToken == "" {
		return "", fmt.Errorf("Bilibili 未配置（请填写 csrf_token 和 sessdata）")
	}

	filename = ensureExt(filename, data)
	_ = detectMIME(data) // 用于后续扩展

	body := &bytes.Buffer{}
	writer := multipart.NewWriter(body)
	part, err := writer.CreateFormFile("file", filename)
	if err != nil {
		return "", fmt.Errorf("创建 form 失败: %w", err)
	}
	part.Write(data)
	writer.Close()

	bucket := cfg.Bucket
	if bucket == "" {
		bucket = "openplatform"
	}

	req, err := http.NewRequest("POST", "https://api.bilibili.com/x/upload/web/image", body)
	if err != nil {
		return "", fmt.Errorf("创建请求失败: %w", err)
	}
	req.Header.Set("Content-Type", writer.FormDataContentType())
	req.Header.Set("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
	req.Header.Set("Cookie", fmt.Sprintf("SESSDATA=%s; bili_jct=%s", cfg.Sessdata, cfg.CSRFToken))

	// 添加 bucket 和 csrf 参数
	q := req.URL.Query()
	q.Add("bucket", bucket)
	q.Add("csrf", cfg.CSRFToken)
	req.URL.RawQuery = q.Encode()

	resp, err := providerHTTPClient.Do(req)
	if err != nil {
		return "", fmt.Errorf("上传请求失败: %w", err)
	}
	bodyBytes, readErr := readClose(resp)
	if readErr != nil {
		return "", fmt.Errorf("读取 Bilibili 响应失败: %w", readErr)
	}

	if resp.StatusCode != 200 {
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

	url := result.Data.Location
	if len(url) > 4 && url[:4] == "http" {
		if url[:5] == "http:" {
			url = "https:" + url[5:]
		}
	}
	return url, nil
}

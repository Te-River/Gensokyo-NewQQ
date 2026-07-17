// ChatGLM 图床 — 智谱第三方图床。
// 仅在管理员显式允许第三方图床后使用。
package imagehosting

import (
	"bytes"
	"fmt"
	"mime/multipart"
	"net/http"
	"strings"
)

func tryChatGLM(data []byte, filename string) (string, error) {
	filename = ensureExt(filename, data)
	mime := detectMIME(data)

	body := &bytes.Buffer{}
	writer := multipart.NewWriter(body)
	part, err := writer.CreateFormFile("file", filename)
	if err != nil {
		return "", fmt.Errorf("创建 form 失败: %w", err)
	}
	if _, err = part.Write(data); err != nil {
		return "", fmt.Errorf("写入文件数据失败: %w", err)
	}
	if err = writer.Close(); err != nil {
		return "", fmt.Errorf("关闭 multipart writer 失败: %w", err)
	}

	req, err := http.NewRequest(http.MethodPost, "https://chatglm.cn/chatglm/backend-api/assistant/file_upload", body)
	if err != nil {
		return "", fmt.Errorf("创建请求失败: %w", err)
	}
	req.Header.Set("Content-Type", writer.FormDataContentType())
	req.Header.Set("User-Agent", "Gensokyo-NewQQ/imagehosting")
	req.Header.Set("X-File-Mime", mime)

	resp, err := imageHostingHTTPClient.Do(req)
	if err != nil {
		return "", fmt.Errorf("上传请求失败: %w", err)
	}
	bodyBytes, readErr := readClose(resp)
	if readErr != nil {
		return "", fmt.Errorf("读取 ChatGLM 响应失败: %w", readErr)
	}

	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return "", fmt.Errorf("ChatGLM 返回 HTTP %d: %s", resp.StatusCode, string(bodyBytes))
	}

	var result struct {
		Result struct {
			FileURL string `json:"file_url"`
		} `json:"result"`
	}
	if err := jsonUnmarshal(bodyBytes, &result); err != nil {
		return "", fmt.Errorf("解析响应失败: %w", err)
	}
	fileURL := strings.TrimSpace(result.Result.FileURL)
	if fileURL == "" {
		return "", fmt.Errorf("ChatGLM 返回空 URL")
	}
	if err := requireHTTPSURL(fileURL); err != nil {
		return "", fmt.Errorf("ChatGLM 返回无效图片 URL: %w", err)
	}
	return fileURL, nil
}

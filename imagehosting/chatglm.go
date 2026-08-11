// ChatGLM 图床 — 智谱免费图床
// 无需配置，启用即可使用
package imagehosting

import (
	"bytes"
	"fmt"
	"mime/multipart"
	"net/http"
)

func tryChatGLM(data []byte, filename string) (string, error) {
	filename = ensureExt(filename, data)
	mime := detectMIME(data)

	// 构造 multipart/form-data 请求
	body := &bytes.Buffer{}
	writer := multipart.NewWriter(body)
	part, err := writer.CreateFormFile("file", filename)
	if err != nil {
		return "", fmt.Errorf("创建 form 失败: %w", err)
	}
	_, err = part.Write(data)
	if err != nil {
		return "", fmt.Errorf("写入文件数据失败: %w", err)
	}
	writer.Close()

	req, err := http.NewRequest("POST", "https://chatglm.cn/chatglm/backend-api/assistant/file_upload", body)
	if err != nil {
		return "", fmt.Errorf("创建请求失败: %w", err)
	}
	req.Header.Set("Content-Type", writer.FormDataContentType())
	req.Header.Set("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
	// 指定图片 MIME 让服务端正确识别
	req.Header.Set("X-File-Mime", mime)

	resp, err := providerHTTPClient.Do(req)
	if err != nil {
		return "", fmt.Errorf("上传请求失败: %w", err)
	}
	bodyBytes, readErr := readClose(resp)
	if readErr != nil {
		return "", fmt.Errorf("读取 ChatGLM 响应失败: %w", readErr)
	}

	if resp.StatusCode != 200 {
		return "", fmt.Errorf("ChatGLM 返回 HTTP %d: %s", resp.StatusCode, string(bodyBytes))
	}

	// 解析 JSON 响应
	// {"result":{"file_url":"https://..."}}
	var result struct {
		Result struct {
			FileURL string `json:"file_url"`
		} `json:"result"`
	}
	if err := jsonUnmarshal(bodyBytes, &result); err != nil {
		return "", fmt.Errorf("解析响应失败: %w", err)
	}
	if result.Result.FileURL == "" {
		return "", fmt.Errorf("ChatGLM 返回空 URL")
	}
	return result.Result.FileURL, nil
}

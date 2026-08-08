// Ukaka / 星野 图床 — 签名上传
// 免费，无需配置，共用签名服务器 https://bed-sign.vercel.0013107.xyz
package imagehosting

import (
	"bytes"
	"fmt"
	"mime/multipart"
	"net/http"
)

const (
	_signURL    = "https://bed-sign.vercel.0013107.xyz/sign"
	_signOrigin = "https://bed.vercel.0013107.xyz"
)

func tryUkaka(data []byte, filename string) (string, error) {
	return signedUpload(data, filename, "ukaka")
}

func tryXingye(data []byte, filename string) (string, error) {
	return signedUpload(data, filename, "xingye")
}

func signedUpload(data []byte, filename, module string) (string, error) {
	filename = ensureExt(filename, data)
	mime := detectMIME(data)

	// 1. 获取签名
	signResp, err := httpGet(_signURL, map[string]string{
		"module":   module,
		"filename": filename,
		"mimeType": mime,
	})
	if err != nil {
		return "", fmt.Errorf("获取签名失败: %w", err)
	}

	var signData struct {
		URL          string            `json:"url"`
		ResourceURL  string            `json:"resourceUrl"`
		Header       map[string]string `json:"header"`
		Body         map[string]string `json:"body"`
	}
	if err := jsonUnmarshal(signResp, &signData); err != nil {
		return "", fmt.Errorf("解析签名响应失败: %w", err)
	}
	if signData.URL == "" || signData.ResourceURL == "" {
		return "", fmt.Errorf("签名返回数据不完整")
	}

	// 2. 上传
	if module == "xingye" {
		// 星野：PUT 直传
		ct := signData.Header["Content-Type"]
		if ct == "" {
			ct = mime
		}
		resp, err := httpPut(signData.URL, ct, bytes.NewReader(data), nil)
		if err != nil {
			return "", fmt.Errorf("星野上传失败: %w", err)
		}
		if _, readErr := readClose(resp); readErr != nil {
			return "", fmt.Errorf("读取星野响应失败: %w", readErr)
		}
		if resp.StatusCode >= 300 {
			return "", fmt.Errorf("星野返回 HTTP %d", resp.StatusCode)
		}
	} else {
		// Ukaka：POST multipart
		body := &bytes.Buffer{}
		writer := multipart.NewWriter(body)

		// 写入 body 中的额外字段
		for k, v := range signData.Body {
			if k != "file" && v != "" {
				writer.WriteField(k, v)
			}
		}

		part, err := writer.CreateFormFile("file", filename)
		if err != nil {
			return "", fmt.Errorf("创建 form 失败: %w", err)
		}
		part.Write(data)
		writer.Close()

		resp, err := providerHTTPClient.Post(signData.URL, writer.FormDataContentType(), body)
		if err != nil {
			return "", fmt.Errorf("Ukaka 上传失败: %w", err)
		}
		if _, readErr := readClose(resp); readErr != nil {
			return "", fmt.Errorf("读取 Ukaka 响应失败: %w", readErr)
		}
		if resp.StatusCode >= 300 {
			return "", fmt.Errorf("Ukaka 返回 HTTP %d", resp.StatusCode)
		}
	}

	return signData.ResourceURL, nil
}

// httpGet 简化的 HTTP GET 请求
func httpGet(url string, params map[string]string) ([]byte, error) {
	req, err := http.NewRequest("GET", url, nil)
	if err != nil {
		return nil, err
	}
	q := req.URL.Query()
	for k, v := range params {
		q.Add(k, v)
	}
	req.URL.RawQuery = q.Encode()
	req.Header.Set("Accept", "*/*")
	req.Header.Set("Origin", _signOrigin)
	req.Header.Set("Referer", _signOrigin+"/")
	req.Header.Set("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")

	resp, err := providerHTTPClient.Do(req)
	if err != nil {
		return nil, err
	}
	return readClose(resp)
}

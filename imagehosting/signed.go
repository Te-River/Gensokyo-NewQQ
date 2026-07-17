// Ukaka / 星野图床 — 第三方签名上传。
// 仅在管理员显式允许第三方图床后使用。
package imagehosting

import (
	"bytes"
	"fmt"
	"mime/multipart"
	"net"
	"net/http"
	"net/url"
	"strings"
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

	signResp, err := httpGet(_signURL, map[string]string{
		"module":   module,
		"filename": filename,
		"mimeType": mime,
	})
	if err != nil {
		return "", fmt.Errorf("获取签名失败: %w", err)
	}

	var signData struct {
		URL         string            `json:"url"`
		ResourceURL string            `json:"resourceUrl"`
		Header      map[string]string `json:"header"`
		Body        map[string]string `json:"body"`
	}
	if err := jsonUnmarshal(signResp, &signData); err != nil {
		return "", fmt.Errorf("解析签名响应失败: %w", err)
	}
	if signData.URL == "" || signData.ResourceURL == "" {
		return "", fmt.Errorf("签名返回数据不完整")
	}
	if err := requireHTTPSURL(signData.URL); err != nil {
		return "", fmt.Errorf("签名上传地址无效: %w", err)
	}
	if err := requireHTTPSURL(signData.ResourceURL); err != nil {
		return "", fmt.Errorf("资源地址无效: %w", err)
	}

	if module == "xingye" {
		contentType := signData.Header["Content-Type"]
		if contentType == "" {
			contentType = mime
		}
		resp, err := httpPut(signData.URL, contentType, bytes.NewReader(data), nil)
		if err != nil {
			return "", fmt.Errorf("星野上传失败: %w", err)
		}
		responseBody, readErr := readClose(resp)
		if readErr != nil {
			return "", fmt.Errorf("读取星野响应失败: %w", readErr)
		}
		if resp.StatusCode < 200 || resp.StatusCode >= 300 {
			return "", fmt.Errorf("星野返回 HTTP %d: %s", resp.StatusCode, string(responseBody))
		}
	} else {
		body := &bytes.Buffer{}
		writer := multipart.NewWriter(body)
		for key, value := range signData.Body {
			if key == "file" || value == "" {
				continue
			}
			if err := writer.WriteField(key, value); err != nil {
				return "", fmt.Errorf("写入上传字段失败: %w", err)
			}
		}

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

		resp, err := httpPost(signData.URL, writer.FormDataContentType(), body, nil)
		if err != nil {
			return "", fmt.Errorf("Ukaka 上传失败: %w", err)
		}
		responseBody, readErr := readClose(resp)
		if readErr != nil {
			return "", fmt.Errorf("读取 Ukaka 响应失败: %w", readErr)
		}
		if resp.StatusCode < 200 || resp.StatusCode >= 300 {
			return "", fmt.Errorf("Ukaka 返回 HTTP %d: %s", resp.StatusCode, string(responseBody))
		}
	}

	return signData.ResourceURL, nil
}

func httpGet(rawURL string, params map[string]string) ([]byte, error) {
	req, err := http.NewRequest(http.MethodGet, rawURL, nil)
	if err != nil {
		return nil, err
	}
	query := req.URL.Query()
	for key, value := range params {
		query.Add(key, value)
	}
	req.URL.RawQuery = query.Encode()
	req.Header.Set("Accept", "*/*")
	req.Header.Set("Origin", _signOrigin)
	req.Header.Set("Referer", _signOrigin+"/")
	req.Header.Set("User-Agent", "Gensokyo-NewQQ/imagehosting")

	resp, err := imageHostingHTTPClient.Do(req)
	if err != nil {
		return nil, err
	}
	body, err := readClose(resp)
	if err != nil {
		return nil, err
	}
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return nil, fmt.Errorf("签名服务返回 HTTP %d", resp.StatusCode)
	}
	return body, nil
}

func requireHTTPSURL(rawURL string) error {
	parsed, err := url.Parse(rawURL)
	if err != nil {
		return err
	}
	if parsed.Scheme != "https" || parsed.Host == "" || parsed.User != nil {
		return fmt.Errorf("仅允许不含用户信息的有效 HTTPS URL")
	}

	hostname := strings.TrimSuffix(strings.ToLower(parsed.Hostname()), ".")
	if hostname == "localhost" || strings.HasSuffix(hostname, ".localhost") {
		return fmt.Errorf("不允许本机地址")
	}
	if ip := net.ParseIP(hostname); ip != nil {
		if ip.IsLoopback() || ip.IsPrivate() || ip.IsUnspecified() || ip.IsLinkLocalUnicast() || ip.IsLinkLocalMulticast() {
			return fmt.Errorf("不允许私有、回环或链路本地 IP 地址")
		}
	}
	return nil
}

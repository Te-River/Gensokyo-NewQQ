// QQ频道图床 — 通过向频道发送图片消息获取 qpic.cn CDN 链接。
package imagehosting

import (
	"bytes"
	"crypto/md5"
	"encoding/hex"
	"fmt"
	"mime/multipart"
	"net/http"
	"net/url"
	"strings"

	"github.com/hoshinonyaruko/gensokyo/config"
)

func tryQQChannel(data []byte, filename string) (string, error) {
	cfg := config.GetImageHostingQQChannel()
	channelID := strings.TrimSpace(cfg.ChannelID)
	token := strings.TrimSpace(config.GetImageHostingQQChannelToken())
	if !cfg.Enabled || channelID == "" || token == "" {
		return "", fmt.Errorf("QQ频道未完整配置或未启用")
	}

	filename = ensureExt(filename, data)
	body := &bytes.Buffer{}
	writer := multipart.NewWriter(body)
	part, err := writer.CreateFormFile("file_image", filename)
	if err != nil {
		return "", fmt.Errorf("创建 form 失败: %w", err)
	}
	if _, err := part.Write(data); err != nil {
		return "", fmt.Errorf("写入图片失败: %w", err)
	}
	if err := writer.Close(); err != nil {
		return "", fmt.Errorf("关闭 multipart writer 失败: %w", err)
	}

	endpoint := fmt.Sprintf("https://api.sgroup.qq.com/channels/%s/messages", url.PathEscape(channelID))
	req, err := http.NewRequest(http.MethodPost, endpoint, body)
	if err != nil {
		return "", fmt.Errorf("创建请求失败: %w", err)
	}
	req.Header.Set("Content-Type", writer.FormDataContentType())
	req.Header.Set("Authorization", token)
	req.Header.Set("User-Agent", "Gensokyo-NewQQ/imagehosting")
	query := req.URL.Query()
	query.Add("msg_id", "1")
	req.URL.RawQuery = query.Encode()

	resp, err := imageHostingHTTPClient.Do(req)
	if err != nil {
		return "", fmt.Errorf("上传请求失败: %w", err)
	}
	responseBody, readErr := readClose(resp)
	if readErr != nil {
		return "", fmt.Errorf("读取 QQ频道响应失败: %w", readErr)
	}
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return "", fmt.Errorf("QQ频道返回 HTTP %d: %s", resp.StatusCode, string(responseBody))
	}

	md5hash := md5.Sum(data)
	md5str := strings.ToUpper(hex.EncodeToString(md5hash[:]))
	return fmt.Sprintf("https://gchat.qpic.cn/qmeetpic/0/0-0-%s/0", md5str), nil
}

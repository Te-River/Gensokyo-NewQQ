// QQ频道 图床 — 通过向频道发送图片消息获取 qpic.cn CDN 链接
// 需要配置 channel_id，Bot token 自动从 config 读取
package imagehosting

import (
	"bytes"
	"crypto/md5"
	"encoding/hex"
	"fmt"
	"mime/multipart"
	"net/http"
	"strings"

	"github.com/hoshinonyaruko/gensokyo/config"
)

func tryQQChannel(data []byte, filename string) (string, error) {
	cfg := config.GetImageHostingQQChannel()
	if cfg.ChannelID == "" {
		return "", fmt.Errorf("QQ频道 未配置（请填写 channel_id）")
	}

	_ = detectMIME(data)

	body := &bytes.Buffer{}
	writer := multipart.NewWriter(body)
	part, err := writer.CreateFormFile("file_image", filename)
	if err != nil {
		return "", fmt.Errorf("创建 form 失败: %w", err)
	}
	part.Write(data)
	writer.Close()

	// 用 config 中配置的 token
	token := config.GetImageHostingQQChannelToken()
	url := fmt.Sprintf("https://api.sgroup.qq.com/channels/%s/messages", cfg.ChannelID)
	req, err := http.NewRequest("POST", url, body)
	if err != nil {
		return "", fmt.Errorf("创建请求失败: %w", err)
	}
	req.Header.Set("Content-Type", writer.FormDataContentType())
	if token != "" {
		req.Header.Set("Authorization", token)
	}
	q := req.URL.Query()
	q.Add("msg_id", "1")
	req.URL.RawQuery = q.Encode()

	resp, err := providerHTTPClient.Do(req)
	if err != nil {
		return "", fmt.Errorf("上传请求失败: %w", err)
	}
	if _, readErr := readClose(resp); readErr != nil {
		return "", fmt.Errorf("读取 QQ 频道响应失败: %w", readErr)
	}

	md5hash := md5.Sum(data)
	md5str := strings.ToUpper(hex.EncodeToString(md5hash[:]))
	return fmt.Sprintf("https://gchat.qpic.cn/qmeetpic/0/0-0-%s/0", md5str), nil
}

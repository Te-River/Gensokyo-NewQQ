// Nature 图床 — 腾讯 COS 直传（密钥内置）
// 免费，无需配置，仅支持图片（PNG/JPG/WebP/GIF）
//
// 使用硬编码的腾讯 COS 密钥对指向 sgame-data-service-1252931805 存储桶。
// 如需自定义 COS，请使用 imagehosting/cos.go（需配置）。
package imagehosting

import (
	"bytes"
	"crypto/hmac"
	"crypto/sha1"
	"encoding/base64"
	"encoding/hex"
	"fmt"
	"hash"
	"net/http"
	"time"
)

// 硬编码的 Nature COS 密钥（从 Python 版移植）
var (
	_natureSecretID  = mustB64("QUtJRHJiOFRiZlhBWnJ5cVRzMnlnQlNWSkdzSFRROGR0d21O")
	_natureSecretKey = mustB64("UFphTnhLV2ZjTHAzNHJQanJ1dGtXRnlaQ2N5REdCMGQ=")
	_natureBucket    = "sgame-data-service-1252931805"
	_natureRegion    = "ap-nanjing"
	_natureCDN       = "https://download.nature.qq.com"
	_naturePrefix    = "SnsShare/SocialProfile"
)

func mustB64(s string) string {
	b, err := base64.StdEncoding.DecodeString(s)
	if err != nil {
		panic("imagehosting: base64 decode failed: " + err.Error())
	}
	return string(b)
}

func tryNature(data []byte, filename string) (string, error) {
	// 仅支持图片
	mime := detectMIME(data)
	ext := detectExt(data)
	if mime == "image/gif" {
		mime = "image/jpeg" // Nature 特殊处理
	}

	ts := time.Now().Unix()
	rand := fmt.Sprintf("%x", time.Now().UnixNano()%0x100000000)[:8]
	uploadPath := fmt.Sprintf("%s/%d_%s.%s", _naturePrefix, ts, rand, ext)
	host := fmt.Sprintf("%s.cos.%s.myqcloud.com", _natureBucket, _natureRegion)

	signTime := fmt.Sprintf("%d;%d", ts, ts+3600)
	signKey := hmacSha1N(_natureSecretKey, signTime)
	fmtStr := fmt.Sprintf("put\n/%s\n\nhost=%s\n", uploadPath, host)
	sts := fmt.Sprintf("sha1\n%s\n%s\n", signTime, sha1HexN(fmtStr))
	sig := hmacSha1N(signKey, sts)

	auth := fmt.Sprintf("q-sign-algorithm=sha1&q-ak=%s&q-sign-time=%s&q-key-time=%s&q-header-list=host&q-url-param-list=&q-signature=%s",
		_natureSecretID, signTime, signTime, sig)

	url := fmt.Sprintf("https://%s/%s", host, uploadPath)
	req, err := http.NewRequest("PUT", url, bytes.NewReader(data))
	if err != nil {
		return "", fmt.Errorf("创建请求失败: %w", err)
	}
	req.Header.Set("Host", host)
	req.Header.Set("Content-Type", mime)
	req.Header.Set("Authorization", auth)

	resp, err := providerHTTPClient.Do(req)
	if err != nil {
		return "", fmt.Errorf("上传请求失败: %w", err)
	}
	if _, readErr := readClose(resp); readErr != nil {
		return "", fmt.Errorf("读取 Nature 响应失败: %w", readErr)
	}

	if resp.StatusCode != 200 {
		return "", fmt.Errorf("Nature 返回 HTTP %d", resp.StatusCode)
	}

	return fmt.Sprintf("%s/%s", _natureCDN, uploadPath), nil
}

func hmacSha1N(key, data string) string {
	h := hmac.New(func() hash.Hash { return sha1.New() }, []byte(key))
	h.Write([]byte(data))
	return hex.EncodeToString(h.Sum(nil))
}

func sha1HexN(data string) string {
	h := sha1.New()
	h.Write([]byte(data))
	return hex.EncodeToString(h.Sum(nil))
}

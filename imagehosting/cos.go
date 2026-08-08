// COS 图床 — 腾讯云对象存储
// 需在配置中填写 secret_id / secret_key / region / bucket
//
// 采用 HMAC-SHA1 自签名直传，不依赖 COS SDK。
package imagehosting

import (
	"bytes"
	"crypto/hmac"
	"crypto/sha1"
	"encoding/hex"
	"fmt"
	"hash"
	"net/http"
	"time"

	"github.com/hoshinonyaruko/gensokyo/config"
)

func tryCOS(data []byte, filename string) (string, error) {
	cfg := config.GetImageHostingCOS()
	if cfg.SecretID == "" || cfg.SecretKey == "" || cfg.Bucket == "" || cfg.Region == "" {
		return "", fmt.Errorf("COS 未配置（请填写 secret_id / secret_key / region / bucket）")
	}

	ts := time.Now().Unix()
	key := fmt.Sprintf("gensokyo/%d/%s", ts, filename)
	host := fmt.Sprintf("%s.cos.%s.myqcloud.com", cfg.Bucket, cfg.Region)

	mime := detectMIME(data)
	signTime := fmt.Sprintf("%d;%d", ts, ts+3600)
	signKey := hmacSha1(cfg.SecretKey, signTime)
	fmtStr := fmt.Sprintf("put\n/%s\n\nhost=%s\n", key, host)
	sts := fmt.Sprintf("sha1\n%s\n%s\n", signTime, sha1Hex(fmtStr))
	sig := hmacSha1(signKey, sts)

	auth := fmt.Sprintf("q-sign-algorithm=sha1&q-ak=%s&q-sign-time=%s&q-key-time=%s&q-header-list=host&q-url-param-list=&q-signature=%s",
		cfg.SecretID, signTime, signTime, sig)

	url := fmt.Sprintf("https://%s/%s", host, key)
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
		return "", fmt.Errorf("读取 COS 响应失败: %w", readErr)
	}

	if resp.StatusCode != 200 {
		return "", fmt.Errorf("COS 返回 HTTP %d", resp.StatusCode)
	}

	domain := cfg.Domain
	if domain == "" {
		domain = fmt.Sprintf("https://%s", host)
	}
	return fmt.Sprintf("%s/%s", domain, key), nil
}

func hmacSha1(key, data string) string {
	h := hmac.New(func() hash.Hash { return sha1.New() }, []byte(key))
	h.Write([]byte(data))
	return hex.EncodeToString(h.Sum(nil))
}

func sha1Hex(data string) string {
	h := sha1.New()
	h.Write([]byte(data))
	return hex.EncodeToString(h.Sum(nil))
}


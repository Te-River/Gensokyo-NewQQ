// Nature 图床 — 腾讯 COS 直传（凭据配置注入）
// 需在配置中填写 secret_id / secret_key / region / bucket
//
// 采用 HMAC-SHA1 自签名直传，不依赖 COS SDK。
// 安全说明：云凭据必须通过配置注入，禁止硬编码在源码中。
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

// 非敏感默认路径前缀（凭据必须来自配置）
var _naturePrefix = "SnsShare/SocialProfile"

func tryNature(data []byte, filename string) (string, error) {
	cfg := config.GetImageHostingNature()
	if cfg.SecretID == "" || cfg.SecretKey == "" {
		return "", fmt.Errorf("Nature 未配置（请填写 secret_id / secret_key）")
	}
	if cfg.Bucket == "" || cfg.Region == "" {
		return "", fmt.Errorf("Nature 未配置（请填写 region / bucket）")
	}

	// 仅支持图片
	mime := detectMIME(data)
	ext := detectExt(data)
	if mime == "image/gif" {
		mime = "image/jpeg" // Nature 特殊处理
	}

	ts := time.Now().Unix()
	rand := fmt.Sprintf("%x", time.Now().UnixNano()%0x100000000)[:8]
	uploadPath := fmt.Sprintf("%s/%d_%s.%s", _naturePrefix, ts, rand, ext)
	host := fmt.Sprintf("%s.cos.%s.myqcloud.com", cfg.Bucket, cfg.Region)

	signTime := fmt.Sprintf("%d;%d", ts, ts+3600)
	signKey := hmacSha1N(cfg.SecretKey, signTime)
	fmtStr := fmt.Sprintf("put\n/%s\n\nhost=%s\n", uploadPath, host)
	sts := fmt.Sprintf("sha1\n%s\n%s\n", signTime, sha1HexN(fmtStr))
	sig := hmacSha1N(signKey, sts)

	auth := fmt.Sprintf("q-sign-algorithm=sha1&q-ak=%s&q-sign-time=%s&q-key-time=%s&q-header-list=host&q-url-param-list=&q-signature=%s",
		cfg.SecretID, signTime, signTime, sig)

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

	domain := cfg.Domain
	if domain == "" {
		domain = fmt.Sprintf("https://%s", host)
	}
	return fmt.Sprintf("%s/%s", domain, uploadPath), nil
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

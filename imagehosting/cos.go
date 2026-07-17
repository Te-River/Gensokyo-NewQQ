// COS 图床 — 腾讯云对象存储。
// 需在配置中填写 secret_id / secret_key / region / bucket。
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
	"net/url"
	"regexp"
	"strings"
	"time"

	"github.com/hoshinonyaruko/gensokyo/config"
)

var cosIdentifierPattern = regexp.MustCompile(`^[a-z0-9][a-z0-9-]{0,62}$`)

func tryCOS(data []byte, filename string) (string, error) {
	cfg := config.GetImageHostingCOS()
	secretID := strings.TrimSpace(cfg.SecretID)
	secretKey := strings.TrimSpace(cfg.SecretKey)
	bucket := strings.ToLower(strings.TrimSpace(cfg.Bucket))
	region := strings.ToLower(strings.TrimSpace(cfg.Region))
	if !cfg.Enabled || secretID == "" || secretKey == "" || bucket == "" || region == "" {
		return "", fmt.Errorf("COS 未配置或未启用")
	}
	if !cosIdentifierPattern.MatchString(bucket) || !cosIdentifierPattern.MatchString(region) {
		return "", fmt.Errorf("COS bucket 或 region 格式无效")
	}

	filename = safeCOSObjectName(ensureExt(filename, data))
	now := time.Now().UTC()
	key := fmt.Sprintf("gensokyo/%s/%d-%s", now.Format("20060102"), now.UnixNano(), filename)
	host := fmt.Sprintf("%s.cos.%s.myqcloud.com", bucket, region)

	mime := detectMIME(data)
	ts := now.Unix()
	signTime := fmt.Sprintf("%d;%d", ts, ts+3600)
	signKey := hmacSha1(secretKey, signTime)
	formatString := fmt.Sprintf("put\n/%s\n\nhost=%s\n", key, host)
	stringToSign := fmt.Sprintf("sha1\n%s\n%s\n", signTime, sha1Hex(formatString))
	signature := hmacSha1(signKey, stringToSign)

	authorization := fmt.Sprintf("q-sign-algorithm=sha1&q-ak=%s&q-sign-time=%s&q-key-time=%s&q-header-list=host&q-url-param-list=&q-signature=%s",
		secretID, signTime, signTime, signature)

	uploadURL := fmt.Sprintf("https://%s/%s", host, key)
	req, err := http.NewRequest(http.MethodPut, uploadURL, bytes.NewReader(data))
	if err != nil {
		return "", fmt.Errorf("创建请求失败: %w", err)
	}
	req.Header.Set("Host", host)
	req.Header.Set("Content-Type", mime)
	req.Header.Set("Authorization", authorization)
	req.Header.Set("User-Agent", "Gensokyo-NewQQ/imagehosting")

	resp, err := imageHostingHTTPClient.Do(req)
	if err != nil {
		return "", fmt.Errorf("上传请求失败: %w", err)
	}
	responseBody, readErr := readClose(resp)
	if readErr != nil {
		return "", fmt.Errorf("读取 COS 响应失败: %w", readErr)
	}
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return "", fmt.Errorf("COS 返回 HTTP %d: %s", resp.StatusCode, string(responseBody))
	}

	domain, err := normalizeCOSDomain(cfg.Domain, host)
	if err != nil {
		return "", err
	}
	return fmt.Sprintf("%s/%s", domain, key), nil
}

func safeCOSObjectName(filename string) string {
	filename = strings.TrimSpace(filename)
	if filename == "" {
		return "image"
	}
	var builder strings.Builder
	builder.Grow(len(filename))
	for _, char := range filename {
		switch {
		case char >= 'a' && char <= 'z', char >= 'A' && char <= 'Z', char >= '0' && char <= '9', char == '.', char == '-', char == '_':
			builder.WriteRune(char)
		default:
			builder.WriteByte('_')
		}
		if builder.Len() >= 120 {
			break
		}
	}
	name := strings.Trim(builder.String(), ".-_ ")
	if name == "" {
		return "image"
	}
	return name
}

func normalizeCOSDomain(configuredDomain, host string) (string, error) {
	domain := strings.TrimSpace(configuredDomain)
	if domain == "" {
		return "https://" + host, nil
	}
	if !strings.Contains(domain, "://") {
		domain = "https://" + domain
	}
	parsed, err := url.Parse(domain)
	if err != nil || parsed.Scheme != "https" || parsed.Host == "" || parsed.User != nil {
		return "", fmt.Errorf("COS 自定义域名必须是有效的 HTTPS URL")
	}
	if parsed.RawQuery != "" || parsed.Fragment != "" {
		return "", fmt.Errorf("COS 自定义域名不能包含查询参数或片段")
	}
	return strings.TrimRight(parsed.String(), "/"), nil
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

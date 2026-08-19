package media

import (
	"context"
	"crypto/tls"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"os"
	"strings"
	"time"
)

// FetcherOptions SafeHTTPFetcher 的配置。
type FetcherOptions struct {
	Timeout time.Duration
	// AllowPrivate 允许访问私网/回环地址（仅测试或受控内网环境开启；默认拒绝 SSRF）。
	AllowPrivate bool
	// MaxRedirects 重定向次数上限。
	MaxRedirects int
}

// Fetcher 安全的 HTTP 下载器：timeout / max bytes / 重定向限制 / SSRF 检查 / 状态码 / Content-Type / 文件签名。
type Fetcher struct {
	client        *http.Client
	allowPrivate  bool
	maxRedirects  int
}

// NewFetcher 创建安全下载器。
func NewFetcher(opts FetcherOptions) *Fetcher {
	if opts.Timeout <= 0 {
		opts.Timeout = 30 * time.Second
	}
	if opts.MaxRedirects <= 0 {
		opts.MaxRedirects = 5
	}
	f := &Fetcher{
		allowPrivate: opts.AllowPrivate,
		maxRedirects: opts.MaxRedirects,
	}
	f.client = &http.Client{
		Timeout: opts.Timeout,
		CheckRedirect: func(req *http.Request, via []*http.Request) error {
			if len(via) >= f.maxRedirects {
				return fmt.Errorf("media: too many redirects")
			}
			if !f.allowPrivate {
				if err := f.checkSSRF(req.URL); err != nil {
					return err
				}
			}
			return nil
		},
		Transport: &http.Transport{
			TLSClientConfig: &tls.Config{MinVersion: tls.VersionTLS12}, //nolint:gosec // 仅限制最低版本
		},
	}
	return f
}

// Fetch 下载 URL 到内存，限制 maxBytes。
func (f *Fetcher) Fetch(ctx context.Context, rawURL string, maxBytes int64) ([]byte, string, error) {
	data, err := f.fetchBody(ctx, rawURL, maxBytes, false)
	return data, http.DetectContentType(data), err
}

// FetchToTempFile 下载 URL 到临时文件（大媒体避免内存翻倍）。
// 当内容超过 threshold 时流式写入临时文件，否则写内存并返回 nil path。
func (f *Fetcher) FetchToTempFile(ctx context.Context, rawURL string, maxBytes, threshold int64) (string, string, int64, error) {
	req, err := f.newRequest(ctx, rawURL)
	if err != nil {
		return "", "", 0, err
	}
	resp, err := f.client.Do(req)
	if err != nil {
		return "", "", 0, err
	}
	defer resp.Body.Close()

	if err := f.checkResponse(resp); err != nil {
		return "", "", 0, err
	}

	if maxBytes > 0 && resp.ContentLength > maxBytes {
		return "", "", 0, fmt.Errorf("media: content length %d exceeds limit %d", resp.ContentLength, maxBytes)
	}

	// 小内容走内存
	if threshold > 0 && resp.ContentLength >= 0 && resp.ContentLength <= threshold {
		data, err := io.ReadAll(io.LimitReader(resp.Body, maxBytes+1))
		if err != nil {
			return "", "", 0, err
		}
		if maxBytes > 0 && int64(len(data)) > maxBytes {
			return "", "", 0, fmt.Errorf("media: content exceeds limit %d", maxBytes)
		}
		return "", http.DetectContentType(data), int64(len(data)), nil
	}

	// 大内容流式写临时文件
	tmp, err := os.CreateTemp("", "gensokyo-media-*")
	if err != nil {
		return "", "", 0, err
	}
	written, err := io.Copy(tmp, io.LimitReader(resp.Body, maxBytes+1))
	if err != nil {
		tmp.Close()
		os.Remove(tmp.Name())
		return "", "", 0, err
	}
	if maxBytes > 0 && written > maxBytes {
		tmp.Close()
		os.Remove(tmp.Name())
		return "", "", 0, fmt.Errorf("media: content exceeds limit %d", maxBytes)
	}
	// 读取前 512 字节做签名嗅探
	mime := "application/octet-stream"
	if _, err := tmp.Seek(0, io.SeekStart); err == nil {
		head := make([]byte, 512)
		n, _ := tmp.Read(head)
		mime = http.DetectContentType(head[:n])
	}
	if err := tmp.Close(); err != nil {
		os.Remove(tmp.Name())
		return "", "", 0, err
	}
	return tmp.Name(), mime, written, nil
}

func (f *Fetcher) fetchBody(ctx context.Context, rawURL string, maxBytes int64, allowTemp bool) ([]byte, error) {
	req, err := f.newRequest(ctx, rawURL)
	if err != nil {
		return nil, err
	}
	resp, err := f.client.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()

	if err := f.checkResponse(resp); err != nil {
		return nil, err
	}
	if maxBytes > 0 && resp.ContentLength > maxBytes {
		return nil, fmt.Errorf("media: content length %d exceeds limit %d", resp.ContentLength, maxBytes)
	}
	data, err := io.ReadAll(io.LimitReader(resp.Body, maxBytes+1))
	if err != nil {
		return nil, err
	}
	if maxBytes > 0 && int64(len(data)) > maxBytes {
		return nil, fmt.Errorf("media: content exceeds limit %d", maxBytes)
	}
	return data, nil
}

func (f *Fetcher) newRequest(ctx context.Context, rawURL string) (*http.Request, error) {
	u, err := url.Parse(rawURL)
	if err != nil {
		return nil, fmt.Errorf("media: invalid URL: %w", err)
	}
	if u.Scheme != "http" && u.Scheme != "https" {
		return nil, fmt.Errorf("media: unsupported scheme %q", u.Scheme)
	}
	if !f.allowPrivate {
		if err := f.checkSSRF(u); err != nil {
			return nil, err
		}
	}
	return http.NewRequestWithContext(ctx, http.MethodGet, rawURL, nil)
}

func (f *Fetcher) checkResponse(resp *http.Response) error {
	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("media: HTTP status %d", resp.StatusCode)
	}
	return nil
}

// checkSSRF 拒绝私网/回环/链路本地地址。
func (f *Fetcher) checkSSRF(u *url.URL) error {
	host := u.Hostname()
	if host == "" {
		return fmt.Errorf("media: empty host")
	}
	ips, err := net.LookupIP(host)
	if err != nil {
		return fmt.Errorf("media: DNS lookup failed: %w", err)
	}
	for _, ip := range ips {
		if isBlockedIP(ip) {
			return fmt.Errorf("media: blocked address %s (SSRF guard)", ip)
		}
	}
	return nil
}

func isBlockedIP(ip net.IP) bool {
	if ip.IsLoopback() || ip.IsPrivate() || ip.IsLinkLocalUnicast() ||
		ip.IsLinkLocalMulticast() || ip.IsUnspecified() {
		return true
	}
	// IPv4-mapped IPv6
	if v4 := ip.To4(); v4 != nil {
		return isBlockedIP(v4)
	}
	return strings.HasPrefix(ip.String(), "fc") || strings.HasPrefix(ip.String(), "fd")
}

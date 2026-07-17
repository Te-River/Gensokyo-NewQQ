package imagehosting

import (
	"net/http"
	"net/url"
	"strings"
	"testing"
	"time"
)

func TestImageHostingHTTPClientHasTimeout(t *testing.T) {
	if imageHostingHTTPClient.Timeout != 15*time.Second {
		t.Fatalf("HTTP client timeout = %v", imageHostingHTTPClient.Timeout)
	}
	if imageHostingHTTPClient.CheckRedirect == nil {
		t.Fatal("HTTP client redirect policy is not configured")
	}
}

func TestRequireHTTPSURL(t *testing.T) {
	for _, rawURL := range []string{
		"https://example.com/upload",
		"https://example.com/upload?signature=value",
	} {
		if err := requireHTTPSURL(rawURL); err != nil {
			t.Fatalf("valid HTTPS URL rejected: %s: %v", rawURL, err)
		}
	}
	for _, rawURL := range []string{
		"http://example.com/upload",
		"//example.com/upload",
		"/relative/upload",
		"not-a-url",
		"https://localhost/upload",
		"https://api.localhost/upload",
		"https://127.0.0.1/upload",
		"https://10.0.0.1/upload",
		"https://169.254.1.1/upload",
		"https://[::1]/upload",
		"https://user:pass@example.com/upload",
	} {
		if err := requireHTTPSURL(rawURL); err == nil {
			t.Fatalf("unsafe URL accepted: %s", rawURL)
		}
	}
}

func TestImageHostRedirectPolicy(t *testing.T) {
	makeRequest := func(rawURL string) *http.Request {
		parsed, err := url.Parse(rawURL)
		if err != nil {
			t.Fatalf("parse URL %s: %v", rawURL, err)
		}
		return &http.Request{URL: parsed}
	}

	if err := imageHostingHTTPClient.CheckRedirect(makeRequest("https://cdn.example.com/image.png"), nil); err != nil {
		t.Fatalf("safe redirect rejected: %v", err)
	}
	if err := imageHostingHTTPClient.CheckRedirect(makeRequest("https://127.0.0.1/image.png"), nil); err == nil {
		t.Fatal("private redirect target was accepted")
	}

	via := make([]*http.Request, maxImageHostRedirects)
	for index := range via {
		via[index] = makeRequest("https://example.com/previous")
	}
	if err := imageHostingHTTPClient.CheckRedirect(makeRequest("https://example.com/next"), via); err == nil {
		t.Fatal("excessive redirect chain was accepted")
	}
}

func TestSafeCOSObjectName(t *testing.T) {
	name := safeCOSObjectName("../../图像 test.png")
	if name == "" || strings.ContainsAny(name, "/\\ ") {
		t.Fatalf("unsafe object name: %q", name)
	}
	if len(name) > 120 {
		t.Fatalf("object name too long: %d", len(name))
	}
	if got := safeCOSObjectName("..."); got != "image" {
		t.Fatalf("empty sanitized name = %q", got)
	}
}

func TestNormalizeCOSDomain(t *testing.T) {
	got, err := normalizeCOSDomain("", "bucket.cos.ap-guangzhou.myqcloud.com")
	if err != nil || got != "https://bucket.cos.ap-guangzhou.myqcloud.com" {
		t.Fatalf("default domain = %q, %v", got, err)
	}
	got, err = normalizeCOSDomain("cdn.example.com/base/", "unused")
	if err != nil || got != "https://cdn.example.com/base" {
		t.Fatalf("custom domain = %q, %v", got, err)
	}
	for _, domain := range []string{
		"http://cdn.example.com",
		"https://user:pass@cdn.example.com",
		"https://cdn.example.com?token=value",
		"https://cdn.example.com/#fragment",
	} {
		if _, err := normalizeCOSDomain(domain, "unused"); err == nil {
			t.Fatalf("unsafe COS domain accepted: %s", domain)
		}
	}
}

func TestCOSIdentifierPattern(t *testing.T) {
	for _, value := range []string{"bucket-123456", "ap-guangzhou"} {
		if !cosIdentifierPattern.MatchString(value) {
			t.Fatalf("valid COS identifier rejected: %s", value)
		}
	}
	for _, value := range []string{"", "../bucket", "bucket.example.com", "UPPERCASE", "-prefix"} {
		if cosIdentifierPattern.MatchString(value) {
			t.Fatalf("unsafe COS identifier accepted: %s", value)
		}
	}
}

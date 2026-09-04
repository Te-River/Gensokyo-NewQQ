package imagehosting

import (
	"bytes"
	"errors"
	"fmt"
	"io"
	"net/http"
	"testing"
	"testing/synctest"
	"time"
)

func TestReadCloseRejectsOversizedResponse(t *testing.T) {
	resp := &http.Response{
		Body: io.NopCloser(bytes.NewReader(bytes.Repeat([]byte("x"), int(maxProviderResponseBytes)+1))),
	}
	if _, err := readClose(resp); err == nil {
		t.Fatal("readClose accepted an oversized response body")
	}
}

func TestProviderHTTPClientHasTimeout(t *testing.T) {
	if providerHTTPClient.Timeout != 30*time.Second {
		t.Fatalf("provider HTTP timeout = %s, want 30s", providerHTTPClient.Timeout)
	}
}

func TestRetryUploadReturnsOnFirstSuccess(t *testing.T) {
	calls := 0
	got, err := retryUpload(func() (string, error) {
		calls++
		return "url-1", nil
	})
	if err != nil {
		t.Fatalf("retryUpload returned error on first success: %v", err)
	}
	if got != "url-1" {
		t.Fatalf("retryUpload result = %q, want %q", got, "url-1")
	}
	if calls != 1 {
		t.Fatalf("upload attempts = %d, want 1", calls)
	}
}

func TestRetryUploadSucceedsAfterRetries(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		calls := 0
		got, err := retryUpload(func() (string, error) {
			calls++
			if calls < 3 {
				return "", fmt.Errorf("attempt %d failed", calls)
			}
			return "url-3", nil
		})
		if err != nil {
			t.Fatalf("retryUpload returned error despite eventual success: %v", err)
		}
		if got != "url-3" {
			t.Fatalf("retryUpload result = %q, want %q", got, "url-3")
		}
		if calls != 3 {
			t.Fatalf("upload attempts = %d, want 3", calls)
		}
	})
}

func TestRetryUploadStopsAfterThreeAttempts(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		calls := 0
		sentinel := errors.New("provider down")
		_, err := retryUpload(func() (string, error) {
			calls++
			return "", sentinel
		})
		if calls != maxUploadAttempts {
			t.Fatalf("upload attempts = %d, want %d", calls, maxUploadAttempts)
		}
		if !errors.Is(err, sentinel) {
			t.Fatalf("retryUpload error = %v, want sentinel %v", err, sentinel)
		}
	})
}

// TestRetryUploadBackoffSchedule 用 synctest 虚拟时钟验证 1s/2s 线性退避，瞬时且确定性。
func TestRetryUploadBackoffSchedule(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		var attempts []time.Time
		_, _ = retryUpload(func() (string, error) {
			attempts = append(attempts, time.Now())
			return "", errors.New("always fails")
		})
		if len(attempts) != maxUploadAttempts {
			t.Fatalf("upload attempts = %d, want %d", len(attempts), maxUploadAttempts)
		}
		if gap := attempts[1].Sub(attempts[0]); gap != 1*time.Second {
			t.Fatalf("backoff before 2nd attempt = %s, want 1s", gap)
		}
		if gap := attempts[2].Sub(attempts[1]); gap != 2*time.Second {
			t.Fatalf("backoff before 3rd attempt = %s, want 2s", gap)
		}
	})
}

// TestUploadProviderUnknownProviderFailsWithoutRetry 未知 provider 应立即报错，不进入重试。
func TestUploadProviderUnknownProviderFailsWithoutRetry(t *testing.T) {
	_, err := UploadProvider("foo", []byte("img"), "img.png")
	if err == nil {
		t.Fatal("UploadProvider with unknown provider returned nil error")
	}
	want := "未知或不支持的图床 provider: foo"
	if err.Error() != want {
		t.Fatalf("UploadProvider error = %q, want %q", err.Error(), want)
	}
}

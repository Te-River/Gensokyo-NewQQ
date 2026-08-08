package imagehosting

import (
	"bytes"
	"io"
	"net/http"
	"testing"
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

func TestTryNatureFailsClosedWithoutCredentials(t *testing.T) {
	// 凭据缺失时必须 fail closed，禁止回退到任何内置凭据
	if _, err := tryNature([]byte("fake-image-bytes"), "test.png"); err == nil {
		t.Fatal("tryNature succeeded without credentials; must fail closed")
	}
}

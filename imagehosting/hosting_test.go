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

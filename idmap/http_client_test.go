package idmap

import (
	"bytes"
	"io"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"
)

func TestIDMapHTTPClientLimitsAndTimesOut(t *testing.T) {
	if idmapHTTPClient.Timeout != 30*time.Second {
		t.Fatalf("HTTP timeout = %s, want 30s", idmapHTTPClient.Timeout)
	}

	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write(bytes.Repeat([]byte("x"), int(maxIDMapResponseBytes)+1))
	}))
	defer server.Close()

	resp, err := idmapHTTPClient.Get(server.URL)
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	body, err := io.ReadAll(resp.Body)
	if err != nil {
		t.Fatal(err)
	}
	if int64(len(body)) != maxIDMapResponseBytes+1 {
		t.Fatalf("limited body length = %d, want %d", len(body), maxIDMapResponseBytes+1)
	}
}

package media

import (
	"bytes"
	"context"
	"net/http"
	"net/http/httptest"
	"os"
	"strings"
	"testing"
)

func testServer(t *testing.T) *httptest.Server {
	t.Helper()
	return httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Write([]byte("fake-image-data"))
	}))
}

func TestFetcherRejectsPrivateByDefault(t *testing.T) {
	srv := testServer(t)
	defer srv.Close()

	f := NewFetcher(FetcherOptions{})
	_, _, err := f.Fetch(context.Background(), srv.URL, 1<<20)
	if err == nil {
		t.Fatal("Fetcher allowed loopback address (SSRF guard failed)")
	}
}

func TestFetcherAllowPrivate(t *testing.T) {
	srv := testServer(t)
	defer srv.Close()

	f := NewFetcher(FetcherOptions{AllowPrivate: true})
	data, _, err := f.Fetch(context.Background(), srv.URL, 1<<20)
	if err != nil {
		t.Fatalf("Fetch: %v", err)
	}
	if string(data) != "fake-image-data" {
		t.Fatalf("data = %q", data)
	}
}

func TestFetcherRejectsOversize(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Write(bytes.Repeat([]byte("x"), 1024))
	}))
	defer srv.Close()

	f := NewFetcher(FetcherOptions{AllowPrivate: true})
	_, _, err := f.Fetch(context.Background(), srv.URL, 100)
	if err == nil || !strings.Contains(err.Error(), "exceeds limit") {
		t.Fatalf("err = %v, want size limit error", err)
	}
}

func TestFetcherRejectsBadStatus(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusNotFound)
	}))
	defer srv.Close()

	f := NewFetcher(FetcherOptions{AllowPrivate: true})
	_, _, err := f.Fetch(context.Background(), srv.URL, 1<<20)
	if err == nil || !strings.Contains(err.Error(), "HTTP status 404") {
		t.Fatalf("err = %v, want 404 error", err)
	}
}

func TestFetcherRejectsUnsupportedScheme(t *testing.T) {
	f := NewFetcher(FetcherOptions{AllowPrivate: true})
	if _, _, err := f.Fetch(context.Background(), "ftp://x.com/a", 100); err == nil {
		t.Fatal("accepted ftp scheme")
	}
}

func TestFetcherRedirectLimit(t *testing.T) {
	// 循环重定向
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		http.Redirect(w, r, r.URL.String(), http.StatusFound)
	}))
	defer srv.Close()

	f := NewFetcher(FetcherOptions{AllowPrivate: true, MaxRedirects: 3})
	_, _, err := f.Fetch(context.Background(), srv.URL, 1<<20)
	if err == nil || !strings.Contains(err.Error(), "too many redirects") {
		t.Fatalf("err = %v, want redirect limit", err)
	}
}

func TestFetcherToTempFileAndCleanup(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Write(bytes.Repeat([]byte("y"), 2048))
	}))
	defer srv.Close()

	f := NewFetcher(FetcherOptions{AllowPrivate: true})
	path, mime, size, err := f.FetchToTempFile(context.Background(), srv.URL, 1<<20, 1024)
	if err != nil {
		t.Fatalf("FetchToTempFile: %v", err)
	}
	if path == "" || size != 2048 {
		t.Fatalf("path=%q size=%d", path, size)
	}
	if mime == "" {
		t.Fatal("mime empty")
	}
	p := newPreparedFromTempFile(path, mime, size)
	if err := p.Close(); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(path); !os.IsNotExist(err) {
		t.Fatal("temp file not removed after Close")
	}
}

func TestFetcherToTempFileSmallInMemory(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Write([]byte("small"))
	}))
	defer srv.Close()

	f := NewFetcher(FetcherOptions{AllowPrivate: true})
	path, _, size, err := f.FetchToTempFile(context.Background(), srv.URL, 1<<20, 1024)
	if err != nil {
		t.Fatalf("FetchToTempFile: %v", err)
	}
	if path != "" || size != 5 {
		t.Fatalf("path=%q size=%d, want in-memory", path, size)
	}
}

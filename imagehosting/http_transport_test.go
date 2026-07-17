package imagehosting

import (
	"errors"
	"io"
	"net/http"
	"strings"
	"testing"
	"time"
)

type roundTripFunc func(*http.Request) (*http.Response, error)

func (fn roundTripFunc) RoundTrip(req *http.Request) (*http.Response, error) {
	return fn(req)
}

func useTestHTTPClient(t *testing.T, transport roundTripFunc) {
	t.Helper()
	original := imageHostingHTTPClient
	imageHostingHTTPClient = &http.Client{
		Transport: transport,
		Timeout:   time.Second,
	}
	t.Cleanup(func() {
		imageHostingHTTPClient = original
	})
}

func response(status int, body string) *http.Response {
	return &http.Response{
		StatusCode: status,
		Header:     make(http.Header),
		Body:       io.NopCloser(strings.NewReader(body)),
	}
}

func TestChatGLMValidatesReturnedURL(t *testing.T) {
	useTestHTTPClient(t, func(req *http.Request) (*http.Response, error) {
		if req.URL.Host != "chatglm.cn" {
			t.Fatalf("unexpected request host: %s", req.URL.Host)
		}
		return response(http.StatusOK, `{"result":{"file_url":"https://cdn.example.com/image.png"}}`), nil
	})

	got, err := tryChatGLM(makeTestPNG(t), "image.png")
	if err != nil {
		t.Fatalf("tryChatGLM failed: %v", err)
	}
	if got != "https://cdn.example.com/image.png" {
		t.Fatalf("tryChatGLM URL = %q", got)
	}
}

func TestChatGLMRejectsPrivateReturnedURL(t *testing.T) {
	useTestHTTPClient(t, func(_ *http.Request) (*http.Response, error) {
		return response(http.StatusOK, `{"result":{"file_url":"https://127.0.0.1/image.png"}}`), nil
	})

	if _, err := tryChatGLM(makeTestPNG(t), "image.png"); err == nil {
		t.Fatal("private ChatGLM result URL was accepted")
	}
}

func TestSignedUploadUsesBoundedClient(t *testing.T) {
	requests := 0
	useTestHTTPClient(t, func(req *http.Request) (*http.Response, error) {
		requests++
		switch requests {
		case 1:
			if req.URL.Host != "bed-sign.vercel.0013107.xyz" {
				t.Fatalf("unexpected sign host: %s", req.URL.Host)
			}
			if req.URL.Query().Get("module") != "xingye" {
				t.Fatalf("missing module query: %s", req.URL.RawQuery)
			}
			return response(http.StatusOK, `{"url":"https://upload.example.com/object","resourceUrl":"https://cdn.example.com/object.png","header":{"Content-Type":"image/png"}}`), nil
		case 2:
			if req.Method != http.MethodPut || req.URL.Host != "upload.example.com" {
				t.Fatalf("unexpected upload request: %s %s", req.Method, req.URL.String())
			}
			return response(http.StatusNoContent, ""), nil
		default:
			t.Fatalf("unexpected request count: %d", requests)
			return nil, errors.New("unexpected request")
		}
	})

	got, err := signedUpload(makeTestPNG(t), "image.png", "xingye")
	if err != nil {
		t.Fatalf("signedUpload failed: %v", err)
	}
	if got != "https://cdn.example.com/object.png" {
		t.Fatalf("resource URL = %q", got)
	}
	if requests != 2 {
		t.Fatalf("request count = %d", requests)
	}
}

func TestSignedUploadRejectsPrivateTargetBeforeUpload(t *testing.T) {
	requests := 0
	useTestHTTPClient(t, func(_ *http.Request) (*http.Response, error) {
		requests++
		return response(http.StatusOK, `{"url":"https://10.0.0.1/upload","resourceUrl":"https://cdn.example.com/object.png"}`), nil
	})

	if _, err := signedUpload(makeTestPNG(t), "image.png", "xingye"); err == nil {
		t.Fatal("private signed upload target was accepted")
	}
	if requests != 1 {
		t.Fatalf("upload request occurred after validation failure: %d", requests)
	}
}

func TestHTTPGetRejectsNonSuccessStatus(t *testing.T) {
	useTestHTTPClient(t, func(_ *http.Request) (*http.Response, error) {
		return response(http.StatusBadGateway, "upstream failed"), nil
	})

	if _, err := httpGet("https://example.com/sign", nil); err == nil {
		t.Fatal("non-success response was accepted")
	}
}

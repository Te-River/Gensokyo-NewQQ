package idmap

import (
	"io"
	"net/http"
	"time"
)

const maxIDMapResponseBytes int64 = 4 << 20

type limitedReadCloser struct {
	io.Reader
	closer io.Closer
}

func (r *limitedReadCloser) Close() error {
	return r.closer.Close()
}

type limitedResponseTransport struct {
	base http.RoundTripper
}

func (t limitedResponseTransport) RoundTrip(req *http.Request) (*http.Response, error) {
	base := t.base
	if base == nil {
		base = http.DefaultTransport
	}
	resp, err := base.RoundTrip(req)
	if err != nil || resp == nil || resp.Body == nil {
		return resp, err
	}
	resp.Body = &limitedReadCloser{
		Reader: io.LimitReader(resp.Body, maxIDMapResponseBytes+1),
		closer: resp.Body,
	}
	return resp, nil
}

var idmapHTTPClient = &http.Client{
	Timeout:   30 * time.Second,
	Transport: limitedResponseTransport{base: http.DefaultTransport},
}

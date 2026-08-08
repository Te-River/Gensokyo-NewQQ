package server

import (
	"net/http"
	"time"
)

// MaxWebhookBodyBytes bounds request bodies before signature parsing and queueing.
const MaxWebhookBodyBytes int64 = 4 << 20

// NewHTTPServer applies the same resource limits to every public HTTP listener.
func NewHTTPServer(addr string, handler http.Handler) *http.Server {
	return &http.Server{
		Addr:              addr,
		Handler:           handler,
		ReadHeaderTimeout: 10 * time.Second,
		ReadTimeout:       30 * time.Second,
		WriteTimeout:      60 * time.Second,
		IdleTimeout:       120 * time.Second,
		MaxHeaderBytes:    1 << 20,
	}
}

package main

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"

	"github.com/gin-gonic/gin"
	"github.com/hoshinonyaruko/gensokyo/mylog"
)

func TestMetricsEndpoint(t *testing.T) {
	// Set gin to test mode
	gin.SetMode(gin.TestMode)

	// Increment some metrics
	atomic.StoreUint64(&mylog.MetricMsgReceived, 42)
	atomic.StoreUint64(&mylog.MetricMsgSent, 13)
	atomic.StoreUint64(&mylog.MetricErrorCount, 7)
	atomic.StoreUint64(&mylog.MetricSlowEvents, 2)

	// Create test gin engine
	r := gin.New()
	r.GET("/metrics", MetricsHandler)

	// Perform test request
	req, _ := http.NewRequest("GET", "/metrics", nil)
	w := httptest.NewRecorder()
	r.ServeHTTP(w, req)

	// Check status
	if w.Code != http.StatusOK {
		t.Fatalf("expected status 200, got %d", w.Code)
	}

	body := w.Body.String()

	// Assert body contains Prometheus metric definitions and values
	expectedMetrics := []string{
		"gensokyo_uptime_seconds",
		"gensokyo_messages_received_total 42",
		"gensokyo_messages_sent_total 13",
		"gensokyo_errors_total 7",
		"gensokyo_slow_events_total 2",
		"gensokyo_memory_allocated_bytes",
	}

	for _, expected := range expectedMetrics {
		if !strings.Contains(body, expected) {
			t.Errorf("expected metric %q to be present in response: %s", expected, body)
		}
	}
}

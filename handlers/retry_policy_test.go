package handlers

import (
	"context"
	"errors"
	"testing"
	"time"
)

func TestClassifyQQError(t *testing.T) {
	if !IsQQRateLimited(errors.New(`request failed: {"code":22009}`)) {
		t.Fatal("22009 was not classified as rate limited")
	}
	if !IsQQEventExpired(errors.New(`request failed: {"code":40034026}`)) {
		t.Fatal("40034026 was not classified as event expired")
	}
	if !IsDeliveryTimeout(context.DeadlineExceeded) {
		t.Fatal("context deadline was not classified as timeout")
	}
	if IsDeliveryTimeout(errors.New("permanent validation error")) {
		t.Fatal("permanent error was classified as timeout")
	}
}

func TestRetryPolicy(t *testing.T) {
	if !defaultRetryPolicy.ShouldRetry(context.DeadlineExceeded, 0) {
		t.Fatal("first timeout attempt should be retried")
	}
	if defaultRetryPolicy.ShouldRetry(context.DeadlineExceeded, 2) {
		t.Fatal("last attempt should not be retried")
	}
	if got := defaultRetryPolicy.Backoff(2); got != 2*time.Second {
		t.Fatalf("backoff = %s, want 2s", got)
	}
}

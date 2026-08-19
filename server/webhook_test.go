package server

import (
	"bytes"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/gin-gonic/gin"
)

func TestCreateHandleValidationSafeRejectsOversizedBody(t *testing.T) {
	gin.SetMode(gin.TestMode)
	router := gin.New()
	router.POST("/webhook", CreateHandleValidationSafe(NewWebhookHandler(1)))

	body := bytes.Repeat([]byte("x"), int(MaxWebhookBodyBytes)+1)
	req := httptest.NewRequest(http.MethodPost, "/webhook", bytes.NewReader(body))
	recorder := httptest.NewRecorder()
	router.ServeHTTP(recorder, req)

	if recorder.Code != http.StatusRequestEntityTooLarge {
		t.Fatalf("status = %d, want %d", recorder.Code, http.StatusRequestEntityTooLarge)
	}
}

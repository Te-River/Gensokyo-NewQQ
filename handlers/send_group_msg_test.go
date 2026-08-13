package handlers

import (
	"context"
	"errors"
	"testing"
	"time"

	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

// mockGroupOpenAPI 是仅覆盖 PostGroupMessage 的 openapi.OpenAPI 桩，
// 用于直接驱动重试分支，避免构造完整 SDK 客户端。
type mockGroupOpenAPI struct {
	openapi.OpenAPI // 嵌入接口兜底，未覆盖的方法不会被调用
	calls          int
	seq            []error // 按调用次序返回的错误，耗尽后视为成功
	lastGroupID    string
	lastMsg        dto.APIMessage
}

func (m *mockGroupOpenAPI) PostGroupMessage(ctx context.Context, groupID string, msg dto.APIMessage) (*dto.GroupMessageResponse, error) {
	m.calls++
	m.lastGroupID = groupID
	m.lastMsg = msg
	if m.calls <= len(m.seq) && m.seq[m.calls-1] != nil {
		return nil, m.seq[m.calls-1]
	}
	return &dto.GroupMessageResponse{
		Message:       &dto.Message{ID: "mock-msg-id"},
		MediaResponse: &dto.MediaResponse{Ret: 0},
	}, nil
}

// withZeroBackoff 临时将重试退避改为 0，避免测试期间真实 sleep 1s/2s
func withZeroBackoff(t *testing.T) {
	t.Helper()
	orig := defaultRetryPolicy
	defaultRetryPolicy = RetryPolicy{MaxAttempts: 3, Backoff: func(attempt int) time.Duration { return 0 }}
	t.Cleanup(func() { defaultRetryPolicy = orig })
}

func TestPostGroupMessageWithRetry(t *testing.T) {
	withZeroBackoff(t)

	t.Run("超时后重试成功", func(t *testing.T) {
		mock := &mockGroupOpenAPI{seq: []error{context.DeadlineExceeded, context.DeadlineExceeded}}
		msg := &dto.MessageToCreate{Content: "hello", MsgID: "mid-1"}

		resp, err := postGroupMessageWithRetry(mock, "group-1", msg)

		if err != nil {
			t.Fatalf("期望最终成功，got err: %v", err)
		}
		if resp == nil || resp.Message.ID != "mock-msg-id" {
			t.Fatalf("响应异常: %+v", resp)
		}
		if mock.calls != 3 {
			t.Errorf("调用次数 = %d, want 3", mock.calls)
		}
		if mock.lastGroupID != "group-1" {
			t.Errorf("groupID = %q, want %q", mock.lastGroupID, "group-1")
		}
	})

	t.Run("非超时错误不重试", func(t *testing.T) {
		mock := &mockGroupOpenAPI{seq: []error{errors.New(`request failed: {"code":22009}`)}}

		_, err := postGroupMessageWithRetry(mock, "group-1", &dto.MessageToCreate{Content: "x"})

		if err == nil {
			t.Fatal("期望返回限流错误")
		}
		if !IsQQRateLimited(err) {
			t.Errorf("错误未被分类为限流: %v", err)
		}
		if mock.calls != 1 {
			t.Errorf("调用次数 = %d, want 1（非超时错误不应重试）", mock.calls)
		}
	})

	t.Run("持续超时耗尽重试次数", func(t *testing.T) {
		mock := &mockGroupOpenAPI{seq: []error{context.DeadlineExceeded, context.DeadlineExceeded, context.DeadlineExceeded}}

		_, err := postGroupMessageWithRetry(mock, "group-1", &dto.MessageToCreate{Content: "x"})

		if !IsDeliveryTimeout(err) {
			t.Errorf("期望最终仍为超时错误，got: %v", err)
		}
		if mock.calls != 3 {
			t.Errorf("调用次数 = %d, want 3", mock.calls)
		}
	})
}

func TestPostGroupRichMediaMessageWithRetry(t *testing.T) {
	withZeroBackoff(t)

	t.Run("超时后重试成功且清空EventID", func(t *testing.T) {
		mock := &mockGroupOpenAPI{seq: []error{context.DeadlineExceeded}}
		rm := &dto.RichMediaMessage{EventID: "evt-1", FileType: 1, URL: "https://x.com/a.png"}

		resp, err := postGroupRichMediaMessageWithRetry(mock, "group-1", rm)

		if err != nil {
			t.Fatalf("期望最终成功，got err: %v", err)
		}
		if resp == nil || resp.Message.ID != "mock-msg-id" {
			t.Fatalf("响应异常: %+v", resp)
		}
		if mock.calls != 2 {
			t.Errorf("调用次数 = %d, want 2", mock.calls)
		}
		if rm.EventID != "" {
			t.Errorf("EventID 未被清空: %q", rm.EventID)
		}
		if last, ok := mock.lastMsg.(*dto.RichMediaMessage); !ok || last.EventID != "" {
			t.Errorf("重试请求仍携带 EventID: %+v", mock.lastMsg)
		}
	})

	t.Run("非超时错误不重试", func(t *testing.T) {
		mock := &mockGroupOpenAPI{seq: []error{errors.New(`request failed: {"code":40034026}`)}}

		_, err := postGroupRichMediaMessageWithRetry(mock, "group-1", &dto.RichMediaMessage{EventID: "evt-2", FileType: 1})

		if err == nil {
			t.Fatal("期望返回事件过期错误")
		}
		if !IsQQEventExpired(err) {
			t.Errorf("错误未被分类为事件过期: %v", err)
		}
		if mock.calls != 1 {
			t.Errorf("调用次数 = %d, want 1（非超时错误不应重试）", mock.calls)
		}
	})
}

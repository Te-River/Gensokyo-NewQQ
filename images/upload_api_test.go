package images

import (
	"context"
	"errors"
	"fmt"
	"strings"
	"testing"
	"testing/synctest"
	"time"

	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

// stubOpenAPI 仅覆盖 uploadMedia/uploadMediaPrivate 用到的两个方法，
// 其余方法若被意外调用会因嵌入接口为 nil 而 panic，便于暴露问题。
type stubOpenAPI struct {
	openapi.OpenAPI
	postGroup func(ctx context.Context, groupID string, msg dto.APIMessage) (*dto.GroupMessageResponse, error)
	postC2C   func(ctx context.Context, userID string, msg dto.APIMessage) (*dto.C2CMessageResponse, error)
}

func (s *stubOpenAPI) PostGroupMessage(ctx context.Context, groupID string, msg dto.APIMessage) (*dto.GroupMessageResponse, error) {
	return s.postGroup(ctx, groupID, msg)
}

func (s *stubOpenAPI) PostC2CMessage(ctx context.Context, userID string, msg dto.APIMessage) (*dto.C2CMessageResponse, error) {
	return s.postC2C(ctx, userID, msg)
}

func newRichMediaMessage() *dto.RichMediaMessage {
	return &dto.RichMediaMessage{FileData: "base64data", FileType: 1}
}

func TestIsTimeoutError(t *testing.T) {
	tests := []struct {
		name string
		err  error
		want bool
	}{
		{name: "nil error", err: nil, want: false},
		{name: "bare deadline exceeded", err: context.DeadlineExceeded, want: true},
		{name: "wrapped deadline exceeded", err: fmt.Errorf("post group media: %w", context.DeadlineExceeded), want: true},
		{name: "deadline text without wrapping", err: errors.New("PostGroupMessage: context deadline exceeded"), want: true},
		{name: "business error 22009", err: errors.New("ret 22009: 主动消息频控"), want: false},
		{name: "context canceled is not timeout", err: context.Canceled, want: false},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := isTimeoutError(tt.err); got != tt.want {
				t.Fatalf("isTimeoutError(%v) = %v, want %v", tt.err, got, tt.want)
			}
		})
	}
}

// attemptOutcome 描述单次上传 API 调用的返回。
type attemptOutcome struct {
	fileInfo string // err == nil 时生效
	err      error
}

// stubPost 返回按 attempt 顺序取用的 outcome，越界时重复最后一个。
func stubPost(outcomes []attemptOutcome, calls *int, seenCtxs *[]context.Context) func(ctx context.Context, _ string, _ dto.APIMessage) (*dto.GroupMessageResponse, error) {
	return func(ctx context.Context, _ string, _ dto.APIMessage) (*dto.GroupMessageResponse, error) {
		*calls++
		if ctx.Err() != nil {
			return nil, fmt.Errorf("attempt %d: context already done before API call: %w", *calls, ctx.Err())
		}
		if deadline, ok := ctx.Deadline(); !ok || deadline.IsZero() {
			// 每次尝试必须携带独立超时 context
			return nil, fmt.Errorf("attempt %d: context has no deadline", *calls)
		}
		*seenCtxs = append(*seenCtxs, ctx)
		idx := *calls - 1
		if idx >= len(outcomes) {
			idx = len(outcomes) - 1
		}
		out := outcomes[idx]
		if out.err != nil {
			return nil, out.err
		}
		return &dto.GroupMessageResponse{MediaResponse: &dto.MediaResponse{FileInfo: out.fileInfo}}, nil
	}
}

func TestUploadMediaRetryBehavior(t *testing.T) {
	businessErr := errors.New("ret 22009: 主动消息频控")
	timeoutWrap := fmt.Errorf("post group media: %w", context.DeadlineExceeded)
	timeoutText := errors.New("PostGroupMessage: context deadline exceeded")

	tests := []struct {
		name      string
		outcomes  []attemptOutcome
		wantCalls int
		wantFile  string
		wantErr   error
	}{
		{
			name:      "success on first attempt",
			outcomes:  []attemptOutcome{{fileInfo: "file-1"}},
			wantCalls: 1,
			wantFile:  "file-1",
		},
		{
			name:      "business error returns immediately without retry",
			outcomes:  []attemptOutcome{{err: businessErr}},
			wantCalls: 1,
			wantErr:   businessErr,
		},
		{
			name:      "wrapped timeout retried then success",
			outcomes:  []attemptOutcome{{err: timeoutWrap}, {fileInfo: "file-2"}},
			wantCalls: 2,
			wantFile:  "file-2",
		},
		{
			name:      "deadline text retried then success",
			outcomes:  []attemptOutcome{{err: timeoutText}, {err: timeoutText}, {fileInfo: "file-3"}},
			wantCalls: 3,
			wantFile:  "file-3",
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			synctest.Test(t, func(t *testing.T) {
				calls := 0
				var seenCtxs []context.Context
				stub := &stubOpenAPI{postGroup: stubPost(tt.outcomes, &calls, &seenCtxs)}

				got, err := uploadMedia(context.Background(), "group-1", newRichMediaMessage(), stub)

				if calls != tt.wantCalls {
					t.Fatalf("PostGroupMessage calls = %d, want %d", calls, tt.wantCalls)
				}
				if tt.wantErr != nil {
					if !errors.Is(err, tt.wantErr) {
						t.Fatalf("uploadMedia error = %v, want error matching %v", err, tt.wantErr)
					}
					if got != "" {
						t.Fatalf("uploadMedia fileInfo = %q on failure, want empty", got)
					}
				} else {
					if err != nil {
						t.Fatalf("uploadMedia unexpected error: %v", err)
					}
					if got != tt.wantFile {
						t.Fatalf("uploadMedia fileInfo = %q, want %q", got, tt.wantFile)
					}
				}
				for i, ctx := range seenCtxs {
					if !errors.Is(ctx.Err(), context.Canceled) {
						t.Errorf("attempt %d context not cancelled after upload: %v", i+1, ctx.Err())
					}
				}
			})
		})
	}
}

// TestUploadMediaAllTimeoutAttemptsExhausted 验证 3 次总尝试、1s/2s 退避、末次错误透传。
func TestUploadMediaAllTimeoutAttemptsExhausted(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		calls := 0
		var attempts []time.Time
		stub := &stubOpenAPI{
			postGroup: func(ctx context.Context, _ string, _ dto.APIMessage) (*dto.GroupMessageResponse, error) {
				calls++
				attempts = append(attempts, time.Now())
				return nil, fmt.Errorf("attempt %d: %w", calls, context.DeadlineExceeded)
			},
		}

		_, err := uploadMedia(context.Background(), "group-1", newRichMediaMessage(), stub)

		if calls != maxMediaUploadAttempts {
			t.Fatalf("PostGroupMessage calls = %d, want %d", calls, maxMediaUploadAttempts)
		}
		if gap := attempts[1].Sub(attempts[0]); gap != 1*time.Second {
			t.Fatalf("backoff before 2nd attempt = %s, want 1s", gap)
		}
		if gap := attempts[2].Sub(attempts[1]); gap != 2*time.Second {
			t.Fatalf("backoff before 3rd attempt = %s, want 2s", gap)
		}
		if !errors.Is(err, context.DeadlineExceeded) {
			t.Fatalf("uploadMedia error = %v, want DeadlineExceeded", err)
		}
		if !strings.Contains(err.Error(), "attempt 3") {
			t.Fatalf("uploadMedia returned %q, want last attempt error", err.Error())
		}
	})
}

func TestUploadMediaPrivateRetryBehavior(t *testing.T) {
	businessErr := errors.New("ret 22009: 主动消息频控")
	timeoutWrap := fmt.Errorf("post c2c media: %w", context.DeadlineExceeded)

	tests := []struct {
		name      string
		outcomes  []attemptOutcome
		wantCalls int
		wantFile  string
		wantErr   error
	}{
		{
			name:      "success on first attempt",
			outcomes:  []attemptOutcome{{fileInfo: "c2c-file-1"}},
			wantCalls: 1,
			wantFile:  "c2c-file-1",
		},
		{
			name:      "business error returns immediately without retry",
			outcomes:  []attemptOutcome{{err: businessErr}},
			wantCalls: 1,
			wantErr:   businessErr,
		},
		{
			name:      "timeout retried then success",
			outcomes:  []attemptOutcome{{err: timeoutWrap}, {err: timeoutWrap}, {fileInfo: "c2c-file-3"}},
			wantCalls: 3,
			wantFile:  "c2c-file-3",
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			synctest.Test(t, func(t *testing.T) {
				calls := 0
				var seenCtxs []context.Context
				postC2C := func(ctx context.Context, _ string, _ dto.APIMessage) (*dto.C2CMessageResponse, error) {
					calls++
					if deadline, ok := ctx.Deadline(); !ok || deadline.IsZero() {
						return nil, fmt.Errorf("attempt %d: context has no deadline", calls)
					}
					seenCtxs = append(seenCtxs, ctx)
					idx := calls - 1
					if idx >= len(tt.outcomes) {
						idx = len(tt.outcomes) - 1
					}
					out := tt.outcomes[idx]
					if out.err != nil {
						return nil, out.err
					}
					return &dto.C2CMessageResponse{MediaResponse: &dto.MediaResponse{FileInfo: out.fileInfo}}, nil
				}
				stub := &stubOpenAPI{postC2C: postC2C}

				got, err := uploadMediaPrivate(context.Background(), "user-1", newRichMediaMessage(), stub)

				if calls != tt.wantCalls {
					t.Fatalf("PostC2CMessage calls = %d, want %d", calls, tt.wantCalls)
				}
				if tt.wantErr != nil {
					if !errors.Is(err, tt.wantErr) {
						t.Fatalf("uploadMediaPrivate error = %v, want error matching %v", err, tt.wantErr)
					}
					if got != "" {
						t.Fatalf("uploadMediaPrivate fileInfo = %q on failure, want empty", got)
					}
				} else {
					if err != nil {
						t.Fatalf("uploadMediaPrivate unexpected error: %v", err)
					}
					if got != tt.wantFile {
						t.Fatalf("uploadMediaPrivate fileInfo = %q, want %q", got, tt.wantFile)
					}
				}
				for i, ctx := range seenCtxs {
					if !errors.Is(ctx.Err(), context.Canceled) {
						t.Errorf("attempt %d context not cancelled after upload: %v", i+1, ctx.Err())
					}
				}
			})
		})
	}
}

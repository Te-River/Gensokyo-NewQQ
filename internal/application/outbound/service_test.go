package outbound

import (
	"context"
	"errors"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/hoshinonyaruko/gensokyo/internal/domain/identity"
	"github.com/hoshinonyaruko/gensokyo/internal/domain/message"
)

// mockSender 记录调用并模拟失败/成功。
type mockSender struct {
	failTimes int32
	calls     int32
	lastReply *ReplyRef
}

func (m *mockSender) Send(_ context.Context, target identity.ResolvedTarget, msg QQMessage) (QQSendResult, error) {
	atomic.AddInt32(&m.calls, 1)
	m.lastReply = msg.Reply
	if atomic.LoadInt32(&m.calls) <= m.failTimes {
		return QQSendResult{}, errors.New("mock: deadline exceeded")
	}
	return QQSendResult{MessageID: "mid-1"}, nil
}

type retryClassifier struct{}

func (retryClassifier) Classify(err error) ErrorClass {
	if err != nil && strings.Contains(err.Error(), "deadline") {
		return ErrorClass{Retryable: true}
	}
	return ErrorClass{}
}

func testCommand() OutboundCommand {
	u := identity.ResolvedUser{OpenID: "01234567890123456789012345678901", VirtualUserID: "1001"}
	return OutboundCommand{
		Target:  identity.ResolvedTarget{Kind: identity.TargetPrivate, User: &u},
		Message: OutboundMessage{Parts: []message.MessagePart{message.TextPart{Text: "hi"}}},
	}
}

func TestSendSuccess(t *testing.T) {
	s := NewService(&mockSender{}, DefaultRetryPolicy(retryClassifier{}))
	res, err := s.Send(context.Background(), testCommand())
	if err != nil {
		t.Fatalf("Send: %v", err)
	}
	if res.MessageID != "mid-1" {
		t.Fatalf("MessageID = %q", res.MessageID)
	}
}

func TestSendRetriesThenSucceeds(t *testing.T) {
	sender := &mockSender{failTimes: 2}
	s := NewService(sender, DefaultRetryPolicy(retryClassifier{}))
	if _, err := s.Send(context.Background(), testCommand()); err != nil {
		t.Fatalf("Send: %v", err)
	}
	if got := atomic.LoadInt32(&sender.calls); got != 3 {
		t.Fatalf("calls = %d, want 3 (2 failures + 1 success)", got)
	}
}

func TestSendGivesUpAfterMaxAttempts(t *testing.T) {
	sender := &mockSender{failTimes: 100}
	policy := DefaultRetryPolicy(retryClassifier{})
	policy.Backoff = func(int) time.Duration { return 0 }
	s := NewService(sender, policy)
	if _, err := s.Send(context.Background(), testCommand()); err == nil {
		t.Fatal("Send should fail after exhausting retries")
	}
	if got := atomic.LoadInt32(&sender.calls); int(got) != policy.MaxAttempts {
		t.Fatalf("calls = %d, want %d", got, policy.MaxAttempts)
	}
}

func TestSendNonRetryableFailsImmediately(t *testing.T) {
	s2 := &OutboundService{sender: &failSender{}, retry: DefaultRetryPolicy(retryClassifier{})}
	if _, err := s2.Send(context.Background(), testCommand()); err == nil {
		t.Fatal("Send should fail")
	}
}

type failSender struct{}

func (failSender) Send(context.Context, identity.ResolvedTarget, QQMessage) (QQSendResult, error) {
	return QQSendResult{}, errors.New("rate limited")
}

func TestSendMessageReplyPassed(t *testing.T) {
	sender := &mockSender{}
	s := NewService(sender, DefaultRetryPolicy(retryClassifier{}))

	cmd := testCommand()
	cmd.Message.Reply = &ReplyRef{MessageID: "999"}
	if _, err := s.Send(context.Background(), cmd); err != nil {
		t.Fatalf("Send: %v", err)
	}
	if sender.lastReply == nil || sender.lastReply.MessageID != "999" {
		t.Fatalf("reply not passed: %+v", sender.lastReply)
	}
}

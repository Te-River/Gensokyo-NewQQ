package action

import (
	"context"
	"errors"
	"testing"
)

func echoHandler(_ context.Context, req interface{}) (interface{}, error) {
	return req, nil
}

func TestRegistryRegisterAndDispatch(t *testing.T) {
	reg := NewRegistry()
	reg.Register("send_msg", Handler{Decode: DecodeSendMessage, Handle: echoHandler})
	d := NewDispatcher(reg)

	out, err := d.Dispatch(context.Background(),
		[]byte(`{"action":"send_msg","params":{"group_id":123,"message":"hi"}}`))
	if err != nil {
		t.Fatalf("Dispatch: %v", err)
	}
	act, ok := out.(*SendMessageAction)
	if !ok {
		t.Fatalf("out type = %T", out)
	}
	if act.GroupID != "123" {
		t.Fatalf("GroupID = %q, want 123 (int coerced to string)", act.GroupID)
	}
	if act.Message != "hi" {
		t.Fatalf("Message = %q", act.Message)
	}
}

func TestDispatchUnknownAction(t *testing.T) {
	d := NewDispatcher(NewRegistry())
	_, err := d.Dispatch(context.Background(), []byte(`{"action":"nope","params":{}}`))
	if !errors.Is(err, ErrUnknownAction) {
		t.Fatalf("err = %v, want ErrUnknownAction", err)
	}
}

func TestDispatchEmptyAction(t *testing.T) {
	d := NewDispatcher(NewRegistry())
	if _, err := d.Dispatch(context.Background(), []byte(`{"params":{}}`)); !errors.Is(err, ErrUnknownAction) {
		t.Fatalf("err = %v", err)
	}
}

func TestDispatchInvalidParams(t *testing.T) {
	reg := NewRegistry()
	reg.Register("send_msg", Handler{Decode: DecodeSendMessage, Handle: echoHandler})
	d := NewDispatcher(reg)
	// 缺 group_id/user_id → 校验失败
	if _, err := d.Dispatch(context.Background(), []byte(`{"action":"send_msg","params":{"message":"x"}}`)); err == nil {
		t.Fatal("expected validation error")
	}
	// 坏 JSON params
	if _, err := d.Dispatch(context.Background(), []byte(`{"action":"send_msg","params":"oops"}`)); err == nil {
		t.Fatal("expected decode error")
	}
}

func TestDispatchMalformedEnvelope(t *testing.T) {
	d := NewDispatcher(NewRegistry())
	if _, err := d.Dispatch(context.Background(), []byte(`{not-json`)); err == nil {
		t.Fatal("expected error on malformed envelope")
	}
}

// TestHTTPAndWSShareDispatcher HTTP 与 WS 共用同一 Dispatcher（同一注册表实例）。
func TestHTTPAndWSShareDispatcher(t *testing.T) {
	reg := NewRegistry()
	reg.Register("send_msg", Handler{Decode: DecodeSendMessage, Handle: echoHandler})

	httpD := NewDispatcher(reg)
	wsD := NewDispatcher(reg) // WS 复用同一注册表（可进一步复用同一实例）

	out1, err := httpD.Dispatch(context.Background(), []byte(`{"action":"send_msg","params":{"user_id":"u1","message":"a"}}`))
	if err != nil {
		t.Fatal(err)
	}
	out2, err := wsD.Dispatch(context.Background(), []byte(`{"action":"send_msg","params":{"user_id":"u2","message":"b"}}`))
	if err != nil {
		t.Fatal(err)
	}
	if out1.(*SendMessageAction).UserID != "u1" || out2.(*SendMessageAction).UserID != "u2" {
		t.Fatalf("dispatch mismatch: %+v %+v", out1, out2)
	}
}

func TestSendMessageStringUserID(t *testing.T) {
	req, err := DecodeSendMessage([]byte(`{"user_id":"u42","message":"hi"}`))
	if err != nil {
		t.Fatal(err)
	}
	if req.(*SendMessageAction).UserID != "u42" {
		t.Fatalf("UserID = %q", req.(*SendMessageAction).UserID)
	}
}

package handlers

import (
	"encoding/json"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/callapi"
)

type MockClient struct {
	sentMessage map[string]interface{}
}

func (m *MockClient) SendMessage(message map[string]interface{}) error {
	m.sentMessage = message
	return nil
}

func TestSetGroupCardMock(t *testing.T) {
	client := &MockClient{}
	msg := callapi.ActionMessage{
		Action: "set_group_card",
		Params: callapi.ParamsContent{
			GroupID: "123456",
			UserID:  "7890",
		},
		Echo: "echo_test_card",
	}

	result := callapi.CallAPIFromDict(client, nil, nil, msg)
	if result == "" {
		t.Fatal("expected non-empty JSON response for set_group_card")
	}

	// 检查返回的 JSON
	var resp struct {
		Message string      `json:"message"`
		RetCode int         `json:"retcode"`
		Status  string      `json:"status"`
		Echo    interface{} `json:"echo"`
	}
	err := json.Unmarshal([]byte(result), &resp)
	if err != nil {
		t.Fatalf("failed to unmarshal JSON response: %v", err)
	}

	if resp.Status != "ok" || resp.RetCode != 0 || resp.Echo != "echo_test_card" {
		t.Errorf("unexpected JSON response values: %+v", resp)
	}

	// 检查 client 是否收到正确的 map
	if client.sentMessage == nil {
		t.Fatal("client did not receive sent message map")
	}
	if client.sentMessage["status"] != "ok" || client.sentMessage["echo"] != "echo_test_card" {
		t.Errorf("unexpected message map content: %+v", client.sentMessage)
	}
	retcodeVal := client.sentMessage["retcode"]
	if val, ok := retcodeVal.(float64); !ok || val != 0 {
		t.Errorf("expected retcode 0 (float64), got %v (%T)", retcodeVal, retcodeVal)
	}
}

func TestSetGroupAddRequestMock(t *testing.T) {
	client := &MockClient{}
	msg := callapi.ActionMessage{
		Action: "set_group_add_request",
		Params: callapi.ParamsContent{
			GroupID: "123456",
			UserID:  "7890",
		},
		Echo: "echo_test_add",
	}

	result := callapi.CallAPIFromDict(client, nil, nil, msg)
	if result == "" {
		t.Fatal("expected non-empty JSON response for set_group_add_request")
	}

	// 检查返回的 JSON
	var resp struct {
		Message string      `json:"message"`
		RetCode int         `json:"retcode"`
		Status  string      `json:"status"`
		Echo    interface{} `json:"echo"`
	}
	err := json.Unmarshal([]byte(result), &resp)
	if err != nil {
		t.Fatalf("failed to unmarshal JSON response: %v", err)
	}

	if resp.Status != "ok" || resp.RetCode != 0 || resp.Echo != "echo_test_add" {
		t.Errorf("unexpected JSON response values: %+v", resp)
	}

	// 检查 client 是否收到正确的 map
	if client.sentMessage == nil {
		t.Fatal("client did not receive sent message map")
	}
	if client.sentMessage["status"] != "ok" || client.sentMessage["echo"] != "echo_test_add" {
		t.Errorf("unexpected message map content: %+v", client.sentMessage)
	}
	retcodeVal := client.sentMessage["retcode"]
	if val, ok := retcodeVal.(float64); !ok || val != 0 {
		t.Errorf("expected retcode 0 (float64), got %v (%T)", retcodeVal, retcodeVal)
	}
}

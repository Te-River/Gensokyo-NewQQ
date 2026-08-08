package action

import (
	"encoding/json"
	"errors"
)

// SendMessageAction typed 的发送消息动作（P9.1 示例）。
// group_id / user_id 兼容 int/string 输入，解码后统一为 string。
type SendMessageAction struct {
	GroupID string `json:"group_id"`
	UserID  string `json:"user_id"`
	Message string `json:"message"`
}

// Validate 动作参数校验。
func (a *SendMessageAction) Validate() error {
	if a.GroupID == "" && a.UserID == "" {
		return errors.New("group_id or user_id required")
	}
	return nil
}

// DecodeSendMessage 解码并校验 send_msg 类参数（兼容 int/string ID）。
func DecodeSendMessage(data []byte) (interface{}, error) {
	var raw struct {
		GroupID json.RawMessage `json:"group_id"`
		UserID  json.RawMessage `json:"user_id"`
		Message string          `json:"message"`
	}
	if err := json.Unmarshal(data, &raw); err != nil {
		return nil, err
	}
	a := &SendMessageAction{Message: raw.Message}
	var err error
	if a.GroupID, err = stringOrEmpty(raw.GroupID); err != nil {
		return nil, err
	}
	if a.UserID, err = stringOrEmpty(raw.UserID); err != nil {
		return nil, err
	}
	if err := a.Validate(); err != nil {
		return nil, err
	}
	return a, nil
}

// stringOrEmpty 把 JSON 标量（字符串或数字）转为 string；null/缺省为空串。
func stringOrEmpty(raw json.RawMessage) (string, error) {
	if len(raw) == 0 || string(raw) == "null" {
		return "", nil
	}
	var s string
	if err := json.Unmarshal(raw, &s); err == nil {
		return s, nil
	}
	var n json.Number
	if err := json.Unmarshal(raw, &n); err == nil {
		return n.String(), nil
	}
	return "", errors.New("expected string or number")
}

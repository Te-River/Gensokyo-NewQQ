package handlers

import (
	"encoding/json"
	"reflect"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/callapi"
)

// TestStringListUnmarshalJSON 表驱动验证 M1 修复:数字数组/字符串数组/混合/null 四种输入
// 均能元素级归一化,不再因数字元素导致整请求 Unmarshal 失败。
func TestStringListUnmarshalJSON(t *testing.T) {
	tests := []struct {
		name    string
		in      string
		want    []string
		wantErr bool
	}{
		{"数字数组归一化为字符串", `[123,456]`, []string{"123", "456"}, false},
		{"字符串数组直通", `["123","456"]`, []string{"123", "456"}, false},
		{"混合数组逐元素归一化", `[123,"456",789]`, []string{"123", "456", "789"}, false},
		{"null不报错且为空", `null`, nil, false},
		{"空数组为空", `[]`, nil, false},
		{"非数组输入报错", `"123"`, nil, true},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			var got callapi.StringList
			err := json.Unmarshal([]byte(tt.in), &got)
			if (err != nil) != tt.wantErr {
				t.Fatalf("err = %v, wantErr %v", err, tt.wantErr)
			}
			if tt.wantErr {
				return
			}
			if !reflect.DeepEqual([]string(got), tt.want) {
				t.Errorf("got %v, want %v", got, tt.want)
			}
		})
	}
}

// TestParamsContentUserIDsNumericArray 复现 M1 原始报文:OneBot 客户端发数字 user_ids 数组,
// 经 ActionMessage→ParamsContent(*Alias 嵌入 UnmarshalJSON)双层解析后 StringList 仍须生效,
// 且既有的 group_id 柔性转换不受影响。
func TestParamsContentUserIDsNumericArray(t *testing.T) {
	raw := `{"action":"set_group_kick","echo":1,"params":{"group_id":"123456","user_ids":[3607918353,123456],"user_openids":[1,2],"group_openids":[42]}}`
	var msg callapi.ActionMessage
	if err := json.Unmarshal([]byte(raw), &msg); err != nil {
		t.Fatalf("数字数组不应导致整请求解析失败(M1 回归): %v", err)
	}
	if want := []string{"3607918353", "123456"}; !reflect.DeepEqual([]string(msg.Params.UserIDs), want) {
		t.Errorf("user_ids = %v, want %v", msg.Params.UserIDs, want)
	}
	if want := []string{"1", "2"}; !reflect.DeepEqual([]string(msg.Params.UserOpenIDs), want) {
		t.Errorf("user_openids = %v, want %v", msg.Params.UserOpenIDs, want)
	}
	if want := []string{"42"}; !reflect.DeepEqual([]string(msg.Params.GroupOpenIDs), want) {
		t.Errorf("group_openids = %v, want %v", msg.Params.GroupOpenIDs, want)
	}
	if msg.Params.GroupID != "123456" {
		t.Errorf("group_id 既有柔性转换不应受影响: %q", msg.Params.GroupID)
	}
}

// TestParamsContentUserIDsStringArray 字符串形态 user_ids 行为不变(向后兼容)。
func TestParamsContentUserIDsStringArray(t *testing.T) {
	raw := `{"action":"set_group_kick","params":{"user_ids":["111","222"]}}`
	var msg callapi.ActionMessage
	if err := json.Unmarshal([]byte(raw), &msg); err != nil {
		t.Fatalf("字符串数组解析失败: %v", err)
	}
	if want := []string{"111", "222"}; !reflect.DeepEqual([]string(msg.Params.UserIDs), want) {
		t.Errorf("user_ids = %v, want %v", msg.Params.UserIDs, want)
	}
}

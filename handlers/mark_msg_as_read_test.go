package handlers

// mark_msg_as_read 回执契约：官方无已读上报接口，空实现只回 retcode/status/echo。
// 改前 data 是 gjson.Result 被直接序列化，回执里出现 Indexes/Num/Raw/Str/Type 等内部字段；
// 改后 data 必须是 null。

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/callapi"
)

func TestMarkMsgAsReadDataIsNull(t *testing.T) {
	client := &contractTestClient{}

	raw, err := MarkThisMessageAsRead(client, nil, nil, callapi.ActionMessage{
		Action: "mark_msg_as_read",
		Echo:   "read-1",
	})
	if err != nil {
		t.Fatalf("MarkThisMessageAsRead 返回错误: %v", err)
	}

	var got map[string]interface{}
	if err := json.Unmarshal([]byte(raw), &got); err != nil {
		t.Fatalf("回执不是合法 JSON: %v (%s)", err, raw)
	}
	if got["retcode"] != float64(0) || got["status"] != "ok" {
		t.Errorf("回执应为 retcode:0/status:ok: %s", raw)
	}
	if got["echo"] != "read-1" {
		t.Errorf("echo 应原样回显: %v", got["echo"])
	}
	data, ok := got["data"]
	if !ok {
		t.Fatalf("回执缺 data 键（应为 null）: %s", raw)
	}
	if data != nil {
		t.Errorf("data 应为 null，实际: %v (%s)", data, raw)
	}
	for _, leaked := range []string{"Indexes", "Raw", "Str", "Num"} {
		if strings.Contains(raw, leaked) {
			t.Errorf("回执泄漏 gjson 内部字段 %q: %s", leaked, raw)
		}
	}
	if client.last["data"] != nil {
		t.Errorf("下发 payload 的 data 应为 nil，实际: %v", client.last["data"])
	}
}

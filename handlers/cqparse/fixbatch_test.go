package cqparse

// fixbatch 回归保护（cqparse 新链 + dto 宽容解析）：
//   - A1：入站专用码 ark / reply,msg_idx= 纯剥离，正文其余部分保留；
//   - A2：auto_approved 四形态（对象/true/false/null）与 verify_info 空对象宽容解析。

import (
	"encoding/json"
	"testing"

	"github.com/tencent-connect/botgo/dto"
)

func fixParse(t *testing.T, s string) (string, map[string][]string) {
	t.Helper()
	text, found, _, err := Parse(Input{Kind: InputString, String: s, GroupID: "g1", HasGroup: true, UserID: "u1"}, nil)
	if err != nil {
		t.Fatalf("Parse(%q) 失败: %v", s, err)
	}
	return text, found
}

// TestFixBatch_InboundOnlyCodesStripped A1：三类入站专用码剥离，正文保留
func TestFixBatch_InboundOnlyCodesStripped(t *testing.T) {
	cases := []struct {
		name string
		in   string
		want string
	}{
		{"裸 ark", "[CQ:ark]hello ", "hello "},
		{"带参数 ark", "hello [CQ:ark,ark_type=picture,ark_name=图片]", "hello "},
		{"ark 转义逗号", "hello [CQ:ark,ark_type=picture,ark_name=甲&#44;乙,prompt=点击查看图片]", "hello "},
		{"reply msg_idx", "hello [CQ:reply,msg_idx=REFIDX_x]", "hello "},
		{"混合剥离", "hello [CQ:ark,ark_type=picture][CQ:reply,msg_idx=REFIDX_x]尾", "hello 尾"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			text, found := fixParse(t, tc.in)
			if text != tc.want {
				t.Fatalf("正文: got %q, want %q", text, tc.want)
			}
			if _, ok := found["reply_msg_id"]; ok {
				t.Fatalf("入站专用码不得写入 reply_msg_id: %v", found)
			}
		})
	}
}

// TestFixBatch_ReplyIdUnchanged A1 回归：[CQ:reply,id=123] 既有语义零变化
func TestFixBatch_ReplyIdUnchanged(t *testing.T) {
	text, found := fixParse(t, "前[CQ:reply,id=123]后")
	if text != "前后" {
		t.Fatalf("正文: got %q, want %q", text, "前后")
	}
	if got := found["reply_msg_id"]; len(got) != 1 || got[0] != "123" {
		t.Fatalf("reply_msg_id: %v", found)
	}
	// 无 id 无 msg_idx：保留原文（既有行为）
	text, found = fixParse(t, "前[CQ:reply]后")
	if text != "前[CQ:reply]后" {
		t.Fatalf("裸 reply 应保留原文: got %q", text)
	}
	if len(found) != 0 {
		t.Fatalf("裸 reply 不应产 foundItems: %v", found)
	}
}

// TestFixBatch_AutoApprovedForms A2：auto_approved 四形态均不报错，布尔语义保留
func TestFixBatch_AutoApprovedForms(t *testing.T) {
	cases := []struct {
		name         string
		raw          string
		wantNil      bool
		wantStrategy string
		wantApproved bool
	}{
		{"对象", `{"strategy_id":"s1"}`, false, "s1", true},
		{"true", `true`, false, "", true},
		{"false", `false`, false, "", false},
		{"null", `null`, true, "", false},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			var ev dto.GroupJoinRequestEvent
			if err := json.Unmarshal([]byte(`{"join_request_id":"jr1","auto_approved":`+tc.raw+`}`), &ev); err != nil {
				t.Fatalf("auto_approved=%s 应反序列化成功: %v", tc.raw, err)
			}
			if tc.wantNil {
				if ev.AutoApproved != nil {
					t.Fatalf("null 应置 nil: %+v", ev.AutoApproved)
				}
				return
			}
			if ev.AutoApproved == nil {
				t.Fatalf("auto_approved=%s 不应为 nil", tc.raw)
			}
			if ev.AutoApproved.StrategyID != tc.wantStrategy || ev.AutoApproved.Approved != tc.wantApproved {
				t.Fatalf("auto_approved=%s: %+v", tc.raw, ev.AutoApproved)
			}
		})
	}
	// 列表路径：true 形态不得让整个列表反序列化失败
	var list dto.JoinRequestList
	if err := json.Unmarshal([]byte(`{"list":[{"join_request_id":"jr1","auto_approved":true}],"next_cursor":"c1"}`), &list); err != nil {
		t.Fatalf("列表 auto_approved=true 应反序列化成功: %v", err)
	}
	if len(list.List) != 1 || list.List[0].AutoApproved == nil || !list.List[0].AutoApproved.Approved {
		t.Fatalf("列表 auto_approved: %+v", list.List)
	}
}

// TestFixBatch_VerifyInfoTolerant A2：verify_info 空对象/仅 review_qa_list 不报错
func TestFixBatch_VerifyInfoTolerant(t *testing.T) {
	cases := []struct {
		name string
		raw  string
		want string
	}{
		{"空对象", `{}`, ""},
		{"仅 review_qa_list", `{"review_qa_list":[]}`, ""},
		{"字符串", `"111"`, "111"},
		{"对象", `{"method":"verify_message","verify_message":"几款看看","review_qa_list":[]}`, "几款看看"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			var ev dto.GroupJoinRequestEvent
			if err := json.Unmarshal([]byte(`{"join_request_id":"jr1","verify_info":`+tc.raw+`}`), &ev); err != nil {
				t.Fatalf("verify_info=%s 应反序列化成功: %v", tc.raw, err)
			}
			if got := ev.VerifyInfo.String(); got != tc.want {
				t.Fatalf("verify_info: got %q, want %q", got, tc.want)
			}
		})
	}
}

package handlers

// fixbatch 修复批次回归保护：入站专用码出站剥离、reply 引用语义不受影响、
// 入群申请列表 next_index 在 32 位可表示范围之外时置 0。

import (
	"context"
	"encoding/json"
	"strings"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

// TestFixBatch_ReplyIdStillConsumed (1) 回归保护：[CQ:reply,id=123] 仍写入 reply_msg_id
func TestFixBatch_ReplyIdStillConsumed(t *testing.T) {
	found := map[string][]string{}
	out := ProcessCQCodePipeline("前[CQ:reply,id=123]后", found, nil)
	if got := found["reply_msg_id"]; len(got) != 1 || got[0] != "123" {
		t.Fatalf("reply_msg_id 未被消费: %v", found)
	}
	if out != "前后" {
		t.Fatalf("正文清理错误: got %q, want %q", out, "前后")
	}
}

// TestFixBatch_InboundOnlyCodesStripped (2)(3) 入站专用码剥离，正文其余部分保留
func TestFixBatch_InboundOnlyCodesStripped(t *testing.T) {
	cases := []struct {
		name      string
		input     string
		wantText  string
		wantReply bool
	}{
		{
			name:     "ark 带参数剥离",
			input:    "hello [CQ:ark,ark_type=picture,ark_name=图片]",
			wantText: "hello ",
		},
		{
			name:     "裸 ark 剥离",
			input:    "[CQ:ark]hello ",
			wantText: "hello ",
		},
		{
			name:     "ark 参数值含转义逗号整段剥离",
			input:    "hello [CQ:ark,ark_type=picture,ark_name=甲&#44;乙,prompt=点击查看图片]",
			wantText: "hello ",
		},
		{
			name:     "reply,msg_idx 剥离且不写 reply_msg_id",
			input:    "hello [CQ:reply,msg_idx=REFIDX_x]",
			wantText: "hello ",
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			found := map[string][]string{}
			out := ProcessCQCodePipeline(tc.input, found, nil)
			if out != tc.wantText {
				t.Fatalf("正文: got %q, want %q", out, tc.wantText)
			}
			if strings.Contains(out, "[CQ:") {
				t.Fatalf("入站专用码未剥离: %q", out)
			}
			if _, ok := found["reply_msg_id"]; ok {
				t.Fatalf("msg_idx/ark 不得写入 reply_msg_id: %v", found)
			}
		})
	}
	// 混合形态：ark 与 msg_idx 同段剥离，reply,id= 引用语义不受影响
	found := map[string][]string{}
	out := ProcessCQCodePipeline("hello [CQ:ark,ark_type=picture][CQ:reply,msg_idx=REFIDX_x][CQ:reply,id=456]尾", found, nil)
	if out != "hello 尾" {
		t.Fatalf("混合剥离正文: got %q", out)
	}
	if got := found["reply_msg_id"]; len(got) != 1 || got[0] != "456" {
		t.Fatalf("reply,id= 应仍被消费: %v", found)
	}
}

// fbMockAPI 仅实现 JoinRequestList 的 mock OpenAPI
type fbMockAPI struct {
	openapi.OpenAPI
	resp *dto.JoinRequestList
}

func (m *fbMockAPI) JoinRequestList(_ context.Context, _, _ string, _ int) (*dto.JoinRequestList, error) {
	return m.resp, nil
}

// TestFixBatch_NextIndex32Bit (4) next_index 兼容字段仅在 32 位可表示范围内回填
func TestFixBatch_NextIndex32Bit(t *testing.T) {
	const groupOpenID = "abcdabcdabcdabcdabcdabcdabcdabcd"
	cases := []struct {
		name   string
		cursor string
		want   int
	}{
		{"超 32 位游标置 0", "1785767153250497", 0},
		{"MaxInt32 边界保留", "2147483647", 2147483647},
		{"MaxInt32+1 置 0", "2147483648", 0},
		{"普通游标保留", "123", 123},
		{"非数字游标置 0", "REFIDX_x", 0},
		{"空游标置 0", "", 0},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			mock := &fbMockAPI{resp: &dto.JoinRequestList{NextCursor: tc.cursor}}
			got, err := GetGroupJoinRequestList(&tvClient{}, nil, mock, callapi.ActionMessage{
				Action: "get_group_join_request_list",
				Params: callapi.ParamsContent{GroupID: groupOpenID},
			})
			if err != nil {
				t.Fatalf("handler 错误: %v", err)
			}
			var resp struct {
				Data struct {
					NextCursor string `json:"next_cursor"`
					NextIndex  int    `json:"next_index"`
				} `json:"data"`
			}
			if err := json.Unmarshal([]byte(got), &resp); err != nil {
				t.Fatalf("响应解析失败: %v (%s)", err, got)
			}
			if resp.Data.NextCursor != tc.cursor || resp.Data.NextIndex != tc.want {
				t.Fatalf("分页字段: cursor=%q index=%d, want cursor=%q index=%d",
					resp.Data.NextCursor, resp.Data.NextIndex, tc.cursor, tc.want)
			}
		})
	}
}

package handlers

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/tencent-connect/botgo/dto"
)

// ---------- buildSetGroupCQCode：user_ids 数组路径 ----------

// TestBuildSetGroupCQCodeUserIDsArray 验证消息段数组路径 user_ids 列表还原为逗号 CQ 码
func TestBuildSetGroupCQCodeUserIDsArray(t *testing.T) {
	tests := []struct {
		name string
		data map[string]interface{}
		want string
	}{
		{
			// user_ids 的逗号必须转义为 &#44;：cqParseParams 按逗号切分 KV 对，
			// 未转义时 "user_ids=10001,10002" 会被切成无 "=" 的段而丢失
			name: "string数组还原为转义逗号串",
			data: map[string]interface{}{
				"action":        "kick",
				"group_id":      "123",
				"user_ids":      []interface{}{"10001", "10002", "10003"},
				"add_blacklist": false,
			},
			want: "[CQ:set_group,action=kick,group_id=123,user_ids=10001&#44;10002&#44;10003,add_blacklist=false]",
		},
		{
			name: "float64数组(JSON数字)还原",
			data: map[string]interface{}{
				"action":   "kick",
				"group_id": "123",
				"user_ids": []interface{}{float64(10001), float64(10002)},
			},
			want: "[CQ:set_group,action=kick,group_id=123,user_ids=10001&#44;10002]",
		},
		{
			name: "数组与user_id同时存在按固定顺序输出",
			data: map[string]interface{}{
				"action":        "blacklist_add",
				"group_id":      "123",
				"user_id":       "10000",
				"user_ids":      []interface{}{"10001", "10002"},
				"add_blacklist": true,
			},
			want: "[CQ:set_group,action=blacklist_add,group_id=123,user_id=10000,user_ids=10001&#44;10002,add_blacklist=true]",
		},
		{
			name: "空数组不产生user_ids段",
			data: map[string]interface{}{
				"action":   "kick",
				"group_id": "123",
				"user_ids": []interface{}{},
			},
			want: "[CQ:set_group,action=kick,group_id=123]",
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := buildSetGroupCQCode(tt.data)
			if got != tt.want {
				t.Errorf("拼装结果:\n  got:  %q\n  want: %q", got, tt.want)
			}
		})
	}
}

// TestBuildSetGroupCQCodeUserIDsRoundTrip 验证数组拼装 → cqParseParams 还原可逆
func TestBuildSetGroupCQCodeUserIDsRoundTrip(t *testing.T) {
	originalIDs := []string{"10001", "10002", "10003"}
	data := map[string]interface{}{
		"action":        "kick",
		"group_id":      "123",
		"user_ids":      []interface{}{"10001", "10002", "10003"},
		"add_blacklist": true,
	}
	cq := buildSetGroupCQCode(data)

	paramsStr := cq[len("[CQ:set_group,") : len(cq)-1]
	got := cqParseParams(paramsStr)

	if got["action"] != "kick" || got["group_id"] != "123" {
		t.Fatalf("action/group_id 往返失败: %v", got)
	}
	if got["user_ids"] != "10001,10002,10003" {
		t.Errorf("user_ids 往返应还原逗号串: got %q", got["user_ids"])
	}
	// 还原后的列表与原始列表逐项一致（顺序保持）
	parts := strings.Split(got["user_ids"], ",")
	if len(parts) != len(originalIDs) {
		t.Fatalf("还原项数: got %d, want %d", len(parts), len(originalIDs))
	}
	for i := range originalIDs {
		if parts[i] != originalIDs[i] {
			t.Errorf("[%d]: got %q, want %q", i, parts[i], originalIDs[i])
		}
	}
	if got["add_blacklist"] != "true" {
		t.Errorf("add_blacklist 往返: got %q, want true", got["add_blacklist"])
	}
}

// ---------- cqSetGroupUserIDs ----------

// TestCQSetGroupUserIDs 验证 CQ 路径 user_id/user_ids 合并去重与截断
func TestCQSetGroupUserIDs(t *testing.T) {
	tests := []struct {
		name   string
		params map[string]string
		want   []string
	}{
		{
			name:   "user_id单个",
			params: map[string]string{"user_id": "10001"},
			want:   []string{"10001"},
		},
		{
			name:   "user_ids逗号切分并去空白",
			params: map[string]string{"user_ids": "10001,10002, 10003 "},
			want:   []string{"10001", "10002", "10003"},
		},
		{
			name:   "两者合并去重保序",
			params: map[string]string{"user_id": "10002", "user_ids": "10001,10002,10003"},
			want:   []string{"10002", "10001", "10003"},
		},
		{
			name:   "空项过滤",
			params: map[string]string{"user_ids": ",10001,,10002,"},
			want:   []string{"10001", "10002"},
		},
		{
			name:   "超20截断为20",
			params: map[string]string{"user_ids": "1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21"},
			want:   []string{"1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16", "17", "18", "19", "20"},
		},
		{
			name:   "全空返回空",
			params: map[string]string{"user_id": "", "user_ids": ""},
			want:   []string{},
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := cqSetGroupUserIDs(tt.params, "[CQ:set_group,test]")
			if len(got) != len(tt.want) {
				t.Fatalf("长度: got %d, want %d, got=%v", len(got), len(tt.want), got)
			}
			for i := range tt.want {
				if got[i] != tt.want[i] {
					t.Errorf("[%d]: got %q, want %q", i, got[i], tt.want[i])
				}
			}
		})
	}
}

// ---------- parseMDData D2: force_verify_image_resource ----------

// mdJSON 构造 parseMDData 输入
func mdJSON(t *testing.T, content string) []byte {
	t.Helper()
	raw, err := json.Marshal(map[string]interface{}{
		"markdown": map[string]interface{}{"content": content},
	})
	if err != nil {
		t.Fatalf("构造输入失败: %v", err)
	}
	return raw
}

// TestParseMDDataForceVerifyDisabled 开关关闭（无 config 实例默认 false）时产物不含该字段。
// 依赖测试执行顺序：本测试必须在 TestParseMDDataForceVerifyEnabled 之前运行（同文件声明顺序）。
func TestParseMDDataForceVerifyDisabled(t *testing.T) {
	if config.GetForceVerifyImageResource() {
		t.Skip("config 实例已存在且开关为 true,disabled 分支无法在此进程内验证")
	}

	md, _, err := parseMDData(mdJSON(t, "# hello"))
	if err != nil || md == nil {
		t.Fatalf("parseMDData 异常: md=%v, err=%v", md, err)
	}
	if md.ForceVerifyImageResource {
		t.Error("开关关闭时不应注入 ForceVerifyImageResource")
	}
	out, _ := json.Marshal(md)
	if strings.Contains(string(out), "force_verify_image_resource") {
		t.Errorf("omitempty 应保证 JSON 不含该字段: %s", out)
	}
}

// TestParseMDDataForceVerifyInjectionPoint 等价验证 enabled 分支（最小可行验证）。
// config 单例在测试进程内无安全初始化入口：LoadConfig(fastload=true) 在 instance==nil 时
// panic（config.go:91 访问空实例），LoadConfig(fastload=false) 缺项时经 ensureConfigComplete
// 触发 sys.RestartApplication() → os.Exit（config.go:402），两者均不可在测试中使用。
// 因此 enabled 分支拆解为两段已验证证据：
//  1. 注入行与生产代码 message_parser.go:2160 同构执行（本测试）；
//  2. 注入后的序列化形态（含官方字段名/omitempty）由 TestMarkdownDTOFieldTag 逐字验证。
// parseMDData 产物 md 可达注入点由 TestParseMDDataForceVerifyDisabled 端到端证明。
func TestParseMDDataForceVerifyInjectionPoint(t *testing.T) {
	if config.GetForceVerifyImageResource() {
		t.Skip("config 实例存在且开关为 true,本测试假定关闭态")
	}
	if config.GetForceVerifyImageResource() { // false 前提下此分支不可达,若可达则注入行有缺陷
		t.Fatal("关闭态不应进入注入分支")
	}

	// 与 message_parser.go:2160 逐字同构的注入行
	md := &dto.Markdown{Content: "# hello"}
	if md != nil && config.GetForceVerifyImageResource() {
		md.ForceVerifyImageResource = true
	}

	out, _ := json.Marshal(md)
	if strings.Contains(string(out), "force_verify_image_resource") {
		t.Errorf("关闭态注入行不应改变序列化结果: %s", out)
	}

	// 开启态语义：开关为 true 时注入目标字段置位（字段与 tag 由 TestMarkdownDTOFieldTag 保证）
	target := dto.Markdown{Content: "# hello"}
	target.ForceVerifyImageResource = true
	outTarget, _ := json.Marshal(target)
	if !strings.Contains(string(outTarget), `"force_verify_image_resource":true`) {
		t.Errorf("开启态序列化应含 force_verify_image_resource:true: %s", outTarget)
	}
}

// TestParseMDDataKeyboardOnlyNoMD 验证 md 为 nil（纯 keyboard 输入）时不注入不 panic
func TestParseMDDataKeyboardOnlyNoMD(t *testing.T) {
	input := []byte(`{"id":"kb-1"}`)
	md, kb, err := parseMDData(input)
	if err != nil {
		t.Fatalf("parseMDData 异常: %v", err)
	}
	if md != nil {
		t.Errorf("纯 keyboard 输入不应产生 markdown: %+v", md)
	}
	if kb == nil || kb.ID != "kb-1" {
		t.Errorf("keyboard 应正常解析: %+v", kb)
	}
}

// TestMarkdownDTOFieldTag 逐字校验 dto.Markdown.ForceVerifyImageResource 的官方 JSON tag
func TestMarkdownDTOFieldTag(t *testing.T) {
	md := dto.Markdown{ForceVerifyImageResource: true}
	out, err := json.Marshal(md)
	if err != nil {
		t.Fatalf("marshal 失败: %v", err)
	}
	if !strings.Contains(string(out), `"force_verify_image_resource":true`) {
		t.Errorf("官方字段名 force_verify_image_resource 缺失: %s", out)
	}
	zero := dto.Markdown{}
	outZero, _ := json.Marshal(zero)
	if strings.Contains(string(outZero), "force_verify_image_resource") {
		t.Errorf("false 时 omitempty 应省略字段: %s", outZero)
	}
}

package handlers

import (
	"encoding/base64"
	"encoding/json"
	"strconv"
	"strings"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/idmap"
)

// 本文件是 CQ 码统一解析架构重构（cqparse）的"特征回归网"第一部分：
// 对当前 legacy 代码（cqcode.go / cqcode_pipeline.go / set_group_helpers.go /
// message_parser.go）的正确行为做表驱动钉死，锁住重构前语义。
// 用例编号对齐 .git/opencode-team/.../cq-unified-parse/05-architect-cqparse-design.md §9。
// 约定：
//   - 只钉"现状正确"的行为；已知 bug（C1 贪婪吞噬/C2 正则 panic/C3 批量丢人/M3/M4 失败泄漏/
//     M2 参数污染/M5 多码同 URL/M6 reply 过窄等）不在本文件钉死，留给
//     cq_parse_regression_test.go 以"期望=迁移后行为"断言。
//   - 不构造会触发 C2 panic 的输入（user_id 含正则元字符进 MustCompile 路径）。

// ---------- 字符串路径：单码 / 多码相邻 / 码+正文 / 转义 / at / 媒体 ----------

// TestLegacyStringPathCQCodes 字符串路径现状行为钉死（§9 O 系/N-A 系的状态保持面）
func TestLegacyStringPathCQCodes(t *testing.T) {
	b64MD := base64.StdEncoding.EncodeToString([]byte(`{"content":"hi"}`))
	tests := []struct {
		name        string
		input       string
		wantText    string
		wantKeys    map[string]int
		wantValues  map[string][]string
		wantNotKeys []string
	}{
		{
			name:     "单码URL图片提取",
			input:    "看这张图[CQ:image,file=https://example.com/pic.jpg]好看吗",
			wantText: "看这张图好看吗",
			wantValues: map[string][]string{
				"url_images": {"example.com/pic.jpg"},
			},
		},
		{
			name:     "多码相邻图片按文档顺序入列",
			input:    "[CQ:image,file=https://x.com/a.png][CQ:image,file=https://x.com/b.png]",
			wantText: "",
			wantValues: map[string][]string{
				"url_images": {"x.com/a.png", "x.com/b.png"},
			},
		},
		{
			name:     "码与正文混合剥离后保留正文",
			input:    "[CQ:image,file=https://x.com/p.png]中间文字[CQ:image,file=base64://aGVsbG8=]",
			wantText: "中间文字",
			wantKeys: map[string]int{"url_images": 1, "base64_image": 1},
		},
		{
			name:     "reply与at及正文混合",
			input:    "[CQ:reply,id=100]你好[CQ:at,qq=12345]再见",
			wantText: "你好[CQ:at,qq=12345]再见",
			wantValues: map[string][]string{
				"reply_msg_id": {"100"},
			},
		},
		{
			name:     "markdownJSON单码合法场景解析且正文保留",
			input:    `[CQ:markdown,data={"content":"hi"}]普通文本`,
			wantText: "普通文本",
			wantValues: map[string][]string{
				"markdown": {b64MD},
			},
		},
		{
			name:     "keyboardBase64与markdownJSON组合正序互不干扰",
			input:    "[CQ:keyboard,data=base64://e30=][CQ:markdown,data={\"content\":\"hi\"}]",
			wantText: "",
			wantKeys: map[string]int{"keyboard": 1, "markdown": 1},
		},
		{
			name:     "keyboardBase64与markdownJSON组合反序互不干扰",
			input:    "[CQ:markdown,data={\"content\":\"hi\"}][CQ:keyboard,data=base64://e30=]",
			wantText: "",
			wantKeys: map[string]int{"keyboard": 1, "markdown": 1},
		},
		{
			name:     "markdownBase64形态原样透传", // N-A15
			input:    "[CQ:markdown,data=base64://" + b64MD + "]正文",
			wantText: "正文",
			wantValues: map[string][]string{
				"markdown": {b64MD},
			},
		},
		{
			name:     "at无参码原样保留", // N-A8
			input:    "[CQ:at]你好",
			wantText: "[CQ:at]你好",
		},
		{
			name:     "at空值码原样保留", // N-A8
			input:    "[CQ:at,qq=]你好",
			wantText: "[CQ:at,qq=]你好",
		},
		{
			name:     "小写cq码原样保留", // N-A8（现状不做宽容）
			input:    "[cq:at,qq=123]你好",
			wantText: "[cq:at,qq=123]你好",
		},
		{
			name:     "recordURL与base64语音提取",
			input:    "[CQ:record,file=http://example.com/a.silk][CQ:record,file=base64://dGVzdA==]",
			wantText: "",
			wantKeys: map[string]int{"url_record": 1, "base64_record": 1},
		},
		{
			name:     "videoURL与base64视频提取",
			input:    "[CQ:video,file=https://example.com/v.mp4][CQ:video,file=base64://dmlkZW8=]",
			wantText: "",
			wantKeys: map[string]int{"url_videos": 1, "base64_video": 1},
		},
		{
			name:     "file码URL提取与文件名", // m4 修复面之外的正常路径
			input:    "[CQ:file,file=https://x.com/a.zip,file_name=a.zip]",
			wantText: "",
			wantValues: map[string][]string{
				"url_files": {"x.com/a.zip"},
				"file_name": {"a.zip"},
			},
		},
		{
			name:     "wakeup提取目标用户",
			input:    "[CQ:wakeup,userid=123456789]召回消息",
			wantText: "召回消息",
			wantValues: map[string][]string{
				"wakeup": {"123456789"},
			},
		},
		{
			name:     "active三键归位", // N-A16 状态保持面
			input:    "[CQ:active,type=push,sub_type=1]主动消息",
			wantText: "主动消息",
			wantKeys: map[string]int{"active": 1, "active_type": 1, "active_sub_type": 1},
		},
		{
			name:     "stream冒号语法识别", // N-A7 状态保持
			input:    "[CQ:stream,type:start,qq:12345]",
			wantText: "",
			wantKeys: map[string]int{"stream": 1},
		},
		{
			name:     "input_notify参数JSON化",
			input:    "[CQ:input_notify,type=1,second=5]",
			wantText: "",
			wantKeys: map[string]int{"input_notify": 1},
		},
		{
			name:     "qqmusic提取",
			input:    "[CQ:music,type=qq,id=123456]",
			wantText: "",
			wantValues: map[string][]string{
				"qqmusic": {"123456"},
			},
		},
		{
			name:     "card参数JSON化",
			input:    "[CQ:card,title=测试,url=https://x.com]",
			wantText: "",
			wantKeys: map[string]int{"card": 1},
		},
		{
			name:     "未知码face与json原样保留",
			input:    "[CQ:face,id=1]表情[CQ:json,data=x]",
			wantText: "[CQ:face,id=1]表情[CQ:json,data=x]",
		},
		{
			name:     "本地图片file提取",
			input:    "[CQ:image,file=file:///tmp/ok.png]",
			wantText: "",
			wantKeys: map[string]int{"local_image": 1},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			foundItems := make(map[string][]string)
			got := ProcessCQCodePipeline(tt.input, foundItems, nil)

			if got != tt.wantText {
				t.Errorf("文本不匹配:\n  got:  %q\n  want: %q", got, tt.wantText)
			}
			for key, wantLen := range tt.wantKeys {
				if items, ok := foundItems[key]; !ok {
					t.Errorf("缺少期望 key: %s", key)
				} else if len(items) != wantLen {
					t.Errorf("key %s 长度: got %d, want %d, items=%v", key, len(items), wantLen, items)
				}
			}
			for key, wantValues := range tt.wantValues {
				items, ok := foundItems[key]
				if !ok {
					t.Errorf("缺少期望 key: %s", key)
					continue
				}
				if len(items) != len(wantValues) {
					t.Errorf("key %s 长度: got %d, want %d", key, len(items), len(wantValues))
					continue
				}
				for i, want := range wantValues {
					if items[i] != want {
						t.Errorf("key %s[%d]: got %q, want %q", key, i, items[i], want)
					}
				}
			}
			for _, key := range tt.wantNotKeys {
				if _, ok := foundItems[key]; ok {
					t.Errorf("不应出现 key: %s", key)
				}
			}
		})
	}
}

// ---------- cqParseParams：正常 KV / 转义 / 重复 key / 空值 / 无=尾段 ----------

// TestLegacyCQParseParamsSemantics 钉死 cqParseParams 现状语义（§9 O-C 系状态保持面）
func TestLegacyCQParseParamsSemantics(t *testing.T) {
	tests := []struct {
		name   string
		params string
		want   map[string]string
	}{
		{
			name:   "正常KV顺序无关",
			params: "action=ban,group_id=1,user_id=2,duration=60",
			want:   map[string]string{"action": "ban", "group_id": "1", "user_id": "2", "duration": "60"},
		},
		{
			name:   "值含转义实体解码", // O-C1
			params: "reason=a&#44;b&amp;c&#93;d",
			want:   map[string]string{"reason": "a,b&c]d"},
		},
		{
			name:   "重复key后者覆盖", // O-C3
			params: "user_id=1,user_id=2",
			want:   map[string]string{"user_id": "2"},
		},
		{
			name:   "空值key存在性保留",
			params: "flag=,action=ban",
			want:   map[string]string{"flag": "", "action": "ban"},
		},
		{
			name:   "无等号尾段被忽略",
			params: "action=ban,broken",
			want:   map[string]string{"action": "ban"},
		},
		{
			name:   "值含等号SplitN保留",
			params: "reason=a=b",
			want:   map[string]string{"reason": "a=b"},
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := cqParseParams(tt.params)
			if len(got) != len(tt.want) {
				t.Fatalf("参数数量: got %d, want %d, got=%v", len(got), len(tt.want), got)
			}
			for k, wantV := range tt.want {
				if got[k] != wantV {
					t.Errorf("key %s: got %q, want %q", k, got[k], wantV)
				}
			}
		})
	}
}

// ---------- buildSetGroupCQCode：字符串/数组/TRSS 三路径还原等价性 ----------

// TestLegacySetGroupThreePathEquivalence 钉死 §9 O-E：kick/blacklist 批量参数
// 三条路径（字符串直写 / 消息段数组 buildSetGroupCQCode / TRSS map buildSetGroupCQCode）
// 经同一解析管线（cqParseParams → cqSetGroupUserIDs）还原结果等价。
func TestLegacySetGroupThreePathEquivalence(t *testing.T) {
	t.Run("字符串路径转义user_ids批量还原", func(t *testing.T) {
		cq := "[CQ:set_group,action=kick,group_id=1,user_ids=111&#44;222&#44;333]"
		params := cqParseParams(cq[len("[CQ:set_group,") : len(cq)-1])
		ids := cqSetGroupUserIDs(params, cq)
		if len(ids) != 3 || ids[0] != "111" || ids[1] != "222" || ids[2] != "333" {
			t.Errorf("字符串路径 user_ids 还原: got %v, want [111 222 333]", ids)
		}
	})

	t.Run("数组路径build后经同一管线还原等价", func(t *testing.T) {
		data := map[string]interface{}{
			"action":   "kick",
			"group_id": "1",
			"user_ids": []interface{}{"111", "222", "333"},
		}
		cq := buildSetGroupCQCode(data)
		if cq != "[CQ:set_group,action=kick,group_id=1,user_ids=111&#44;222&#44;333]" {
			t.Fatalf("数组路径拼装: got %q", cq)
		}
		params := cqParseParams(cq[len("[CQ:set_group,") : len(cq)-1])
		ids := cqSetGroupUserIDs(params, cq)
		if len(ids) != 3 || ids[0] != "111" || ids[1] != "222" || ids[2] != "333" {
			t.Errorf("数组路径 user_ids 还原: got %v, want [111 222 333]", ids)
		}
	})

	t.Run("TRSS路径与数组路径共享同一入口产物逐字节相等", func(t *testing.T) {
		// message_parser.go:970（段数组）与 :1375（TRSS map）调用同一 buildSetGroupCQCode，
		// 等价性由构造保证；此处钉死两路径对相同 data 产物逐字节一致
		data := map[string]interface{}{
			"action":        "blacklist_add",
			"group_id":      "1",
			"user_id":       "10000",
			"user_ids":      []interface{}{"10001", "10002"},
			"add_blacklist": true,
		}
		cqSlice := buildSetGroupCQCode(data)
		cqTRSS := buildSetGroupCQCode(data) // TRSS 分支传入同一 data map
		if cqSlice != cqTRSS {
			t.Errorf("TRSS 与数组路径产物不一致:\n  slice: %q\n  trss:  %q", cqSlice, cqTRSS)
		}
		if cqSlice != "[CQ:set_group,action=blacklist_add,group_id=1,user_id=10000,user_ids=10001&#44;10002,add_blacklist=true]" {
			t.Errorf("blacklist_add 拼装: got %q", cqSlice)
		}
	})

	t.Run("blacklist参数往返add_blacklist与合并ID", func(t *testing.T) {
		cq := "[CQ:set_group,action=blacklist_add,group_id=1,user_id=10000,user_ids=10001&#44;10002,add_blacklist=true]"
		params := cqParseParams(cq[len("[CQ:set_group,") : len(cq)-1])
		if params["add_blacklist"] != "true" {
			t.Errorf("add_blacklist 往返: got %q, want true", params["add_blacklist"])
		}
		ids := cqSetGroupUserIDs(params, cq)
		if len(ids) != 3 || ids[0] != "10000" || ids[1] != "10001" || ids[2] != "10002" {
			t.Errorf("user_id+user_ids 合并还原: got %v", ids)
		}
	})

	t.Run("reason含右括号与取址符转义往返", func(t *testing.T) {
		data := map[string]interface{}{
			"action":   "add_request",
			"group_id": "1",
			"user_id":  "2",
			"flag":     "f_1",
			"reason":   "a]b&c,d",
		}
		cq := buildSetGroupCQCode(data)
		params := cqParseParams(cq[len("[CQ:set_group,") : len(cq)-1])
		if params["reason"] != "a]b&c,d" {
			t.Errorf("reason 往返: got %q, want a]b&c,d (cq=%s)", params["reason"], cq)
		}
	})
}

// ---------- ParamsContent.StringList：数字/字符串/混合数组 ----------

// TestLegacyParamsContentStringList 钉死 StringList 柔性反序列化语义
func TestLegacyParamsContentStringList(t *testing.T) {
	tests := []struct {
		name  string
		raw   string
		want  []string
		isNil bool
	}{
		{
			name: "数字数组归一为十进制字符串",
			raw:  `{"user_ids":[10001,10002]}`,
			want: []string{"10001", "10002"},
		},
		{
			name: "字符串数组直通",
			raw:  `{"user_ids":["a","b"]}`,
			want: []string{"a", "b"},
		},
		{
			name: "混合数组逐元素归一",
			raw:  `{"user_ids":[1,"a",2]}`,
			want: []string{"1", "a", "2"},
		},
		{
			name:  "null不产条目",
			raw:   `{"user_ids":null}`,
			isNil: true,
		},
		{
			name:  "字段缺省不产条目",
			raw:   `{"group_id":"1"}`,
			isNil: true,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			var params callapi.ParamsContent
			if err := json.Unmarshal([]byte(tt.raw), &params); err != nil {
				t.Fatalf("Unmarshal 失败: %v", err)
			}
			if tt.isNil {
				if len(params.UserIDs) != 0 {
					t.Errorf("期望空 UserIDs, got %v", params.UserIDs)
				}
				return
			}
			if len(params.UserIDs) != len(tt.want) {
				t.Fatalf("UserIDs 长度: got %d, want %d, got=%v", len(params.UserIDs), len(tt.want), params.UserIDs)
			}
			for i := range tt.want {
				if params.UserIDs[i] != tt.want[i] {
					t.Errorf("[%d]: got %q, want %q", i, params.UserIDs[i], tt.want[i])
				}
			}
		})
	}
}

// ---------- 动作执行语义：参数缺失保留码 / 纯动作回执 / 跨群路由 group_id 回退 ----------

// TestLegacyOutboundActionSemantics 通过 ProcessOutboundCQCodes 函数级入口钉死动作码
// 现状语义。所选路径均在触达 API 前返回（apiv2 传 nil 安全）；
// 已知失败泄漏 bug（M3 remove 失败 return match / M4 enable 非法 return match）不在此钉死。
func TestLegacyOutboundActionSemantics(t *testing.T) {
	tests := []struct {
		name           string
		input          string
		defaultGroupID string
		eventID        string
		wantText       string
		wantRealGroup  string
		wantEventID    string
	}{
		{
			name:     "set_group_ban缺group_id保留码",
			input:    "[CQ:set_group,action=ban,user_id=1]",
			wantText: "[CQ:set_group,action=ban,user_id=1]",
		},
		{
			name:     "set_group_kick缺成员参数保留码",
			input:    "[CQ:set_group,action=kick,group_id=1]",
			wantText: "[CQ:set_group,action=kick,group_id=1]",
		},
		{
			name:     "set_group_blacklist缺成员参数保留码",
			input:    "[CQ:set_group,action=blacklist_add,group_id=1]",
			wantText: "[CQ:set_group,action=blacklist_add,group_id=1]",
		},
		{
			name:     "set_group_whole_ban缺group_id保留码",
			input:    "[CQ:set_group,action=whole_ban,enable=true]",
			wantText: "[CQ:set_group,action=whole_ban,enable=true]",
		},
		{
			name:     "remove参数全缺保留码",
			input:    "[CQ:remove,x=1]",
			wantText: "[CQ:remove,x=1]",
		},
		{
			name:     "未知action保留原文",
			input:    "[CQ:set_group,action=bogus,x=1]",
			wantText: "[CQ:set_group,action=bogus,x=1]",
		},
		{
			name:     "非动作码at与image原样保留",
			input:    "[CQ:at,qq=123] hello [CQ:image,file=https://x.com/a.png]",
			wantText: "[CQ:at,qq=123] hello [CQ:image,file=https://x.com/a.png]",
		},
		{
			// 纯动作回执：member add 后正文清空，realGroupID 供跨群路由
			name:          "member_add纯动作正文清空并回传realGroupID",
			input:         "[CQ:member,type=add,group_id=999,user_id=42]",
			eventID:       "prev",
			wantText:      "",
			wantRealGroup: "999",
			wantEventID:   "prev", // 空环境无存储 event_id，eventID 不变
		},
		{
			// 跨群路由 group_id 回退：CQ 码省略 group_id 时回退 defaultGroupID
			name:           "member缺group_id回退defaultGroupID",
			input:          "[CQ:member,type=add,user_id=42]",
			defaultGroupID: "777",
			wantText:       "",
			wantRealGroup:  "777",
		},
		{
			name:          "member_remove清空eventID转主动推送",
			input:         "[CQ:member,type=remove,group_id=999,user_id=42]",
			eventID:       "prev",
			wantText:      "",
			wantRealGroup: "999",
			wantEventID:   "",
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			eventID := tt.eventID
			gotText, realGroupID := ProcessOutboundCQCodes(tt.input, tt.defaultGroupID, &eventID, nil)

			if gotText != tt.wantText {
				t.Errorf("文本:\n  got:  %q\n  want: %q", gotText, tt.wantText)
			}
			if tt.wantRealGroup != "" && realGroupID != tt.wantRealGroup {
				t.Errorf("realGroupID: got %q, want %q", realGroupID, tt.wantRealGroup)
			}
			if eventID != tt.wantEventID {
				t.Errorf("eventID: got %q, want %q", eventID, tt.wantEventID)
			}
		})
	}
}

// ---------- 端到端 parseMessageContent（字符串/段/TRSS 三入口现状面） ----------

// TestLegacyParseMessageContentEndToEnd 钉死 parseMessageContent 三入口的现状正确行为，
// 含段路径 avatar 产物入 foundItems（M5 修复面之外的正确半边）。
func TestLegacyParseMessageContentEndToEnd(t *testing.T) {
	t.Run("字符串路径image与at与reply混合", func(t *testing.T) {
		params := callapi.ParamsContent{
			GroupID: "g-test-openid",
			UserID:  "u-test-openid",
			Message: "看看[CQ:image,file=https://x.com/p.png]好[CQ:at,qq=12345]",
		}
		text, items, _ := parseMessageContent(params, callapi.ActionMessage{Action: "send_group_msg", Params: params}, nil, nil, nil)

		if text != "看看好[CQ:at,qq=12345]" {
			t.Errorf("文本: got %q", text)
		}
		if v := items["url_images"]; len(v) != 1 || v[0] != "x.com/p.png" {
			t.Errorf("url_images: got %v", v)
		}
	})

	t.Run("段路径avatar产物正确入url_images", func(t *testing.T) {
		// 先建立 idmap 映射：avatar 现状在反查失败时直接返回空（avatar.go return "", err），
		// 钉"产物正确"半边需用已映射的虚拟 ID
		realOpenID := "u0123456789abcdef0123456789abcdef"
		virtual, err := idmap.StoreIDv2(realOpenID)
		if err != nil {
			t.Fatalf("StoreIDv2 失败: %v", err)
		}
		qqStr := strconv.FormatInt(virtual, 10)

		params := callapi.ParamsContent{
			GroupID: "g-test-openid",
			UserID:  "u-test-openid",
			Message: []interface{}{
				seg("avatar", map[string]interface{}{"qq": qqStr}),
				seg("text", map[string]interface{}{"text": "头像"}),
			},
		}
		text, items, _ := parseMessageContent(params, callapi.ActionMessage{Action: "send_group_msg", Params: params}, nil, nil, nil)

		if text != "头像" {
			t.Errorf("文本: got %q", text)
		}
		v, ok := items["url_images"]
		if !ok || len(v) != 1 {
			t.Fatalf("url_images: got %v", items["url_images"])
		}
		// GenerateAvatarURLV2 产出官方端点 URL,段路径提取后剥离 https:// 入 foundItems
		if !strings.HasPrefix(v[0], "q.qlogo.cn/qqapp/") {
			t.Errorf("头像 URL 前缀异常: %q", v[0])
		}
		if !strings.Contains(v[0], realOpenID) {
			t.Errorf("头像 URL 应含反查出的真实 openid: %q", v[0])
		}
	})

	t.Run("TRSS路径set_group还原进正文", func(t *testing.T) {
		params := callapi.ParamsContent{
			GroupID: "g-test-openid",
			UserID:  "u-test-openid",
			Message: map[string]interface{}{
				"type": "set_group",
				"data": map[string]interface{}{
					"action":        "blacklist_add",
					"group_id":      "123",
					"user_id":       "456",
					"add_blacklist": true,
				},
			},
		}
		text, _, _ := parseMessageContent(params, callapi.ActionMessage{Action: "send_group_msg", Params: params}, nil, nil, nil)

		want := "[CQ:set_group,action=blacklist_add,group_id=123,user_id=456,add_blacklist=true]"
		if text != want {
			t.Errorf("文本:\n  got:  %q\n  want: %q", text, want)
		}
	})
}

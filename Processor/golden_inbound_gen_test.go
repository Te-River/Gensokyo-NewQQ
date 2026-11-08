//go:build goldengen

package Processor

// 黄金样本生成器（任务 #58a）：用真 Processor 的 legacy 入站转换链路产出
// OneBot V11 事件 JSON 金样，供 Rust 侧回放对齐（#58b）。
//
// 生成命令：
//
//	go test -tags goldengen ./Processor -run TestGoldenGen
//
// 产出：Processor/testdata/inbound/<事件类型>.json + manifest.json。
// 本文件自带 build tag，默认 `go build ./...` / `go test ./Processor` 不受影响。
// 文件名 golden_ 前缀：本文件自行注入 config 单例并 t.Chdir，不依赖 zzz* 系列的执行顺序。

import (
	"encoding/json"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/structs"
	"github.com/hoshinonyaruko/gensokyo/template"
	"github.com/tencent-connect/botgo/dto"
)

// goldenSettleTimeout 等待广播 goroutine 收敛的上限：Process* 里多数上报是
// `go p.BroadcastMessageToAll(...)`，不等待就可能读到半截。
const goldenSettleTimeout = 2 * time.Second

// goldenCapture 收集 legacy 广播到正向 WS 客户端的上报 map。
type goldenCapture struct {
	mu   sync.Mutex
	msgs []map[string]interface{}
}

var _ callapi.WebSocketServerClienter = (*goldenCapture)(nil)

func (g *goldenCapture) SendMessage(m map[string]interface{}) error {
	g.mu.Lock()
	g.msgs = append(g.msgs, m)
	g.mu.Unlock()
	return nil
}

func (g *goldenCapture) Close() error { return nil }

func (g *goldenCapture) len() int {
	g.mu.Lock()
	defer g.mu.Unlock()
	return len(g.msgs)
}

// snapshot 深拷贝已捕获的上报（map 在广播链里会被 applyOpUserIDType 就地改写）。
func (g *goldenCapture) snapshot() ([]map[string]interface{}, error) {
	g.mu.Lock()
	defer g.mu.Unlock()
	out := make([]map[string]interface{}, 0, len(g.msgs))
	for _, m := range g.msgs {
		b, err := json.Marshal(m)
		if err != nil {
			return nil, err
		}
		var copied map[string]interface{}
		if err := json.Unmarshal(b, &copied); err != nil {
			return nil, err
		}
		out = append(out, copied)
	}
	return out, nil
}

// goldenCase 一个事件类型的驱动单元。
type goldenCase struct {
	event   string // QQ 事件类型，同时是 fixture 文件名
	handler string // main.go 里注册的 handler 名
	process string // 实际调用的 Process* 方法
	// envelope 与真实 WS 推送的 d 字段同形，直接 json.Unmarshal 到对应 dto。
	envelope string
	run      func(p *Processors, raw []byte) error
}

// goldenCases 覆盖 main.go getHandlerByName 里会产出 OneBot 事件的入站类型
// （ReadyHandler / ErrorNotifyHandler 不产出事件，未纳入）。
func goldenCases() []goldenCase {
	return []goldenCase{
		{
			event:    "GROUP_AT_MESSAGE_CREATE",
			handler:  "GroupATMessageEventHandler",
			process:  "ProcessGroupMessage",
			envelope: `{"id":"MSG_AT_1","content":"hi","group_id":"GROUP_OPENID_1","author":{"id":"USER_OPENID_1","username":"tester","member_role":"member"},"timestamp":"2024-01-01T00:00:00+08:00","message_scene":{"ext":["msg_idx=REFIDX_1"]}}`,
			run: func(p *Processors, raw []byte) error {
				var d dto.WSGroupATMessageData
				if err := json.Unmarshal(raw, &d); err != nil {
					return err
				}
				return p.ProcessGroupMessage(&d)
			},
		},
		{
			event:    "GROUP_MESSAGE_CREATE",
			handler:  "GroupMessageEventHandler",
			process:  "ProcessGroupNormalMessage",
			envelope: `{"id":"MSG_FULL_1","content":"hello group","group_id":"GROUP_OPENID_1","author":{"id":"USER_OPENID_1","username":"tester","member_role":"member"},"timestamp":"2024-01-01T00:00:00+08:00"}`,
			run: func(p *Processors, raw []byte) error {
				var d dto.WSGroupMessageData
				if err := json.Unmarshal(raw, &d); err != nil {
					return err
				}
				return p.ProcessGroupNormalMessage(&d)
			},
		},
		{
			event:    "C2C_MESSAGE_CREATE",
			handler:  "C2CMessageEventHandler",
			process:  "ProcessC2CMessage",
			envelope: `{"id":"MSG_C2C_1","content":"hello","author":{"id":"USER_OPENID_1","username":"tester"},"timestamp":"2024-01-01T00:00:00+08:00"}`,
			run: func(p *Processors, raw []byte) error {
				var d dto.WSC2CMessageData
				if err := json.Unmarshal(raw, &d); err != nil {
					return err
				}
				return p.ProcessC2CMessage(&d)
			},
		},
		{
			event:    "GROUP_MEMBER_ADD",
			handler:  "GroupMemberAddEventHandler",
			process:  "ProcessGroupMember(GROUP_MEMBER_ADD)",
			envelope: `{"id":"EVENT_MEMBER_ADD_1","group_openid":"GROUP_OPENID_1","member_openid":"USER_OPENID_2","timestamp":1704067200}`,
			run: func(p *Processors, raw []byte) error {
				var d dto.GroupMemberEvent
				if err := json.Unmarshal(raw, &d); err != nil {
					return err
				}
				// 对应 main.go:847：handler 把外层 WSPayload.ID 写进 data.EventID。
				d.EventID = d.ID
				p.ProcessGroupMember(&d, "GROUP_MEMBER_ADD")
				return nil
			},
		},
		{
			event:    "GROUP_MEMBER_REMOVE",
			handler:  "GroupMemberRemoveEventHandler",
			process:  "ProcessGroupMember(GROUP_MEMBER_REMOVE)",
			envelope: `{"id":"EVENT_MEMBER_REMOVE_1","group_openid":"GROUP_OPENID_1","member_openid":"USER_OPENID_2","timestamp":1704067200}`,
			run: func(p *Processors, raw []byte) error {
				var d dto.GroupMemberEvent
				if err := json.Unmarshal(raw, &d); err != nil {
					return err
				}
				p.ProcessGroupMember(&d, "GROUP_MEMBER_REMOVE")
				return nil
			},
		},
		{
			event:    "GROUP_JOIN_REQUEST",
			handler:  "GroupJoinRequestEventHandler",
			process:  "ProcessGroupJoinRequest",
			envelope: `{"id":"EVENT_JOIN_1","group_openid":"GROUP_OPENID_1","join_request_id":"JOIN_REQUEST_1","member_openid":"USER_OPENID_3","username":"applicant","apply_at":"2024-01-01T00:00:00+08:00","verify_info":{"method":"admin_confirm","verify_message":"let me in"}}`,
			run: func(p *Processors, raw []byte) error {
				var d dto.GroupJoinRequestEvent
				if err := json.Unmarshal(raw, &d); err != nil {
					return err
				}
				// 对应 main.go:864：handler 把外层 WSPayload.ID 写进 data.EventID。
				d.EventID = d.ID
				p.ProcessGroupJoinRequest(&d)
				return nil
			},
		},
		{
			event:    "INTERACTION_CREATE",
			handler:  "InteractionHandler",
			process:  "ProcessInlineSearch",
			envelope: `{"id":"INTERACTION_1","event_id":"EVENT_INTERACTION_1","type":11,"scene":"group","chat_type":1,"timestamp":"2024-01-01T00:00:00+08:00","group_openid":"GROUP_OPENID_1","group_member_openid":"USER_OPENID_1","data":{"name":"button","type":1,"resolved":{"button_data":"cmd_1","button_id":"BTN_1","user_id":"USER_OPENID_1","feature_id":"FEATURE_1","message_id":"MSG_AT_1"}}}`,
			run: func(p *Processors, raw []byte) error {
				var d dto.WSInteractionData
				if err := json.Unmarshal(raw, &d); err != nil {
					return err
				}
				return p.ProcessInlineSearch(&d)
			},
		},
		{
			event:    "FRIEND_ADD",
			handler:  "FriendAddEventHandler",
			process:  "ProcessFriendAdd",
			envelope: `{"event_id":"EVENT_FRIEND_ADD_1","openid":"USER_OPENID_4","timestamp":1704067200,"scene":1,"scene_param":"callback_data_1"}`,
			run: func(p *Processors, raw []byte) error {
				var d dto.WSFriendAddData
				if err := json.Unmarshal(raw, &d); err != nil {
					return err
				}
				return p.ProcessFriendAdd(&d)
			},
		},
		{
			event:    "FRIEND_DEL",
			handler:  "FriendDelEventHandler",
			process:  "ProcessFriendDel",
			envelope: `{"event_id":"EVENT_FRIEND_DEL_1","openid":"USER_OPENID_4","timestamp":1704067200}`,
			run: func(p *Processors, raw []byte) error {
				var d dto.WSFriendDelData
				if err := json.Unmarshal(raw, &d); err != nil {
					return err
				}
				return p.ProcessFriendDel(&d)
			},
		},
		{
			event:    "GROUP_ADD_ROBOT",
			handler:  "GroupAddRobotEventHandler",
			process:  "ProcessGroupAddBot",
			envelope: `{"id":"EVENT_ADD_ROBOT_1","event_id":"EVENT_ADD_ROBOT_1","group_openid":"GROUP_OPENID_5","op_member_openid":"USER_OPENID_5","timestamp":1704067200}`,
			run: func(p *Processors, raw []byte) error {
				var d dto.GroupAddBotEvent
				if err := json.Unmarshal(raw, &d); err != nil {
					return err
				}
				return p.ProcessGroupAddBot(&d)
			},
		},
		{
			event:    "GROUP_DEL_ROBOT",
			handler:  "GroupDelRobotEventHandler",
			process:  "ProcessGroupDelBot",
			envelope: `{"id":"EVENT_DEL_ROBOT_1","event_id":"EVENT_DEL_ROBOT_1","group_openid":"GROUP_OPENID_5","op_member_openid":"USER_OPENID_5","timestamp":1704067200}`,
			run: func(p *Processors, raw []byte) error {
				var d dto.GroupAddBotEvent
				if err := json.Unmarshal(raw, &d); err != nil {
					return err
				}
				return p.ProcessGroupDelBot(&d)
			},
		},
		{
			event:    "GROUP_MSG_REJECT",
			handler:  "GroupMsgRejectHandler",
			process:  "ProcessGroupMsgReject",
			envelope: `{"event_id":"EVENT_MSG_REJECT_1","group_openid":"GROUP_OPENID_5","op_member_openid":"USER_OPENID_5","timestamp":1704067200}`,
			run: func(p *Processors, raw []byte) error {
				var d dto.GroupMsgRejectEvent
				if err := json.Unmarshal(raw, &d); err != nil {
					return err
				}
				return p.ProcessGroupMsgReject(&d)
			},
		},
		{
			event:    "GROUP_MSG_RECEIVE",
			handler:  "GroupMsgReceiveHandler",
			process:  "ProcessGroupMsgRecive",
			envelope: `{"event_id":"EVENT_MSG_RECEIVE_1","group_openid":"GROUP_OPENID_5","op_member_openid":"USER_OPENID_5","timestamp":1704067200}`,
			run: func(p *Processors, raw []byte) error {
				var d dto.GroupMsgReceiveEvent
				if err := json.Unmarshal(raw, &d); err != nil {
					return err
				}
				return p.ProcessGroupMsgRecive(&d)
			},
		},
		{
			event:    "C2C_MSG_REJECT",
			handler:  "C2CMsgRejectHandler",
			process:  "ProcessC2CMsgReject",
			envelope: `{"event_id":"EVENT_C2C_REJECT_1","openid":"USER_OPENID_6","timestamp":1704067200}`,
			run: func(p *Processors, raw []byte) error {
				var d dto.WSC2CMsgRejectData
				if err := json.Unmarshal(raw, &d); err != nil {
					return err
				}
				return p.ProcessC2CMsgReject(&d)
			},
		},
		{
			event:    "C2C_MSG_RECEIVE",
			handler:  "C2CMsgReceiveHandler",
			process:  "ProcessC2CMsgReceive",
			envelope: `{"event_id":"EVENT_C2C_RECEIVE_1","openid":"USER_OPENID_6","timestamp":1704067200}`,
			run: func(p *Processors, raw []byte) error {
				var d dto.WSC2CMsgReceiveData
				if err := json.Unmarshal(raw, &d); err != nil {
					return err
				}
				return p.ProcessC2CMsgReceive(&d)
			},
		},
	}
}

// goldenConfigYAML 以完整模板为基底覆写生成用配置；每条都必须命中，
// 否则说明 template.ConfigTemplate 漂移，需要同步更新本列表。
func goldenConfigYAML(t *testing.T) string {
	t.Helper()
	type patch struct{ old, new string }
	patches := []patch{
		{`arch_mode : new`, `arch_mode : legacy`},
		{`string_ob11 : false`, `string_ob11 : true`},
		{`native_ob11 : false`, `native_ob11 : true`},
		{`enable_ws_server: true`, `enable_ws_server: false`},
		{`webhook_path : "webhook"`, `webhook_path : ""`},
		{`["ws://<YOUR_WS_ADDRESS>:<YOUR_WS_PORT>"]`, `[]`},
	}
	yml := template.ConfigTemplate
	for _, p := range patches {
		if !strings.Contains(yml, p.old) {
			t.Fatalf("配置模板中找不到 %q，生成器需要随模板更新", p.old)
		}
		yml = strings.Replace(yml, p.old, p.new, 1)
	}
	return yml
}

// goldenSettle 等到广播收敛（连续 3 次采样条数不变）后返回深拷贝的上报。
func goldenSettle(c *goldenCapture) ([]map[string]interface{}, error) {
	deadline := time.Now().Add(goldenSettleTimeout)
	last, stable := -1, 0
	for time.Now().Before(deadline) {
		if n := c.len(); n == last {
			stable++
			if stable >= 3 {
				break
			}
		} else {
			last, stable = n, 0
		}
		time.Sleep(20 * time.Millisecond)
	}
	return c.snapshot()
}

// goldenNormalize 把不可复现字段归零：键名为 time 的一律置 0
// （构造时取 time.Now()，见 ProcessGroupMessage.go 的 Time 字段等 23 处）。
func goldenNormalize(v interface{}) interface{} {
	switch t := v.(type) {
	case map[string]interface{}:
		for k, x := range t {
			if k == "time" {
				t[k] = float64(0)
				continue
			}
			t[k] = goldenNormalize(x)
		}
		return t
	case []interface{}:
		for i := range t {
			t[i] = goldenNormalize(t[i])
		}
		return t
	}
	return v
}

// goldenSortPayloads 按紧凑 JSON 字典序排序：多个上报由独立 goroutine 广播，
// 到达顺序不确定，排序后才能逐字节复现。
func goldenSortPayloads(payloads []map[string]interface{}) error {
	keys := make([]string, len(payloads))
	for i, m := range payloads {
		b, err := json.Marshal(m)
		if err != nil {
			return err
		}
		keys[i] = string(b)
	}
	idx := make([]int, len(payloads))
	for i := range idx {
		idx[i] = i
	}
	sort.Slice(idx, func(a, b int) bool { return keys[idx[a]] < keys[idx[b]] })
	sorted := make([]map[string]interface{}, len(payloads))
	for i, j := range idx {
		sorted[i] = payloads[j]
	}
	copy(payloads, sorted)
	return nil
}

// goldenWriteJSON 以 2 空格缩进写盘（map 键由 encoding/json 天然字母序）。
// 关闭 HTML 转义：fixtures 里可能带 <@OpenID> 原文，转义后不便人工比对。
func goldenWriteJSON(t *testing.T, path string, v interface{}) {
	t.Helper()
	f, err := os.Create(path)
	if err != nil {
		t.Fatalf("创建 %s 失败: %v", path, err)
	}
	defer f.Close()
	enc := json.NewEncoder(f)
	enc.SetEscapeHTML(false)
	enc.SetIndent("", "  ")
	if err := enc.Encode(v); err != nil {
		t.Fatalf("写入 %s 失败: %v", path, err)
	}
}

// goldenManifestEvent 记录单个事件类型的来源与产出条数。
type goldenManifestEvent struct {
	Event    string `json:"event"`
	Handler  string `json:"handler"`
	Process  string `json:"process"`
	Payloads int    `json:"payloads"`
}

type goldenManifest struct {
	Generator string                `json:"generator"`
	Shape     string                `json:"shape"`
	Events    []goldenManifestEvent `json:"events"`
	Config    map[string]interface{} `json:"config"`
	Normalize []string              `json:"normalize"`
	Notes     []string              `json:"notes"`
}

// TestGoldenGen 生成 Processor/testdata/inbound 下的入站金样。
// 连跑两次字节一致：time 已归零、上报数组已排序、idmap 使用哈希派生 ID。
func TestGoldenGen(t *testing.T) {
	// 输出目录必须在 chdir 之前解析：t.Chdir 会让相对路径落到临时目录。
	outDir, err := filepath.Abs(filepath.Join("testdata", "inbound"))
	if err != nil {
		t.Fatalf("解析输出目录失败: %v", err)
	}
	if err := os.MkdirAll(outDir, 0o755); err != nil {
		t.Fatalf("创建输出目录失败: %v", err)
	}

	// idmap（idmap-identity/idmap-msg）在 cwd 建库，且失败会走
	// mylog.Fatalf → os.Exit(1)，所以必须切到空临时目录。
	t.Chdir(t.TempDir())
	// bbolt 句柄不关，Windows 下临时目录清理会失败（The process cannot access the file）。
	defer idmap.CloseDB()

	cfgPath := filepath.Join(t.TempDir(), "config.yml")
	if err := os.WriteFile(cfgPath, []byte(goldenConfigYAML(t)), 0o600); err != nil {
		t.Fatalf("写入测试配置失败: %v", err)
	}
	if _, err := config.LoadConfig(cfgPath, false); err != nil {
		t.Fatalf("加载测试配置失败: %v", err)
	}
	if got := config.GetArchMode(); got != "legacy" || !config.GetStringOb11() || !config.GetNativeOb11() || config.GetArrayValue() {
		t.Fatalf("生成器配置不符: arch_mode=%q string_ob11=%v native_ob11=%v array=%v",
			got, config.GetStringOb11(), config.GetNativeOb11(), config.GetArrayValue())
	}

	manifest := goldenManifest{
		Generator: "go test -tags goldengen ./Processor -run TestGoldenGen",
		Shape:     "每个 <event>.json 为按紧凑 JSON 字典序排序的上报数组，元素是一条 OneBot V11 事件 payload",
		Config: map[string]interface{}{
			"app_id":               config.GetAppID(),
			"arch_mode":            config.GetArchMode(),
			"string_ob11":          config.GetStringOb11(),
			"native_ob11":          config.GetNativeOb11(),
			"array":                config.GetArrayValue(),
			"hash_id":              config.GetHashIDValue(),
			"op_user_id_type":      config.GetOpUserIDType(),
			"two_way_echo":         config.GetTwoWayEcho(),
			"auto_put_interaction": config.GetAutoPutInteraction(),
			"enable_ws_server":     false,
			"http_address":         "",
			"webhook_path":         "",
			"ws_address":           []string{},
		},
		Normalize: []string{
			"键名为 time 的字段一律置 0：构造时取 time.Now()，不可复现（含由输入时间戳派生的事件也一并归零）",
			"上报数组按紧凑 JSON 字典序排序：多条上报由独立 goroutine 广播，到达顺序不确定",
			"输入信封为固定字面量（见 goldengen 生成器源码），不含任何运行时采样值",
			"idmap 虚拟 ID 走模板默认 hash_id=true 的哈希派生，与分配顺序无关，空 cwd 下每次一致",
		},
		Notes: []string{
			"只复刻 main.go handler 内的 Process* 调用；handler 层副作用（runWithTimer / botstats / acnode.CheckWordIN）不在金样范围",
			"ReadyHandler / ErrorNotifyHandler 不产出 OneBot 事件，未纳入",
			"string_ob11=false（数字 ID 模式）需要真实 idmap 分配且编号不可预置，留第二批",
			"p.Api / p.Apiv2 为 nil：本批事件类型在纯文本入参下不会触达 QQ OpenAPI",
		},
	}

	for _, c := range goldenCases() {
		t.Run(c.event, func(t *testing.T) {
			cap := &goldenCapture{}
			p := &Processors{
				Settings:        &structs.Settings{AppID: config.GetAppID()},
				WsServerClients: []callapi.WebSocketServerClienter{cap},
			}
			if err := c.run(p, []byte(c.envelope)); err != nil {
				t.Fatalf("驱动 %s 失败: %v", c.event, err)
			}
			payloads, err := goldenSettle(cap)
			if err != nil {
				t.Fatalf("读取 %s 上报失败: %v", c.event, err)
			}
			if len(payloads) == 0 {
				t.Fatalf("%s 未捕获到任何上报，生成器与 Processor 已脱节", c.event)
			}
			for _, m := range payloads {
				goldenNormalize(m)
			}
			if err := goldenSortPayloads(payloads); err != nil {
				t.Fatalf("排序 %s 上报失败: %v", c.event, err)
			}
			goldenWriteJSON(t, filepath.Join(outDir, c.event+".json"), payloads)
			manifest.Events = append(manifest.Events, goldenManifestEvent{
				Event:    c.event,
				Handler:  c.handler,
				Process:  c.process,
				Payloads: len(payloads),
			})
		})
	}

	goldenWriteJSON(t, filepath.Join(outDir, "manifest.json"), manifest)
}

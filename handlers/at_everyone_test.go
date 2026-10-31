package handlers

import (
	"encoding/base64"
	"errors"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/tencent-connect/botgo/dto"
)

// ---------- @全员 归一 ----------

func TestNormalizeAtEveryone(t *testing.T) {
	cases := []struct {
		name string
		in   string
		want string
	}{
		{"主入口 all", "[CQ:at,qq=all] 公告", atEveryoneTag + " 公告"},
		{"宽松别名 everyone", "[CQ:at,qq=everyone] 公告", atEveryoneTag + " 公告"},
		{"大小写不敏感", "[CQ:at,qq=ALL] 公告", atEveryoneTag + " 公告"},
		{"字面标签原样保留", "公告 " + atEveryoneTag, "公告 " + atEveryoneTag},
		{"普通 at 不受影响", "[CQ:at,qq=12345] 公告", "[CQ:at,qq=12345] 公告"},
		{"无 at 原样", "公告正文", "公告正文"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := NormalizeAtEveryone(tc.in); got != tc.want {
				t.Fatalf("NormalizeAtEveryone(%q) = %q, want %q", tc.in, got, tc.want)
			}
		})
	}
}

// CQ 管道里归一后不得残留 CQ 码（文本与 markdown 同源）。
func TestProcessCQCodePipelineNormalizesAtEveryone(t *testing.T) {
	foundItems := map[string][]string{}
	out := ProcessCQCodePipeline("[CQ:at,qq=all] 公告", foundItems, openID32('g'))
	if out != atEveryoneTag+" 公告" {
		t.Fatalf("管道输出 = %q, want %q", out, atEveryoneTag+" 公告")
	}
	if strings.Contains(out, "[CQ:at") {
		t.Fatalf("不应残留 CQ 码: %q", out)
	}
}

// markdown 载荷内部的 [CQ:at,qq=all] 同样要归一：载荷先 base64 编码，
// 文本侧归一够不到其 content（F6）。
func TestProcessCQCodePipelineNormalizesAtEveryoneInMarkdown(t *testing.T) {
	payload := base64.StdEncoding.EncodeToString([]byte(`{"markdown":{"content":"[CQ:at,qq=all] 公告"}}`))
	foundItems := map[string][]string{}
	ProcessCQCodePipeline("[CQ:markdown,data=base64://"+payload+"]", foundItems, nil)

	items := foundItems["markdown"]
	if len(items) != 1 {
		t.Fatalf("应存下 1 份 markdown 载荷, got %d", len(items))
	}
	decoded, err := base64.StdEncoding.DecodeString(items[0])
	if err != nil {
		t.Fatalf("载荷应是合法 base64: %v", err)
	}
	if !strings.Contains(string(decoded), atEveryoneTag) {
		t.Fatalf("markdown content 内的 CQ 码应归一为官方标签, got %s", decoded)
	}
	if strings.Contains(string(decoded), "[CQ:at,qq=all]") {
		t.Fatalf("markdown content 内不应残留 CQ 码: %s", decoded)
	}
	// 归一后 atEveryonePresent 必须能识别出额度需要占用
	if !atEveryonePresent("", foundItems) {
		t.Fatalf("归一后的 markdown 应被识别为带 @全员")
	}
}

// ---------- @全员 计数（每群每自然日 1 次） ----------

// memAtEveryoneCounter 内存计数，避免单测污染 idmap.db。
type memAtEveryoneCounter struct {
	mu   sync.Mutex
	data map[string]string
}

func newMemAtEveryoneCounter() *memAtEveryoneCounter {
	return &memAtEveryoneCounter{data: map[string]string{}}
}

func (m *memAtEveryoneCounter) Read(key string) (string, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	return m.data[key], nil
}

func (m *memAtEveryoneCounter) Write(key, value string) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	if value == "" {
		delete(m.data, key)
		return nil
	}
	m.data[key] = value
	return nil
}

// useMemAtEveryone 替换计数存储与开关，测试结束后恢复。
func useMemAtEveryone(t *testing.T) *memAtEveryoneCounter {
	t.Helper()
	store := newMemAtEveryoneCounter()
	origStore := atEveryoneStore
	origEnabled := atEveryoneLimitEnabled
	atEveryoneStore = store
	atEveryoneLimitEnabled = func() bool { return true }
	t.Cleanup(func() {
		atEveryoneStore = origStore
		atEveryoneLimitEnabled = origEnabled
	})
	return store
}

func TestGuardAtEveryoneSameDaySecondRejected(t *testing.T) {
	useMemAtEveryone(t)
	group := openID32('g')

	rollback, err := GuardAtEveryone(group, false)
	if err != nil {
		t.Fatalf("首次占用应放行: %v", err)
	}
	if rollback == nil {
		t.Fatalf("应返回回滚函数")
	}
	if _, err := GuardAtEveryone(group, false); err == nil {
		t.Fatalf("同日第二次应被拒绝")
	} else if !strings.Contains(err.Error(), "自然日") {
		t.Fatalf("错误信息应说明自然日限制, got %v", err)
	}
}

func TestGuardAtEveryoneCrossDayAllowed(t *testing.T) {
	store := useMemAtEveryone(t)
	group := openID32('g')
	// 预置昨天的占用记录
	if err := store.Write(group, `{"date":"2000-01-01","count":1}`); err != nil {
		t.Fatalf("预置记录失败: %v", err)
	}
	if _, err := GuardAtEveryone(group, false); err != nil {
		t.Fatalf("跨自然日应放行: %v", err)
	}
}

func TestGuardAtEveryoneRollbackReleasesQuota(t *testing.T) {
	useMemAtEveryone(t)
	group := openID32('g')

	rollback, err := GuardAtEveryone(group, false)
	if err != nil {
		t.Fatalf("首次占用应放行: %v", err)
	}
	rollback() // 模拟发送失败回滚
	rollback() // 幂等：重复调用不应再次改动存储

	if _, err := GuardAtEveryone(group, false); err != nil {
		t.Fatalf("回滚后应可再次发送: %v", err)
	}
}

func TestGuardAtEveryoneDisabledDoesNotLimit(t *testing.T) {
	store := useMemAtEveryone(t)
	atEveryoneLimitEnabled = func() bool { return false }
	group := openID32('g')

	for i := 0; i < 3; i++ {
		if _, err := GuardAtEveryone(group, false); err != nil {
			t.Fatalf("开关关闭时第 %d 次不应被拦: %v", i+1, err)
		}
	}
	if len(store.data) != 0 {
		t.Fatalf("开关关闭时不应写计数, got %v", store.data)
	}
}

// shadow 预演（dry-run）不得占用额度：额度由 legacy 真发送路径占用。
func TestGuardAtEveryoneShadowDryRunDoesNotConsumeQuota(t *testing.T) {
	store := useMemAtEveryone(t)
	group := openID32('g')

	for i := 0; i < 3; i++ {
		rollback, err := GuardAtEveryone(group, true)
		if err != nil {
			t.Fatalf("dry-run 第 %d 次不应被拦: %v", i+1, err)
		}
		if rollback == nil {
			t.Fatalf("dry-run 也必须返回可调用的 rollback")
		}
		rollback()
	}
	if len(store.data) != 0 {
		t.Fatalf("dry-run 不应写计数, got %v", store.data)
	}
}

// 自然日按固定 +08:00 切分：00:30 与 08:30（北京时间）必须落在各自日期。
func TestGuardAtEveryoneUsesFixedCSTDate(t *testing.T) {
	cases := []struct {
		name    string
		utc     time.Time
		wantDay string
	}{
		// UTC 16:30 = 次日 00:30（北京时间）
		{"00:30 边界", time.Date(2026, 10, 31, 16, 30, 0, 0, time.UTC), "2026-11-01"},
		// UTC 00:30 = 当日 08:30（北京时间）
		{"08:30 边界", time.Date(2026, 10, 31, 0, 30, 0, 0, time.UTC), "2026-10-31"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			store := useMemAtEveryone(t)
			now := tc.utc
			origNow := atEveryoneNow
			atEveryoneNow = func() time.Time { return now.In(atEveryoneZone) }
			t.Cleanup(func() { atEveryoneNow = origNow })

			group := openID32('g')
			if _, err := GuardAtEveryone(group, false); err != nil {
				t.Fatalf("占用额度失败: %v", err)
			}
			raw := store.data[group]
			if !strings.Contains(raw, `"date":"`+tc.wantDay+`"`) {
				t.Fatalf("计数日期应为 %s（+08:00），got %s", tc.wantDay, raw)
			}
		})
	}
}

// 固定时区断言：不受主机本地时区影响。
func TestAtEveryoneZoneIsFixedCST(t *testing.T) {
	// 无论主机时区如何，2026-10-31T16:30:00Z 在北京时间必然是 11-01 00:30
	got := time.Date(2026, 10, 31, 16, 30, 0, 0, time.UTC).In(atEveryoneZone).Format("2006-01-02 15:04")
	if got != "2026-11-01 00:30" {
		t.Fatalf("固定时区应为 +08:00, got %s", got)
	}
}

// ---------- @全员 出站（群聊两路 / 单聊拒绝） ----------

func TestSendGroupMsgAtEveryone(t *testing.T) {
	cases := []struct {
		name string
		text string
	}{
		{"CQ 入口", "[CQ:at,qq=all] 公告"},
		{"字面标签入口", "公告 " + atEveryoneTag},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			// 每个子用例独立的计数存储：共享会触发「同日第二次」限流
			useMemAtEveryone(t)
			mock := &cardMockAPI{}
			client := &cardRecorderClient{}
			msg := callapi.ActionMessage{
				Action: "send_group_msg",
				Params: callapi.ParamsContent{
					GroupID: openID32('g'),
					UserID:  openID32('u'),
					Message: tc.text,
				},
				Echo: "at-everyone-" + tc.name,
			}
			if _, err := HandleSendGroupMsg(client, nil, mock, msg); err != nil {
				t.Fatalf("HandleSendGroupMsg 返回错误: %v", err)
			}
			if mock.groupCalls != 1 {
				t.Fatalf("应发送一次 PostGroupMessage, got %d", mock.groupCalls)
			}
			mtc, ok := mock.lastGroup.(*dto.MessageToCreate)
			if !ok {
				t.Fatalf("应发送 MessageToCreate, got %T", mock.lastGroup)
			}
			if !strings.Contains(mtc.Content, atEveryoneTag) {
				t.Fatalf("content 应含官方标签, got %q", mtc.Content)
			}
			if strings.Contains(mtc.Content, "[CQ:at") {
				t.Fatalf("content 不应残留 CQ 码, got %q", mtc.Content)
			}
		})
	}
}

func TestSendGroupMsgAtEveryoneSecondSameDay1400(t *testing.T) {
	useMemAtEveryone(t)
	group := openID32('g')
	send := func(echo string) (string, *cardMockAPI) {
		mock := &cardMockAPI{}
		client := &cardRecorderClient{}
		raw, err := HandleSendGroupMsg(client, nil, mock, callapi.ActionMessage{
			Action: "send_group_msg",
			Params: callapi.ParamsContent{
				GroupID: group,
				UserID:  openID32('u'),
				Message: "[CQ:at,qq=all] 公告",
			},
			Echo: echo,
		})
		if err != nil {
			t.Fatalf("handler 返回错误: %v", err)
		}
		return raw, mock
	}

	if _, mock := send("at-everyone-first"); mock.groupCalls != 1 {
		t.Fatalf("第一次应发送成功, got %d 次调用", mock.groupCalls)
	}
	raw, mock := send("at-everyone-second")
	if got := retcodeOf(t, raw); got != 1400 {
		t.Fatalf("同日第二次应回 1400, got %d (%s)", got, raw)
	}
	if mock.groupCalls != 0 {
		t.Fatalf("超限时不应发送消息")
	}
}

func TestSendPrivateMsgAtEveryoneRejected(t *testing.T) {
	useMemAtEveryone(t)
	mock := &cardMockAPI{}
	client := &cardRecorderClient{}
	raw, err := HandleSendPrivateMsg(client, nil, mock, callapi.ActionMessage{
		Action: "send_private_msg",
		Params: callapi.ParamsContent{
			UserID:  openID32('u'),
			Message: "[CQ:at,qq=all] 公告",
		},
		Echo: "at-everyone-private",
	})
	if err != nil {
		t.Fatalf("handler 返回错误: %v", err)
	}
	if got := retcodeOf(t, raw); got != 1400 {
		t.Fatalf("单聊 @全员 应回 1400, got %d (%s)", got, raw)
	}
	if mock.c2cCalls != 0 {
		t.Fatalf("单聊 @全员 不应发送消息")
	}
}

// ---------- 禁言 30 天截断 ----------

func TestClampMuteDuration(t *testing.T) {
	const thirtyDays = 30 * 24 * 60 * 60
	if got := clampMuteDuration(thirtyDays); got != thirtyDays {
		t.Fatalf("30 天整不应被截断, got %d", got)
	}
	if got := clampMuteDuration(thirtyDays + 1); got != thirtyDays {
		t.Fatalf("30 天 +1 秒应截断为 30 天, got %d", got)
	}
	if got := clampMuteDuration(600); got != 600 {
		t.Fatalf("正常时长不应改动, got %d", got)
	}
	if got := clampMuteDuration(0); got != 0 {
		t.Fatalf("0 表示解除禁言,应原样返回, got %d", got)
	}
}

// 发送失败回滚后当日额度应归还（错误路径不消耗配额）。
func TestSendGroupMsgAtEveryoneRollbackOnSendError(t *testing.T) {
	store := useMemAtEveryone(t)
	group := openID32('g')
	mock := &cardMockAPI{sendErr: errors.New("mock send failure")}
	client := &cardRecorderClient{}
	if _, err := HandleSendGroupMsg(client, nil, mock, callapi.ActionMessage{
		Action: "send_group_msg",
		Params: callapi.ParamsContent{
			GroupID: group,
			UserID:  openID32('u'),
			Message: "[CQ:at,qq=all] 公告",
		},
		Echo: "at-everyone-fail",
	}); err != nil {
		t.Fatalf("handler 返回错误: %v", err)
	}
	if got, _ := store.Read(group); got != "" {
		t.Fatalf("发送失败后计数应已回滚, got %q", got)
	}
}

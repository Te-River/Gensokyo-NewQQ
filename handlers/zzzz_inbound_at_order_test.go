package handlers

// 入站 @ 转换保位回归网：真实环境暴露的"前移/丢失"问题。
//
// 真实案例（NoneBot 侧收到的 OneBot 事件）：
//   @bot at检测 @同道中人 @备用 @晓山瑞希
//     → [at:qq=发送者]at检测  <@备用raw>   （at 被拼到最前面 + mention 丢失/裸残留）
//
// 根因（已修）：
//   1) array 模式 sortMessageSegments 把所有 at 段统一排到最前 → "@前移"；
//   2) AT 路径（GROUP_AT_MESSAGE_CREATE）不注册 mentions(is_you/bot) → @bot 识别
//      失败落入 idmap 反查；is_you 误标发送者时全局 selfAtIDs 被污染 → 发送者 @ 被当作
//      @bot 剥离而"丢失"；
//   3) 反查失败的 <@openid> 在 array 模式残留在文本中、已转换的却前移，相对顺序错乱。
//
// 本文件名使用 zzzz 前缀：测试依赖 config 单例注入（LoadConfig 完整模板），
// 必须在本包其他依赖 instance==nil 的测试之后运行（Go 按文件名字母序执行）。
// config 单例注入后所有 GetXxx() 返回零值默认，与 nil 时行为一致
// （GetCQParseMode 零值同样回退 legacy），不影响既有用例。

import (
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/template"
	"github.com/tencent-connect/botgo/dto"
)

const (
	atBotOpenID    = "BEDAB0A652B5271EF7A8E9EB9ED48B42" // 机器人群场景 OpenID
	atSenderOpenID = "D8FB64288B16F160289247BA495B7233" // 消息发送者（真实案例中的 @同道中人）
	atOther1OpenID = "AAAA1111AAAA1111AAAA1111AAAA1111" // 可反查的他人
	atOther2OpenID = "BBBB2222BBBB2222BBBB2222BBBB2222" // 故意不入 idmap（反查失败样本）
)

// atEnvDir 测试专用工作目录（idmap db / config.yml 落在这里，不污染仓库）。
// 由外层测试创建并注册清理；bbolt 句柄进程内不关闭，Windows 下 RemoveAll
// 可能失败，忽略即可（临时目录）。
var atEnvDir string

func atYAMLBool(b bool) string {
	if b {
		return "true"
	}
	return "false"
}

// atLoadConfig 以完整模板为基底生成配置并加载。
// 字段齐全 → ensureConfigComplete 不会追加/重启；重复加载仅刷新单例。
func atLoadConfig(t *testing.T, removeAt, convertOtherAt bool) {
	t.Helper()
	yml := template.ConfigTemplate
	yml = strings.Replace(yml, "remove_at : false", "remove_at : "+atYAMLBool(removeAt), 1)
	yml = strings.Replace(yml, "convert_other_at : false", "convert_other_at : "+atYAMLBool(convertOtherAt), 1)
	yml = strings.Replace(yml, "hash_id : true", "hash_id : false", 1) // 顺序分配虚拟ID：SENDER=1, OTHER1=2
	path := filepath.Join(atEnvDir, "config.yml")
	if err := os.WriteFile(path, []byte(yml), 0600); err != nil {
		t.Fatalf("写入测试配置失败: %v", err)
	}
	if _, err := config.LoadConfig(path, false); err != nil {
		t.Fatalf("加载测试配置失败: %v", err)
	}
}

// atSenderVID / atOther1VID 预置映射后实际分配到的虚拟 ID（动态读取）。
// 同进程可能有前序测试或历史运行留下的 idmap db，虚拟 ID 不保证是 1/2，
// 断言必须基于回读值而非硬编码。
var (
	atSenderVID string
	atOther1VID string
)

// atSeedIDmap 预置映射：SENDER/OTHER1 可反查；OTHER2 必须反查失败
func atSeedIDmap(t *testing.T) {
	t.Helper()
	for _, pair := range []struct {
		openID string
		vid    *string
	}{
		{atSenderOpenID, &atSenderVID},
		{atOther1OpenID, &atOther1VID},
	} {
		row, err := idmap.StoreIDv2(pair.openID)
		if err != nil {
			t.Fatalf("idmap 预置失败 %s: %v", pair.openID, err)
		}
		*pair.vid = strconv.FormatInt(row, 10)
		// RetrieveVirtualValuev2 返回 (real, virtual, err)
		realV, virtualV, err := idmap.RetrieveVirtualValuev2(pair.openID)
		if err != nil || realV != pair.openID || virtualV != *pair.vid {
			t.Fatalf("idmap 反查校验失败 %s: real=%q virtual=%q want %s err=%v", pair.openID, realV, virtualV, *pair.vid, err)
		}
	}
	if _, _, err := idmap.RetrieveVirtualValuev2(atOther2OpenID); err == nil {
		t.Fatalf("%s 不应有 idmap 映射", atOther2OpenID)
	}
}

// atBeginSubtest 清理全局 selfAtIDs/AppID/BotID，避免子测试间相互污染
func atBeginSubtest(t *testing.T) {
	t.Helper()
	reset := func() {
		selfAtMu.Lock()
		selfAtIDs = map[string]struct{}{}
		selfAtMu.Unlock()
		AppID = "8888"
		BotID = "bot-ready-id"
	}
	reset()
	t.Cleanup(reset)
}

func atStdMentions() []*dto.User {
	return []*dto.User{
		{ID: atBotOpenID, IsYou: true},
		{ID: atSenderOpenID},
		{ID: atOther1OpenID},
		{ID: atOther2OpenID},
	}
}

func atATData(content string, mentions []*dto.User) *dto.WSGroupATMessageData {
	return &dto.WSGroupATMessageData{
		Content:  content,
		Mentions: mentions,
		Author:   &dto.User{ID: atSenderOpenID},
	}
}

func atFullData(content string, mentions []*dto.User) *dto.WSGroupMessageData {
	return &dto.WSGroupMessageData{
		Content:  content,
		Mentions: mentions,
		Author:   &dto.User{ID: atSenderOpenID},
	}
}

// atSegSeq 把段数组序列化成可读形式，便于整序断言
func atSegSeq(segs []map[string]interface{}) string {
	parts := make([]string, 0, len(segs))
	for _, s := range segs {
		data, _ := s["data"].(map[string]interface{})
		switch s["type"] {
		case "at":
			qq, _ := data["qq"].(string)
			parts = append(parts, fmt.Sprintf("at(%s)", qq))
		case "text":
			txt, _ := data["text"].(string)
			parts = append(parts, fmt.Sprintf("text(%q)", txt))
		case "image":
			file, _ := data["file"].(string)
			parts = append(parts, fmt.Sprintf("image(%s)", file))
		default:
			parts = append(parts, fmt.Sprintf("%v", s["type"]))
		}
	}
	return strings.Join(parts, ", ")
}

// atWantSegs 由紧凑序列描述构造期望段数组，描述格式同 atSegSeq 输出。
// 以英文逗号分割（测试内容的文本片断不含英文逗号），text 载荷为 %q 引用形式。
func atWantSegs(desc string) []map[string]interface{} {
	if desc == "" {
		return nil
	}
	var segs []map[string]interface{}
	for _, part := range strings.Split(desc, ",") {
		part = strings.TrimSpace(part)
		switch {
		case strings.HasPrefix(part, "at(") && strings.HasSuffix(part, ")"):
			segs = append(segs, map[string]interface{}{
				"type": "at",
				"data": map[string]interface{}{"qq": part[3 : len(part)-1]},
			})
		case strings.HasPrefix(part, "text(") && strings.HasSuffix(part, ")"):
			txt, err := strconv.Unquote(part[5 : len(part)-1])
			if err != nil {
				panic("无法解析的 text 载荷: " + part)
			}
			segs = append(segs, map[string]interface{}{
				"type": "text",
				"data": map[string]interface{}{"text": txt},
			})
		default:
			panic("无法解析的段描述: " + part)
		}
	}
	return segs
}

func atAssertSegs(t *testing.T, got []map[string]interface{}, wantDesc string) {
	t.Helper()
	want := atWantSegs(wantDesc)
	if len(got) != len(want) {
		t.Fatalf("段数不符:\n  got:  %s\n  want: %s", atSegSeq(got), wantDesc)
	}
	for i := range want {
		gotType, _ := got[i]["type"].(string)
		if gotType != want[i]["type"] {
			t.Fatalf("段[%d] 类型: got %v want %v\n  got:  %s\n  want: %s", i, gotType, want[i]["type"], atSegSeq(got), wantDesc)
		}
		gotData, _ := got[i]["data"].(map[string]interface{})
		wantData, _ := want[i]["data"].(map[string]interface{})
		for k, wantV := range wantData {
			if gotData[k] != wantV {
				t.Fatalf("段[%d].%s: got %v want %v\n  got:  %s\n  want: %s", i, k, gotData[k], wantV, atSegSeq(got), wantDesc)
			}
		}
	}
}

// atAssertConsistency string 模式与 array 模式必须表达同一序列（array 文本片断
// 与 at 段按序拼回后等于 string 输出）。
func atAssertConsistency(t *testing.T, strOut string, segs []map[string]interface{}) {
	t.Helper()
	var b strings.Builder
	for _, s := range segs {
		switch s["type"] {
		case "at":
			data, _ := s["data"].(map[string]interface{})
			qq, _ := data["qq"].(string)
			b.WriteString("[CQ:at,qq=" + qq + "]")
		case "text":
			data, _ := s["data"].(map[string]interface{})
			txt, _ := data["text"].(string)
			b.WriteString(txt)
		}
	}
	if b.String() != strOut {
		t.Errorf("string/array 两路径不一致:\n  string: %q\n  array:  %q", strOut, b.String())
	}
}

func TestInboundAtOrderPreserved(t *testing.T) {
	// 环境只在外层初始化一次：临时目录 + 清理挂外层 t（子测试共享）。
	// t.Chdir 让 idmap db 落在临时目录；本文件在本包测试中最后执行，不影响其他用例。
	dir, err := os.MkdirTemp(os.TempDir(), "at_inbound_test_")
	if err != nil {
		t.Fatalf("创建临时目录失败: %v", err)
	}
	atEnvDir = dir
	t.Chdir(dir)
	t.Cleanup(func() { _ = os.RemoveAll(dir) })

	atSeedIDmap(t)

	contentStd := "<@" + atBotOpenID + "> at检测 <@" + atSenderOpenID + "> <@" + atOther1OpenID + "> <@" + atOther2OpenID + ">"
	rawOther2 := "<@" + atOther2OpenID + ">"

	t.Run("①_atbot加多他人_保位", func(t *testing.T) {
		atBeginSubtest(t)
		atLoadConfig(t, true, true)
		wantStr := "at检测 [CQ:at,qq=" + atSenderVID + "] [CQ:at,qq=" + atOther1VID + "] " + rawOther2
		wantArr := fmt.Sprintf("text(%q),at(%s),text(%q),at(%s),text(%q)", "at检测 ", atSenderVID, " ", atOther1VID, " "+rawOther2)

		gotStr := RevertTransformedText(atATData(contentStd, atStdMentions()), "group", nil, nil, 100, 200, false)
		if gotStr != wantStr {
			t.Errorf("string 模式:\n  got:  %q\n  want: %q", gotStr, wantStr)
		}
		gotArr := ConvertToSegmentedMessage(atATData(contentStd, atStdMentions()))
		atAssertSegs(t, gotArr, wantArr)
		atAssertConsistency(t, gotStr, gotArr)
	})

	t.Run("②_atbot在中间_保位", func(t *testing.T) {
		atBeginSubtest(t)
		atLoadConfig(t, true, true)
		content := "前置 <@" + atBotOpenID + "> 中间 <@" + atSenderOpenID + "> 尾 <@" + atOther1OpenID + ">"
		wantStr := "前置  中间 [CQ:at,qq=" + atSenderVID + "] 尾 [CQ:at,qq=" + atOther1VID + "]"
		wantArr := fmt.Sprintf("text(%q),at(%s),text(%q),at(%s)", "前置  中间 ", atSenderVID, " 尾 ", atOther1VID)

		gotStr := RevertTransformedText(atATData(content, atStdMentions()), "group", nil, nil, 100, 200, false)
		if gotStr != wantStr {
			t.Errorf("string 模式:\n  got:  %q\n  want: %q", gotStr, wantStr)
		}
		gotArr := ConvertToSegmentedMessage(atATData(content, atStdMentions()))
		atAssertSegs(t, gotArr, wantArr)
		atAssertConsistency(t, gotStr, gotArr)
	})

	t.Run("③_仅atbot_removeAt=false_转为自身段", func(t *testing.T) {
		atBeginSubtest(t)
		atLoadConfig(t, false, true)
		content := "<@" + atBotOpenID + ">"
		wantStr := "[CQ:at,qq=8888]"
		wantArr := "at(8888)"

		gotStr := RevertTransformedText(atATData(content, atStdMentions()), "group", nil, nil, 100, 200, false)
		if gotStr != wantStr {
			t.Errorf("string 模式:\n  got:  %q\n  want: %q", gotStr, wantStr)
		}
		gotArr := ConvertToSegmentedMessage(atATData(content, atStdMentions()))
		atAssertSegs(t, gotArr, wantArr)
		atAssertConsistency(t, gotStr, gotArr)
	})

	t.Run("④_反查失败他人_原位保留", func(t *testing.T) {
		atBeginSubtest(t)
		atLoadConfig(t, true, true)
		content := "<@" + atBotOpenID + "> <@" + atOther2OpenID + "> 中 <@" + atSenderOpenID + ">"
		wantStr := rawOther2 + " 中 [CQ:at,qq=" + atSenderVID + "]"
		wantArr := fmt.Sprintf("text(%q),at(%s)", rawOther2+" 中 ", atSenderVID)

		gotStr := RevertTransformedText(atATData(content, atStdMentions()), "group", nil, nil, 100, 200, false)
		if gotStr != wantStr {
			t.Errorf("string 模式:\n  got:  %q\n  want: %q", gotStr, wantStr)
		}
		gotArr := ConvertToSegmentedMessage(atATData(content, atStdMentions()))
		atAssertSegs(t, gotArr, wantArr)
		atAssertConsistency(t, gotStr, gotArr)
	})

	t.Run("⑤_convertOtherAt=false_原样保留在原位", func(t *testing.T) {
		atBeginSubtest(t)
		atLoadConfig(t, true, false)
		content := "<@" + atBotOpenID + "> at检测 <@" + atSenderOpenID + "> <@" + atOther1OpenID + ">"
		wantStr := "at检测 <@" + atSenderOpenID + "> <@" + atOther1OpenID + ">"
		wantArr := fmt.Sprintf("text(%q)", "at检测 <@"+atSenderOpenID+"> <@"+atOther1OpenID+">")

		gotStr := RevertTransformedText(atATData(content, atStdMentions()), "group", nil, nil, 100, 200, false)
		if gotStr != wantStr {
			t.Errorf("string 模式:\n  got:  %q\n  want: %q", gotStr, wantStr)
		}
		gotArr := ConvertToSegmentedMessage(atATData(content, atStdMentions()))
		atAssertSegs(t, gotArr, wantArr)
		atAssertConsistency(t, gotStr, gotArr)
	})

	t.Run("⑥_removeAt=false_全部原位转换", func(t *testing.T) {
		atBeginSubtest(t)
		atLoadConfig(t, false, true)
		wantStr := "[CQ:at,qq=8888] at检测 [CQ:at,qq=" + atSenderVID + "] [CQ:at,qq=" + atOther1VID + "] " + rawOther2
		wantArr := fmt.Sprintf("at(8888),text(%q),at(%s),text(%q),at(%s),text(%q)", " at检测 ", atSenderVID, " ", atOther1VID, " "+rawOther2)

		gotStr := RevertTransformedText(atATData(contentStd, atStdMentions()), "group", nil, nil, 100, 200, false)
		if gotStr != wantStr {
			t.Errorf("string 模式:\n  got:  %q\n  want: %q", gotStr, wantStr)
		}
		gotArr := ConvertToSegmentedMessage(atATData(contentStd, atStdMentions()))
		atAssertSegs(t, gotArr, wantArr)
		atAssertConsistency(t, gotStr, gotArr)
	})

	t.Run("⑦_is_you误标发送者_不丢失不误转", func(t *testing.T) {
		atBeginSubtest(t)
		atLoadConfig(t, true, true)
		// 模拟 is_you 误标：发送者被标记 is_you，真正的 bot 仅带 bot 标记
		mentions := []*dto.User{
			{ID: atSenderOpenID, IsYou: true},
			{ID: atBotOpenID, Bot: true},
		}
		content := "<@" + atBotOpenID + "> at检测 <@" + atSenderOpenID + ">"
		wantStr := "at检测 [CQ:at,qq=" + atSenderVID + "]"
		wantArr := fmt.Sprintf("text(%q),at(%s)", "at检测 ", atSenderVID)

		gotStr := RevertTransformedText(atATData(content, mentions), "group", nil, nil, 100, 200, false)
		if gotStr != wantStr {
			t.Errorf("string 模式:\n  got:  %q\n  want: %q", gotStr, wantStr)
		}
		gotArr := ConvertToSegmentedMessage(atATData(content, mentions))
		atAssertSegs(t, gotArr, wantArr)
		atAssertConsistency(t, gotStr, gotArr)
	})

	t.Run("⑧_全量群消息_同语义保位", func(t *testing.T) {
		atBeginSubtest(t)
		atLoadConfig(t, true, true)
		wantStr := "at检测 [CQ:at,qq=" + atSenderVID + "] [CQ:at,qq=" + atOther1VID + "] " + rawOther2
		wantArr := fmt.Sprintf("text(%q),at(%s),text(%q),at(%s),text(%q)", "at检测 ", atSenderVID, " ", atOther1VID, " "+rawOther2)

		gotStr := RevertTransformedText(atFullData(contentStd, atStdMentions()), "group", nil, nil, 100, 200, false)
		if gotStr != wantStr {
			t.Errorf("string 模式:\n  got:  %q\n  want: %q", gotStr, wantStr)
		}
		gotArr := ConvertToSegmentedMessage(atFullData(contentStd, atStdMentions()))
		atAssertSegs(t, gotArr, wantArr)
		atAssertConsistency(t, gotStr, gotArr)
	})
}

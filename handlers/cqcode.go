package handlers

import (
	"context"
	"encoding/base64"
	"regexp"
	"runtime"
	"strconv"
	"strings"
	"sync"

	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/echo"
	"github.com/hoshinonyaruko/gensokyo/handlers/cqparse"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

// ---------- 包级正则表达式（避免重复编译） ----------

// record 码的 URL 捕获排除尾随参数：入站 messageEventCQText 会为语音附件追加
// ",asr=<转义值>"（值内 , 与 ] 已转义为 &#44; / &#93;），插件把入站原文回灌
// 出站时，URL 必须到 ,asr= 为止。只以已知参数名 asr 作切分点（而非任意逗号），
// 因此 URL 自身含逗号（如 https://x/a,b.wav）仍完整落入捕获组。
//
// 统一最大兼容契约（与 Rust 侧同口径）：
//   - 段名与参数键大小写不敏感：(?i:...) 作用域只折叠段名/键名字面，
//     `[CQ:` 前缀与值内 scheme 字面（http://、base64://、file:// 及 type 值 qq 等）
//     维持区分大小写（统一契约边界）；
//   - 段名之前（`[CQ:` 之后）、段名之后、`,` 或 `]` 之前允许任意空白；
//   - 参数按键取值、任意键序：首个逗号后允许任意前置标量段
//     （`\s*,\s*(?:[^,\]]*?,\s*)*` 再跟目标键），其余参数忽略或按现状附加处理；
//   - 键与 `=`、`=` 与值两侧允许空白；值只在两侧 trim，内部空白保留；
//   - 空白集仍为 ASCII 五元集 {空格,\t,\n,\f,\r}（全角/`\v` 不算段名空白）。
//
// stream/music/input_notify/reply 改为整码匹配 + cqUnionParams 按键取值，
// 冒号/等号双语法与任意键序在参数解析层收口。
//
// 修 F1/F2（本轮）：image/record/video/file/markdown/keyboard/card/ark 也改为
// 「整码匹配 + 按键取值」——整码识别复用 cqparse.ScanCode 同一状态机（段名整词匹配、
// 参数区花括号配平），值一律由 cqUnionParams 按 key 取出，因此尾部键
// （`[CQ:image,file=URL,name=y]`、`[CQ:keyboard,data=…,id=x]`）不再被吞进值。
// 下面这批「位置捕获」正则因此不再参与出站解析，仅因既有直接钉桩的回归测试
// （cq_record_param_test.go / zzz_tester_verify_test.go）与注释引用而保留；
// 待那些钉子户迁到管道级断言后可整批删除。
var (
	httpUrlImagePattern   = regexp.MustCompile(`\[CQ:\s*(?i:image)\s*,\s*(?:[^,\]]*?,\s*)*(?i:file)\s*=\s*http://(.+?)\s*\]`)
	httpsUrlImagePattern  = regexp.MustCompile(`\[CQ:\s*(?i:image)\s*,\s*(?:[^,\]]*?,\s*)*(?i:file)\s*=\s*https://(.+?)\s*\]`)
	base64ImagePattern    = regexp.MustCompile(`\[CQ:\s*(?i:image)\s*,\s*(?:[^,\]]*?,\s*)*(?i:file)\s*=\s*base64://(.+?)\s*\]`)
	base64RecordPattern   = regexp.MustCompile(`\[CQ:\s*(?i:record)\s*,\s*(?:[^,\]]*?,\s*)*(?i:file)\s*=\s*base64://(.+?)(?:\s*,\s*(?i:asr)\s*=[^\]]*)?\s*\]`)
	httpUrlRecordPattern  = regexp.MustCompile(`\[CQ:\s*(?i:record)\s*,\s*(?:[^,\]]*?,\s*)*(?i:file)\s*=\s*http://(.+?)(?:\s*,\s*(?i:asr)\s*=[^\]]*)?\s*\]`)
	httpsUrlRecordPattern = regexp.MustCompile(`\[CQ:\s*(?i:record)\s*,\s*(?:[^,\]]*?,\s*)*(?i:file)\s*=\s*https://(.+?)(?:\s*,\s*(?i:asr)\s*=[^\]]*)?\s*\]`)
	httpUrlVideoPattern   = regexp.MustCompile(`\[CQ:\s*(?i:video)\s*,\s*(?:[^,\]]*?,\s*)*(?i:file)\s*=\s*http://(.+?)\s*\]`)
	httpsUrlVideoPattern  = regexp.MustCompile(`\[CQ:\s*(?i:video)\s*,\s*(?:[^,\]]*?,\s*)*(?i:file)\s*=\s*https://(.+?)\s*\]`)
	base64VideoPattern    = regexp.MustCompile(`\[CQ:\s*(?i:video)\s*,\s*(?:[^,\]]*?,\s*)*(?i:file)\s*=\s*base64://(.+?)\s*\]`)
	mdPattern             = regexp.MustCompile(`\[CQ:\s*(?i:markdown)\s*,\s*(?:[^,\]]*?,\s*)*(?i:data)\s*=\s*base64://(.+?)\s*\]`)
	mdJSONPattern         = regexp.MustCompile(`\[CQ:\s*(?i:markdown)\s*,\s*(?:[^,\]]*?,\s*)*(?i:data)\s*=\s*(\{.*\})\s*\]`)
	qqMusicPattern        = regexp.MustCompile(`\[CQ:\s*(?i:music)\s*((?:,[^\]]*)?)\]`)
	inputNotifyPattern    = regexp.MustCompile(`\[CQ:\s*(?i:input_notify)\s*((?:,[^\]]*)?)\]`)
	streamPattern         = regexp.MustCompile(`\[CQ:\s*(?i:stream)\s*((?:,[^\]]*)?)\]`)
	keyboardPattern       = regexp.MustCompile(`\[CQ:\s*(?i:keyboard)\s*,\s*(?:[^,\]]*?,\s*)*(?i:data)\s*=\s*base64://(.+?)\s*\]`)
	keyboardJSONPattern   = regexp.MustCompile(`\[CQ:\s*(?i:keyboard)\s*,\s*(?:[^,\]]*?,\s*)*(?i:data)\s*=\s*(\{.*\})\s*\]`)
	replyRe               = regexp.MustCompile(`\[CQ:\s*(?i:reply)\s*((?:,[^\]]*)?)\]`)
	wakeupPattern         = regexp.MustCompile(`\[CQ:\s*(?i:wakeup)\s*,\s*(?:[^,\]]*?,\s*)*(?i:userid)\s*=\s*([^\]]+?)\s*\]`)
	// stream 值的既有字符面收口：type 为 `\w+`、qq 为纯数字（仅形态放宽，值校验不变）。
	cqStreamTypeRe      = regexp.MustCompile(`^\w+$`)
	localImagePattern   *regexp.Regexp
	localRecordPattern  *regexp.Regexp
	localVideoPattern   *regexp.Regexp
	compilePatternsOnce sync.Once
)

// cqUnionParams 按 key 解析 CQ 码参数区（逗号之后、`]` 之前的原始文本），
// 对齐统一契约 §1~§4 的按键取值口径：
//   - 键 TrimSpace 后统一转小写（段名与键名大小写不敏感，重复键后者覆盖）；
//   - 值仅 TrimSpace 首尾、内部空白保留；不做实体解码（legacy 转义口径不变，
//     与 cqparse buildParams 的差异仅在解码步）；
//   - 无 `=` 的尾随段并入前一 key 值、以 `,` 连接（C3 口径，值内原始逗号不截断）；
//   - allowColon=true 时段内无 `=` 再兼容首个 `:` 分隔（stream 冒号语法），
//     两种语法均可混用。
func cqUnionParams(params string, allowColon bool) map[string]string {
	return cqUnionParamsSegments(strings.Split(params, ","), allowColon)
}

// cqUnionParamsSegments 与 cqUnionParams 完全同口径，入参改为已由 cqparse.ScanCode
// 按花括号配平切好的参数段（整码匹配路径直接用段，避免对 JSON 值内逗号二次切分）。
func cqUnionParamsSegments(segments []string, allowColon bool) map[string]string {
	out := make(map[string]string)
	lastKey := ""
	for _, seg := range segments {
		k, v, found := strings.Cut(seg, "=")
		if !found && allowColon {
			k, v, found = strings.Cut(seg, ":")
		}
		if !found {
			t := strings.TrimSpace(seg)
			if lastKey != "" && t != "" {
				out[lastKey] = out[lastKey] + "," + t
			}
			continue
		}
		key := strings.ToLower(strings.TrimSpace(k))
		if key == "" {
			continue
		}
		out[key] = strings.TrimSpace(v)
		lastKey = key
	}
	return out
}

// cqCodeOpen CQ 码字面前缀（`[CQ:` 区分大小写，统一契约边界）。
const cqCodeOpen = "[CQ:"

// cqApplyNamedCodes 单次扫描 text，对每个段名为 name 的完整 CQ 码调用 fn，
// fn 收到整码原文与按键取值后的参数 map：
//   - 返回 "" → 整码已消费，从正文移除；
//   - 返回整码原文 → 值校验不通过/无法识别，整段字面透传（与今日正则不命中时一致）。
//
// 整码识别复用 cqparse.ScanCode（段名整词匹配 + 参数区花括号配平，修 F1 尾键污染、
// F2 前缀吞段），参数解析复用 cqUnionParams 口径（键小写归一、任意键序、值仅 trim
// 首尾、无 `=` 尾段并入前键、不做实体解码）。
func cqApplyNamedCodes(text, name string, allowColon bool, fn func(raw string, params map[string]string) string) string {
	target := strings.ToLower(name)
	var b strings.Builder
	last := 0
	for i := 0; i < len(text); {
		j := strings.Index(text[i:], cqCodeOpen)
		if j < 0 {
			break
		}
		start := i + j
		action, segments, end, ok := cqparse.ScanCode(text, start)
		if !ok || action != target {
			// 非目标段名或畸形码：跳过一个字符继续扫描，原文保持不动
			i = start + 1
			continue
		}
		if start > last {
			b.WriteString(text[last:start])
		}
		b.WriteString(fn(text[start:end], cqUnionParamsSegments(segments, allowColon)))
		last = end
		i = end
	}
	b.WriteString(text[last:])
	return b.String()
}

// cqMediaItem 按 file 值的 scheme 决定 foundItems 键名与剥 scheme 后的值，
// 键名与值口径与既有位置捕获正则逐项对齐：
// http:// → url_<kind>、https:// → url_<kind>s、base64:// → base64_<kind>、
// file:// → local_<kind>；本地路径的平台差异同现状（Windows 剥 file:/// 三个斜杠，
// Unix 剥 file:// 而把前导 / 留在值内）。无 scheme / 未知 scheme 返回 ok=false，
// 整码字面透传（与今日无 scheme 不消费一致）。
func cqMediaItem(file, kind string) (key, value string, ok bool) {
	switch {
	case file == "":
		return "", "", false
	case strings.HasPrefix(file, "http://"):
		return "url_" + kind, strings.TrimPrefix(file, "http://"), true
	case strings.HasPrefix(file, "https://"):
		return "url_" + kind + "s", strings.TrimPrefix(file, "https://"), true
	case strings.HasPrefix(file, "base64://"):
		return "base64_" + kind, strings.TrimPrefix(file, "base64://"), true
	case runtime.GOOS == "windows" && strings.HasPrefix(file, "file:///"):
		return "local_" + kind, strings.TrimPrefix(file, "file:///"), true
	case runtime.GOOS != "windows" && strings.HasPrefix(file, "file://"):
		return "local_" + kind, strings.TrimPrefix(file, "file://"), true
	default:
		return "", "", false
	}
}

// cqPayloadValue 取 markdown/keyboard 的 data 载荷值（统一两种既有形态）：
//   - base64:// 前缀 → 去前缀原样返回（不解码，现状口径）；
//   - 以 { 起、} 止 → 原始 JSON 原文返回，由调用方决定是否再编码（既有
//     mdJSONPattern / keyboardJSONPattern 的贪婪捕获同样要求值尾是 }）；
//   - 其余形态 ok=false，整码字面透传。
func cqPayloadValue(data string) (string, bool) {
	switch {
	case data == "":
		return "", false
	case strings.HasPrefix(data, "base64://"):
		return strings.TrimPrefix(data, "base64://"), true
	case strings.HasPrefix(data, "{") && strings.HasSuffix(data, "}"):
		return data, true
	default:
		return "", false
	}
}

// initPlatformPatterns 初始化平台相关的正则表达式（Windows vs Unix 路径前缀差异）
// 空白口径与包级正则一致：段名前（`[CQ:` 后）与段名后、键值两侧容忍空白，值仅 trim 首尾；
// 段名与键名大小写不敏感，file 允许出现在任意参数位置（统一契约 §1~§4）。
func initPlatformPatterns() {
	if runtime.GOOS == "windows" {
		localImagePattern = regexp.MustCompile(`\[CQ:\s*(?i:image)\s*,\s*(?:[^,\]]*?,\s*)*(?i:file)\s*=\s*file:///([^\]]+?)\s*\]`)
		localRecordPattern = regexp.MustCompile(`\[CQ:\s*(?i:record)\s*,\s*(?:[^,\]]*?,\s*)*(?i:file)\s*=\s*file:///([^\]]+?)(?:\s*,\s*(?i:asr)\s*=[^\]]*)?\s*\]`)
		localVideoPattern = regexp.MustCompile(`\[CQ:\s*(?i:video)\s*,\s*(?:[^,\]]*?,\s*)*(?i:file)\s*=\s*file:///([^\]]+?)\s*\]`)
	} else {
		localImagePattern = regexp.MustCompile(`\[CQ:\s*(?i:image)\s*,\s*(?:[^,\]]*?,\s*)*(?i:file)\s*=\s*file://([^\]]+?)\s*\]`)
		localRecordPattern = regexp.MustCompile(`\[CQ:\s*(?i:record)\s*,\s*(?:[^,\]]*?,\s*)*(?i:file)\s*=\s*file://([^\]]+?)(?:\s*,\s*(?i:asr)\s*=[^\]]*)?\s*\]`)
		localVideoPattern = regexp.MustCompile(`\[CQ:\s*(?i:video)\s*,\s*(?:[^,\]]*?,\s*)*(?i:file)\s*=\s*file://([^\]]+?)\s*\]`)
	}
}

// ProcessCQFile 解析 [CQ:file,file=xxx,file_name=yyy] 并移除
// file_name 为可选参数，不填时由后续处理自动从路径/URL 提取文件名。
// 空白容忍（统一契约 §1~§4）：段名前（`[CQ:` 后）与段名后、键值两侧可带空白，
// 键 trim 后大小写不敏感、任意键序，值仅 trim 首尾。
// 修 F1：整码匹配后按 key 取 file / file_name，因此
// `[CQ:file,file=https://x/a.zip,note=y]` 的 file 值不再被尾键污染，
// 值自带逗号（`file=https://x/a,b.zip`）也经 C3 并段口径完整保留。
func ProcessCQFile(text string, foundItems map[string][]string) string {
	return cqApplyNamedCodes(text, "file", false, func(match string, params map[string]string) string {
		filePath := params["file"]
		if filePath == "" {
			return match
		}
		// 根据 file 前缀确定类型
		var itemKey, cleanValue string
		switch {
		case strings.HasPrefix(filePath, "file://"):
			itemKey = "local_file"
			safePath, err := resolveLocalMedia(filePath)
			if err != nil {
				mylog.Printf("安全校验失败，跳过本地文件: %v", err)
				return match
			}
			cleanValue = safePath
		case strings.HasPrefix(filePath, "http://"):
			itemKey = "url_file"
			cleanValue = strings.TrimPrefix(filePath, "http://")
		case strings.HasPrefix(filePath, "https://"):
			itemKey = "url_files"
			cleanValue = strings.TrimPrefix(filePath, "https://")
		case strings.HasPrefix(filePath, "base64://"):
			itemKey = "base64_file"
			cleanValue = strings.TrimPrefix(filePath, "base64://")
		default:
			return match
		}
		foundItems[itemKey] = append(foundItems[itemKey], cleanValue)
		if fileName := params["file_name"]; fileName != "" {
			foundItems["file_name"] = append(foundItems["file_name"], fileName)
		}
		return ""
	})
}

// ProcessCQKeyboard 解析 [CQ:keyboard,data=base64://...] 或 [CQ:keyboard,data={json}] 并移除
// 与 [CQ:markdown] 保持一致：data 支持 base64 编码或原始 JSON 两种形式，
// 解码后的键盘 JSON（结构同 parseMDData 的 keyboard 字段）存入 foundItems["keyboard"]
// ProcessCQKeyboard 解析 [CQ:keyboard,data=base64://...] 或 [CQ:keyboard,data={json}] 并移除
// 与 [CQ:markdown] 保持一致：data 支持 base64 编码或原始 JSON 两种形式，
// 解码后的键盘 JSON（结构同 parseMDData 的 keyboard 字段）存入 foundItems["keyboard"]。
// 修 F1：整码匹配 + data 按键取值，`[CQ:keyboard,data=base64://…,id=x]` 的尾键
// 不再混进 base64（此前解码失败 → 整段丢弃、按钮静默消失）；键序与大小写、
// 段名前后空白一并容忍（统一契约 §1~§4）。base64 解码失败仍按现状整码移除不落账。
func ProcessCQKeyboard(text string, foundItems map[string][]string) string {
	return cqApplyNamedCodes(text, "keyboard", false, func(match string, params map[string]string) string {
		data, ok := cqPayloadValue(params["data"])
		if !ok {
			return match // data 缺失或既非 base64:// 也非 {...}：与今日正则不命中一致，字面透传
		}
		if strings.HasPrefix(params["data"], "base64://") {
			decoded, err := base64.StdEncoding.DecodeString(data)
			if err != nil {
				mylog.Printf("[CQ:keyboard] base64 解码失败: %v", err)
				return ""
			}
			foundItems["keyboard"] = append(foundItems["keyboard"], string(decoded))
			return ""
		}
		foundItems["keyboard"] = append(foundItems["keyboard"], data)
		return ""
	})
}

// ProcessCQActive 解析 [CQ:active] 或 [CQ:active,type=xxx,sub_type=yyy] 并移除。
// 空白容忍（统一契约 §1~§4）：段名前（`[CQ:` 后）与段名后、键值两侧可带空白，
// 段名与键名大小写不敏感，任意键序。
func ProcessCQActive(text string, foundItems map[string][]string) string {
	// 先匹配带参数的 [CQ:active,type=xxx,sub_type=yyy]
	re := regexp.MustCompile(`\[CQ:\s*(?i:active)\s*,([^\]]*)\]`)
	text = re.ReplaceAllStringFunc(text, func(match string) string {
		// 捕获组即参数区（首个逗号之后、`]` 之前），逐段按键取值
		inner := match[strings.IndexByte(match, ',')+1 : len(match)-1]
		for _, part := range strings.Split(inner, ",") {
			kv := strings.SplitN(part, "=", 2)
			if len(kv) == 2 {
				switch strings.ToLower(strings.TrimSpace(kv[0])) {
				case "type":
					foundItems["active_type"] = append(foundItems["active_type"], strings.TrimSpace(kv[1]))
				case "sub_type":
					foundItems["active_sub_type"] = append(foundItems["active_sub_type"], strings.TrimSpace(kv[1]))
				}
			}
		}
		foundItems["active"] = []string{"true"}
		return ""
	})
	// 再匹配裸 [CQ:active]（无参数，段名前后可带空白）
	bareRe := regexp.MustCompile(`\[CQ:\s*(?i:active)\s*\]`)
	if bareRe.MatchString(text) {
		foundItems["active"] = []string{"true"}
	}
	text = bareRe.ReplaceAllString(text, "")
	return text
}

// ProcessCQWakeup 解析 [CQ:wakeup,userid=xxxxx] 并移除
// 目标用户（OpenID 或虚拟数字 ID）写入 foundItems["wakeup"]，
// 由 send_private_msg 消费并走 C2C 召回（is_wakeup）发送通道
func ProcessCQWakeup(text string, foundItems map[string][]string) string {
	return wakeupPattern.ReplaceAllStringFunc(text, func(match string) string {
		if submatch := wakeupPattern.FindStringSubmatch(match); len(submatch) > 1 {
			userID := strings.TrimSpace(submatch[1])
			if userID != "" {
				foundItems["wakeup"] = append(foundItems["wakeup"], userID)
			}
		}
		return ""
	})
}

// ---------- 入站专用 CQ 码的出站剥离 ----------

// 入站上报会产出 [CQ:ark,...]（卡片）与 [CQ:reply,msg_idx=...]（引用索引）两类
// 出站链路无消费点的码，插件把入站原文回传出站时它们会以字面量文本发给用户。
// ark 参数值内可能含 &#44; 等转义，故整段匹配到右括号，不按逗号切分；
// reply 仅剥离 msg_idx= 形态，[CQ:reply,id=数字] 是既有出站引用语义（由 replyRe 消费）。
// 统一契约 §1~§4：段名与键名大小写不敏感、段名前（`[CQ:` 后）容忍空白、任意键序。
// ProcessCQInboundOnlyCodes 剥离出站无消费点的入站专用 CQ 码（ark / reply,msg_idx=）。
// ark / reply 的 msg_idx 形态同样改为整码匹配 + 按键取值（修 F2）：段名整词命中后
// 必须紧跟 `,` 或 `]`，`[CQ:CARDIGAN,x=1]`、`[CQ:ARKWright,...]` 这类字面文本
// 不再被 card/ark 前缀吞掉；花括号配平口径保证 ark 值内的 `]` 与 `,` 不截断整码。
// 键名大小写不敏感、段名前后可带空白（统一契约 §1~§4）。
func ProcessCQInboundOnlyCodes(text string) string {
	text = cqApplyNamedCodes(text, "ark", false, func(_ string, _ map[string]string) string {
		return "" // 整段剥离
	})
	return cqApplyNamedCodes(text, "reply", false, func(match string, params map[string]string) string {
		if _, ok := params["msg_idx"]; ok {
			return "" // 入站索引形态：出站无消费点，整段剥离
		}
		return match // [CQ:reply,id=数字] 是既有出站引用语义，由 replyRe 消费
	})
}

// ---------- 出站动作型 CQ 码（单次扫描 + 类型分发） ----------

// ProcessOutboundCQCodes 统一处理出站动作型 CQ 码。
// 单次正则扫描全文，按类型分发执行动作并从文本移除；
// 未知类型（at/image 等标准 CQ 码）原样保留。
// 返回 (清理后的文本, member 跨群路由的 realGroupID)。
func ProcessOutboundCQCodes(text, defaultGroupID string, eventID *string, apiv2 openapi.OpenAPI) (string, string) {
	out, realGroupID, _ := ProcessOutboundCQCodesWithEventFlag(text, defaultGroupID, eventID, apiv2)
	return out, realGroupID
}

// ProcessOutboundCQCodesWithEventFlag 与 ProcessOutboundCQCodes 同语义，额外返回
// 「CQ 是否显式决定过 eventID」（member add 命中写入 / member remove 显式清空为 true）。
// 出站 new 链据此跳过 messageID=="2000" 的缓存 event_id 回填，与 legacy 时序一致。
func ProcessOutboundCQCodesWithEventFlag(text, defaultGroupID string, eventID *string, apiv2 openapi.OpenAPI) (string, string, bool) {
	// 统一契约 §1~§4：段名大小写不敏感、段名前（`[CQ:` 后）与逗号之间容忍空白；
	// 段名字符集放宽为 [a-zA-Z0-9_]+（含数字，与 cqparse.ScanCode / internal 链 / Rust
	// 并集口径一致——数字名段落入 default 原样保留，行为不变、口径统一）；
	// 键值空白由各动作的参数解析函数 trim 并统一小写按键取值（§2/§3）
	re := regexp.MustCompile(`\[CQ:\s*(?i:([a-z0-9_]+))\s*,([^\]]*)\]`)
	var realGroupID string
	var eventIDSet bool
	result := re.ReplaceAllStringFunc(text, func(match string) string {
		inner := match[len("[CQ:") : len(match)-1]
		idx := strings.Index(inner, ",")
		if idx < 0 {
			return match // 无参数形式（如 [CQ:at]），防御性保留
		}
		// 段名前（`[CQ:` 后）与段名后均可能有空白：两侧都要 trim 再小写归一
		cqType := strings.ToLower(strings.Trim(inner[:idx], " \t\r\n\f"))
		paramsStr := inner[idx+1:]
		switch cqType {
		case "member":
			return cqMemberActionWithEventFlag(paramsStr, match, eventID, defaultGroupID, apiv2, &realGroupID, &eventIDSet)
		case "remove":
			return cqRemoveAction(paramsStr, match, defaultGroupID, apiv2)
		case "set_group":
			return cqSetGroupAction(paramsStr, match, defaultGroupID, apiv2)
		default:
			return match // 非动作 CQ 码原样保留
		}
	})
	return result, realGroupID, eventIDSet
}

// cqResolveGroupID 将默认群 ID 反查为真实 OpenID（32 位原生 OpenID 直接使用）
func cqResolveGroupID(groupID string) string {
	if len(groupID) != 32 {
		if realGroupID, err := idmap.RetrieveRowByIDv2(groupID); err == nil && realGroupID != "" {
			return realGroupID
		}
	}
	return groupID
}

// cqMemberAction 处理 [CQ:member,type=add/remove,group_id=虚拟群ID,user_id=虚拟用户ID]
// type=add: 使用存储的 event_id 进行被动回复；type=remove: 转为主动消息发送
func cqMemberAction(paramsStr, match string, eventID *string, defaultGroupID string, apiv2 openapi.OpenAPI, realGroupID *string) string {
	cqMemberActionWithEventFlag(paramsStr, match, eventID, defaultGroupID, apiv2, realGroupID, nil)
	return ""
}

// cqMemberActionWithEventFlag 是 cqMemberAction 的带标志变体：
// eventIDSet（可传 nil 表示不关心）标记「显式决定过 eventID」——
// type=add 命中缓存写入、type=remove 显式清空均为 true；add 未命中为 false（只覆盖语义）。
func cqMemberActionWithEventFlag(paramsStr, match string, eventID *string, defaultGroupID string, apiv2 openapi.OpenAPI, realGroupID *string, eventIDSet *bool) string {
	setEventIDSet := func(v bool) {
		if eventIDSet != nil {
			*eventIDSet = v
		}
	}
	var cqGroupID, cqUserID, memberType string
	for _, part := range strings.Split(paramsStr, ",") {
		kv := strings.SplitN(part, "=", 2)
		if len(kv) != 2 {
			continue
		}
		switch strings.ToLower(strings.TrimSpace(kv[0])) {
		case "type":
			memberType = strings.TrimSpace(kv[1])
		case "group_id":
			cqGroupID = strings.TrimSpace(kv[1])
		case "user_id":
			cqUserID = strings.TrimSpace(kv[1])
		}
	}

	// 将虚拟 user_id 反向转换为 OpenID（用于日志）
	openID, err := idmap.RetrieveRowByIDv2(cqUserID)
	if err != nil || openID == "" {
		mylog.Printf("[CQ:member] user_id=%s 转换为 OpenID 失败: %v", cqUserID, err)
	} else {
		mylog.Printf("[CQ:member] user_id=%s → OpenID=%s", cqUserID, openID)
	}

	// 从 CQ 码中取 group_id，优先于入参
	if cqGroupID == "" {
		cqGroupID = defaultGroupID
	}

	// 将 CQ 码中的虚拟 group_id 转为真实 OpenID（作为目标群）
	realGroupOpenID, err := idmap.RetrieveRowByIDv2(cqGroupID)
	if err != nil || realGroupOpenID == "" {
		mylog.Printf("[CQ:member] groupID=%s 转换为 OpenID 失败: %v", cqGroupID, err)
		realGroupOpenID = cqGroupID
	} else {
		mylog.Printf("[CQ:member] groupID=%s → OpenID=%s", cqGroupID, realGroupOpenID)
	}
	*realGroupID = realGroupOpenID

	switch memberType {
	case "add":
		appID := config.GetAppIDStr()
		key := appID + "_" + realGroupOpenID
		storedEventID := echo.GetEventIDByKey(key)
		if storedEventID != "" {
			*eventID = storedEventID
			setEventIDSet(true)
			mylog.Printf("[CQ:member] 入群回复: 使用 event_id=%s (group->%s, user->%s)", storedEventID, realGroupOpenID, openID)
		} else {
			mylog.Printf("[CQ:member] 入群回复: 未找到 event_id (group=%s)", cqGroupID)
		}
	case "remove":
		*eventID = ""
		setEventIDSet(true)
		mylog.Printf("[CQ:member] 退群消息: 转为主动推送 (group_id=%s, user->%s)", cqGroupID, openID)
	}

	return ""
}

// cqRemoveAction 处理 [CQ:remove,user_id=虚拟ID,msg_id=虚拟msg_id]
// 通过 QQ API 撤回指定消息，并从 messageText 中移除 CQ 码
func cqRemoveAction(paramsStr, match, defaultGroupID string, apiv2 openapi.OpenAPI) string {
	groupID := cqResolveGroupID(defaultGroupID)
	var userID, msgID string
	for _, part := range strings.Split(paramsStr, ",") {
		kv := strings.SplitN(part, "=", 2)
		if len(kv) == 2 {
			switch strings.ToLower(strings.TrimSpace(kv[0])) {
			case "user_id":
				userID = strings.TrimSpace(kv[1])
			case "msg_id":
				msgID = strings.TrimSpace(kv[1])
			}
		}
	}
	if userID == "" || msgID == "" {
		if userID != "" && msgID == "" {
			// 缺 msg_id 时自动查该用户最新消息
			realUserID, err := idmap.RetrieveRowByIDv2(userID)
			if err != nil {
				mylog.Printf("[CQ:remove] 解析 user_id=%s 失败: %v", userID, err)
				return match
			}
			latestRealMsgID, err := idmap.GetLatestMsgID(groupID, realUserID)
			if err != nil {
				mylog.Printf("[CQ:remove] 获取用户 %s 最新消息失败: %v", userID, err)
				return match
			}
			mylog.Printf("[CQ:remove] 自动获取用户 %s 最新消息: %s", userID, latestRealMsgID)
			if err := apiv2.RetractGroupMessage(context.TODO(), groupID, latestRealMsgID); err != nil {
				mylog.Printf("[CQ:remove] 撤回消息失败 group=%s msg=%s: %v", groupID, latestRealMsgID, err)
			} else {
				mylog.Printf("[CQ:remove] 已撤回消息 group=%s msg=%s", groupID, latestRealMsgID)
			}
			return ""
		}
		mylog.Printf("[CQ:remove] user_id 或 msg_id 为空: %s", match)
		return match
	}

	// 解析虚拟 user_id 为真实 OpenID（仅用于校验）
	_, err := idmap.RetrieveRowByIDv2(userID)
	if err != nil {
		mylog.Printf("[CQ:remove] 解析 user_id=%s 失败: %v", userID, err)
		return ""
	}

	// 解析虚拟 msg_id 为真实 message_id
	realMsgID, err := idmap.RetrieveRowByCachev2(msgID)
	if err != nil {
		mylog.Printf("[CQ:remove] 解析 msg_id=%s 失败: %v", msgID, err)
		return ""
	}
	// RetrieveRowByCachev2 返回格式 "groupID msgID"，取后半段
	parts := strings.Split(realMsgID, " ")
	realMsgID = parts[len(parts)-1]

	// 调用撤回 API
	if err := apiv2.RetractGroupMessage(context.TODO(), groupID, realMsgID); err != nil {
		mylog.Printf("[CQ:remove] 撤回消息失败 group=%s msg=%s: %v", groupID, realMsgID, err)
	} else {
		mylog.Printf("[CQ:remove] 已撤回消息 group=%s msg=%s", groupID, realMsgID)
	}

	return "" // 从 messageText 中移除 CQ 码，无论成败都不发送原文
}

// cqParseParams 顺序无关地解析 CQ 码参数（key=value, 逗号分隔）。
// 键 TrimSpace 后统一转小写（统一契约 §1：参数键大小写不敏感，重复键后者覆盖）；
// 值中的 CQ 转义（&#44;=逗号, &#93;=右括号, &amp;=&）会被还原，
// 与 buildSetGroupCQCode 的转义互为逆操作。
func cqParseParams(paramsStr string) map[string]string {
	params := make(map[string]string)
	for _, part := range strings.Split(paramsStr, ",") {
		kv := strings.SplitN(part, "=", 2)
		if len(kv) == 2 {
			value := strings.TrimSpace(kv[1])
			// 反转义：先还原实体，再还原 & 本身（顺序不可颠倒）
			value = strings.ReplaceAll(value, "&#44;", ",")
			value = strings.ReplaceAll(value, "&#93;", "]")
			value = strings.ReplaceAll(value, "&amp;", "&")
			params[strings.ToLower(strings.TrimSpace(kv[0]))] = value
		}
	}
	return params
}

// cqSetGroupAction 处理 [CQ:set_group,action=ban/whole_ban/add_request/strategy_execute/strategy_delete/kick/blacklist_add/blacklist_del,...]
// 统一分发 set_group 系列动作；未知 action 原样保留
func cqSetGroupAction(paramsStr, match, defaultGroupID string, apiv2 openapi.OpenAPI) string {
	params := cqParseParams(paramsStr)
	action := params["action"]
	switch action {
	case "ban":
		return cqSetGroupBanAction(params, match, defaultGroupID, apiv2)
	case "whole_ban":
		return cqSetGroupWholeBanAction(params, match, defaultGroupID, apiv2)
	case "add_request":
		return cqSetGroupAddRequestAction(params, match, defaultGroupID, apiv2)
	case "strategy_execute":
		return cqSetGroupStrategyAction(params, match, apiv2, true)
	case "strategy_delete":
		return cqSetGroupStrategyAction(params, match, apiv2, false)
	case "kick":
		return cqSetGroupKickAction(params, match, defaultGroupID, apiv2)
	case "blacklist_add", "blacklist_del":
		return cqSetGroupBlacklistAction(params, match, defaultGroupID, apiv2, action)
	default:
		mylog.Printf("[CQ:set_group] 未知 action=%s: %s", action, match)
		return match // 未知 action 原样保留
	}
}

// cqSetGroupBanAction 处理 [CQ:set_group,action=ban,group_id=虚拟群ID,user_id=虚拟用户ID,duration=秒]
// 设置成员禁言（duration=0 解除）
func cqSetGroupBanAction(params map[string]string, match, defaultGroupID string, apiv2 openapi.OpenAPI) string {
	userID := params["user_id"]
	groupID := params["group_id"]
	if groupID == "" {
		groupID = defaultGroupID
	}
	if groupID == "" || userID == "" {
		mylog.Printf("[CQ:set_group] ban: group_id 或 user_id 为空: %s", match)
		return match
	}
	duration, err := strconv.Atoi(params["duration"])
	if err != nil {
		duration = 0
	}
	// 反查真实 OpenID（32 位原生 OpenID 直接使用）
	groupOpenID, err := resolveGroupOpenID(groupID)
	if err != nil {
		mylog.Printf("[CQ:set_group] ban: 群 OpenID 反查失败: %v", err)
		return ""
	}
	memberOpenID, err := resolveMemberOpenID(userID)
	if err != nil {
		mylog.Printf("[CQ:set_group] ban: user_id=%s 反查失败: %v", userID, err)
		return ""
	}
	if err := applyRestrictChatSetting(apiv2, groupOpenID, memberOpenID, duration, nil); err != nil {
		mylog.Printf("[CQ:set_group] ban: 设置禁言失败: %v", err)
	} else {
		mylog.Printf("[CQ:set_group] ban: 已设置禁言 group=%s user=%s duration=%d", groupOpenID, memberOpenID, duration)
	}
	return "" // 无论成败都不发送原文
}

// cqSetGroupWholeBanAction 处理 [CQ:set_group,action=whole_ban,group_id=虚拟群ID,enable=true/false]
// 切换全员禁言：官方 POST /restrict_chat_setting 无全员禁言写入字段，共享底层固定返回失败（仅日志）
func cqSetGroupWholeBanAction(params map[string]string, match, defaultGroupID string, apiv2 openapi.OpenAPI) string {
	groupID := params["group_id"]
	if groupID == "" {
		groupID = defaultGroupID
	}
	if groupID == "" {
		mylog.Printf("[CQ:set_group] whole_ban: group_id 为空: %s", match)
		return match
	}
	enable, err := strconv.ParseBool(params["enable"])
	if err != nil {
		mylog.Printf("[CQ:set_group] whole_ban: enable 参数无效: %s", match)
		return match
	}
	groupOpenID, err := resolveGroupOpenID(groupID)
	if err != nil {
		mylog.Printf("[CQ:set_group] whole_ban: 群 OpenID 反查失败: %v", err)
		return ""
	}
	allMute := enable
	if err := applyRestrictChatSetting(apiv2, groupOpenID, "", 0, &allMute); err != nil {
		mylog.Printf("[CQ:set_group] whole_ban: 设置全员禁言失败: %v", err)
	}
	return ""
}

// cqSetGroupAddRequestAction 处理 [CQ:set_group,action=add_request,group_id=虚拟群ID,user_id=虚拟用户ID,flag=申请ID,approve=true/false]
// 审批入群申请（可带 reason / add_to_member_blacklist）
func cqSetGroupAddRequestAction(params map[string]string, match, defaultGroupID string, apiv2 openapi.OpenAPI) string {
	userID := params["user_id"]
	flag := params["flag"]
	groupID := params["group_id"]
	if groupID == "" {
		groupID = defaultGroupID
	}
	if groupID == "" || userID == "" || flag == "" {
		mylog.Printf("[CQ:set_group] add_request: group_id/user_id/flag 不能为空: %s", match)
		return match
	}
	approve := true // 省略时默认通过
	if params["approve"] != "" {
		parsed, err := strconv.ParseBool(params["approve"])
		if err != nil {
			mylog.Printf("[CQ:set_group] add_request: approve 参数无效: %s", match)
			return match
		}
		approve = parsed
	}
	groupOpenID, err := resolveGroupOpenID(groupID)
	if err != nil {
		mylog.Printf("[CQ:set_group] add_request: 群 OpenID 反查失败: %v", err)
		return ""
	}
	memberOpenID, err := resolveMemberOpenID(userID)
	if err != nil {
		mylog.Printf("[CQ:set_group] add_request: user_id=%s 反查失败: %v", userID, err)
		return ""
	}
	var blacklist *bool
	if b, err := strconv.ParseBool(params["add_to_member_blacklist"]); err == nil {
		blacklist = &b
	}
	if err := approveJoinRequest(apiv2, groupOpenID, memberOpenID, flag, approve, params["reason"], blacklist); err != nil {
		mylog.Printf("[CQ:set_group] add_request: 审批失败: %v", err)
	} else {
		op := "decline"
		if approve {
			op = "approve"
		}
		mylog.Printf("[CQ:set_group] add_request: 已审批 group=%s user=%s op=%s", groupOpenID, memberOpenID, op)
	}
	return ""
}

// cqSetGroupStrategyAction 处理 [CQ:set_group,action=strategy_execute/strategy_delete,strategy_id=策略ID]
// execute=true 执行入群自动审批策略（异步约10分钟）；execute=false 删除策略
func cqSetGroupStrategyAction(params map[string]string, match string, apiv2 openapi.OpenAPI, execute bool) string {
	strategyID := params["strategy_id"]
	if strategyID == "" {
		mylog.Printf("[CQ:set_group] strategy: strategy_id 为空: %s", match)
		return match
	}
	if execute {
		if err := apiv2.ExecuteJoinApprovalStrategy(context.TODO(), strategyID); err != nil {
			mylog.Printf("[CQ:set_group] strategy: 执行策略失败: %v", err)
		} else {
			mylog.Printf("[CQ:set_group] strategy: 已执行策略 %s（异步约10分钟）", strategyID)
		}
	} else {
		if err := apiv2.DeleteJoinApprovalStrategy(context.TODO(), strategyID); err != nil {
			mylog.Printf("[CQ:set_group] strategy: 删除策略失败: %v", err)
		} else {
			mylog.Printf("[CQ:set_group] strategy: 已删除策略 %s", strategyID)
		}
	}
	return ""
}

// cqSetGroupKickAction 处理 [CQ:set_group,action=kick,group_id=虚拟群ID,user_id=虚拟用户ID,user_ids=逗号分隔批量,add_blacklist=true/false]
// 单个/批量移出群成员（≤20，单个也走官方批量接口）；add_blacklist=true 移出同时拉黑
func cqSetGroupKickAction(params map[string]string, match, defaultGroupID string, apiv2 openapi.OpenAPI) string {
	groupID := params["group_id"]
	if groupID == "" {
		groupID = defaultGroupID
	}
	ids := cqSetGroupUserIDs(params, match)
	if groupID == "" || len(ids) == 0 {
		mylog.Printf("[CQ:set_group] kick: group_id 或 user_id/user_ids 为空: %s", match)
		return match
	}
	groupOpenID, err := resolveGroupOpenID(groupID)
	if err != nil {
		mylog.Printf("[CQ:set_group] kick: 群 OpenID 反查失败: %v", err)
		return ""
	}
	openIDs := cqSetGroupResolveMembers(ids, "kick")
	if len(openIDs) == 0 {
		return "" // 全部反查失败,无成员可操作
	}
	addBlacklist := false
	if params["add_blacklist"] != "" {
		b, err := strconv.ParseBool(params["add_blacklist"])
		if err != nil {
			mylog.Printf("[CQ:set_group] kick: add_blacklist 参数无效,按 false 处理: %s", params["add_blacklist"])
		} else {
			addBlacklist = b
		}
	}
	req := &dto.BatchRemoveMembersRequest{
		MemberOpenIDs:        openIDs,
		AddToMemberBlacklist: addBlacklist,
	}
	resp, err := apiv2.BatchRemoveMembers(context.TODO(), groupOpenID, req)
	if err != nil {
		mylog.Printf("[CQ:set_group] kick: 批量移出失败: %v", err)
	} else {
		mylog.Printf("[CQ:set_group] kick: 已提交移出 group=%s 共 %d 人(add_blacklist=%v, result=%s)", groupOpenID, len(openIDs), addBlacklist, resp.RemoveMembersResult)
	}
	return "" // 无论成败都不发送原文
}

// cqSetGroupBlacklistAction 处理 [CQ:set_group,action=blacklist_add/blacklist_del,group_id=虚拟群ID,user_id=虚拟用户ID|user_ids=逗号分隔批量]
// 群黑名单增删（≤20）；群内成员 add 会被官方拒绝，错误透传日志
func cqSetGroupBlacklistAction(params map[string]string, match, defaultGroupID string, apiv2 openapi.OpenAPI, action string) string {
	op := "add"
	if action == "blacklist_del" {
		op = "del"
	}
	groupID := params["group_id"]
	if groupID == "" {
		groupID = defaultGroupID
	}
	ids := cqSetGroupUserIDs(params, match)
	if groupID == "" || len(ids) == 0 {
		mylog.Printf("[CQ:set_group] %s: group_id 或 user_id/user_ids 为空: %s", action, match)
		return match
	}
	groupOpenID, err := resolveGroupOpenID(groupID)
	if err != nil {
		mylog.Printf("[CQ:set_group] %s: 群 OpenID 反查失败: %v", action, err)
		return ""
	}
	openIDs := cqSetGroupResolveMembers(ids, action)
	if len(openIDs) == 0 {
		return "" // 全部反查失败,无成员可操作
	}
	req := &dto.MemberBlacklistRequest{
		Op:            op,
		MemberOpenIDs: openIDs,
	}
	resp, err := apiv2.UpdateMemberBlacklist(context.TODO(), groupOpenID, req)
	if err != nil {
		mylog.Printf("[CQ:set_group] %s: 黑名单操作失败: %v", action, err)
	} else if len(resp.FailOpenids) > 0 {
		mylog.Printf("[CQ:set_group] %s: 部分失败 %d 人: %v", action, len(resp.FailOpenids), resp.FailOpenids)
	} else {
		mylog.Printf("[CQ:set_group] %s: 已提交黑名单 op=%s group=%s 共 %d 人", action, op, groupOpenID, len(openIDs))
	}
	return "" // 无论成败都不发送原文
}

// cqSetGroupUserIDs 解析 kick/blacklist 的 user_id 与 user_ids 参数：
// user_ids 按逗号切分；两者同时存在时合并后去重保序；空项过滤；超 20 截断并警告。
func cqSetGroupUserIDs(params map[string]string, match string) []string {
	var ids []string
	if v := strings.TrimSpace(params["user_id"]); v != "" {
		ids = append(ids, v)
	}
	if v := strings.TrimSpace(params["user_ids"]); v != "" {
		for _, part := range strings.Split(v, ",") {
			ids = append(ids, strings.TrimSpace(part))
		}
	}
	seen := make(map[string]struct{}, len(ids))
	result := make([]string, 0, len(ids))
	for _, id := range ids {
		if id == "" {
			continue
		}
		if _, ok := seen[id]; ok {
			continue
		}
		seen[id] = struct{}{}
		result = append(result, id)
	}
	if len(result) > 20 {
		mylog.Printf("[CQ:set_group] user_ids 数量 %d 超过官方单批上限 20,已截断: %s", len(result), match)
		result = result[:20]
	}
	return result
}

// cqSetGroupResolveMembers 逐个反查成员 OpenID（32 位原生 OpenID 直接使用），
// 反查失败的跳过并日志、不中断整批。
func cqSetGroupResolveMembers(ids []string, action string) []string {
	openIDs := make([]string, 0, len(ids))
	for _, id := range ids {
		openID, err := resolveMemberOpenID(id)
		if err != nil {
			mylog.Printf("[CQ:set_group] %s: user_id=%s 反查失败,已跳过: %v", action, id, err)
			continue
		}
		openIDs = append(openIDs, openID)
	}
	return openIDs
}

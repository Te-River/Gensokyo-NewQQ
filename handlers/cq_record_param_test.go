package handlers

import (
	"regexp"
	"runtime"
	"testing"
)

// TestCQRecordParam_RegexCapture 直接钉死 record 三条 URL 正则的捕获组语义：
// 尾随 ,asr=... 不得污染 URL，无尾随参数的既有形态行为不变，
// URL 自身含逗号不得被误切（切分点仅为已知参数名 asr）。
// 注意捕获组不含 scheme 字面量（https:// 在组外），与该键既有约定一致
// （见 cq_parse_regression_test.go: url_records = "x.com/a.mp3"）。
func TestCQRecordParam_RegexCapture(t *testing.T) {
	tests := []struct {
		name    string
		pattern *regexp.Regexp
		input   string
		want    string
	}{
		{
			name:    "https带asr",
			pattern: httpsUrlRecordPattern,
			input:   "[CQ:record,file=https://x/a.wav,asr=你好]",
			want:    "x/a.wav",
		},
		{
			name:    "https无asr不变",
			pattern: httpsUrlRecordPattern,
			input:   "[CQ:record,file=https://x/a.wav]",
			want:    "x/a.wav",
		},
		{
			name:    "http带asr",
			pattern: httpUrlRecordPattern,
			input:   "[CQ:record,file=http://x/a.wav,asr=hi]",
			want:    "x/a.wav",
		},
		{
			name:    "http无asr不变",
			pattern: httpUrlRecordPattern,
			input:   "[CQ:record,file=http://x/a.wav]",
			want:    "x/a.wav",
		},
		{
			name:    "base64带asr",
			pattern: base64RecordPattern,
			input:   "[CQ:record,file=base64://dGVzdA==,asr=你好]",
			want:    "dGVzdA==",
		},
		{
			name:    "base64无asr不变",
			pattern: base64RecordPattern,
			input:   "[CQ:record,file=base64://dGVzdA==]",
			want:    "dGVzdA==",
		},
		{
			name:    "URL含逗号无asr完整保留",
			pattern: httpsUrlRecordPattern,
			input:   "[CQ:record,file=https://x/a,b.wav]",
			want:    "x/a,b.wav",
		},
		{
			name:    "URL含逗号且有asr以asr切分",
			pattern: httpsUrlRecordPattern,
			input:   "[CQ:record,file=https://x/a,b.wav,asr=你好]",
			want:    "x/a,b.wav",
		},
		{
			name:    "asr值含转义逗号",
			pattern: httpsUrlRecordPattern,
			input:   "[CQ:record,file=https://x/a.wav,asr=甲&#44;乙]",
			want:    "x/a.wav",
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			m := tt.pattern.FindStringSubmatch(tt.input)
			if len(m) < 2 {
				t.Fatalf("未匹配: %q", tt.input)
			}
			if m[1] != tt.want {
				t.Fatalf("捕获 URL 错误: got %q, want %q（输入 %q）", m[1], tt.want, tt.input)
			}
		})
	}
}

// TestCQRecordParam_LocalRecord 本地语音同样受尾随参数影响，结论：一并修复。
// 平台字面量差异：Windows 分支 `file:///` 吃掉第三个斜杠（file:///tmp/a.wav → 捕获 tmp/a.wav），
// Unix 分支 `file://` 保留前导 /（file:///tmp/a.wav → 捕获 /tmp/a.wav）；
// 两分支字面量都必须带 `(?:,asr=[^\]]*)?`，否则 ,asr= 会被并入捕获组。
func TestCQRecordParam_LocalRecord(t *testing.T) {
	compilePatternsOnce.Do(initPlatformPatterns)
	cq := "[CQ:record,file=file:///tmp/a.wav,asr=你好]"
	want := "tmp/a.wav"
	if runtime.GOOS != "windows" {
		// Unix 分支的 file:// 只吃两个斜杠，第三个斜杠与路径一起落入捕获组
		want = "/tmp/a.wav"
	}
	m := localRecordPattern.FindStringSubmatch(cq)
	if len(m) < 2 {
		t.Fatalf("未匹配: %q", cq)
	}
	if m[1] != want {
		t.Fatalf("本地语音捕获错误: got %q, want %q", m[1], want)
	}
}

// TestCQRecordParam_PipelineEndToEnd 管道级：URL 不含 asr，且整条 CQ 码（含参数）被移除，asr 不泄漏进正文。
func TestCQRecordParam_PipelineEndToEnd(t *testing.T) {
	tests := []struct {
		name     string
		input    string
		wantKey  string
		wantURL  string
		wantText string
	}{
		{
			name:     "https带asr",
			input:    "听[CQ:record,file=https://x/a.wav,asr=你好]",
			wantKey:  "url_records",
			wantURL:  "x/a.wav",
			wantText: "听",
		},
		{
			name:     "http带asr",
			input:    "[CQ:record,file=http://x/a.wav,asr=你好]",
			wantKey:  "url_record",
			wantURL:  "x/a.wav",
			wantText: "",
		},
		{
			name:     "base64带asr",
			input:    "[CQ:record,file=base64://dGVzdA==,asr=你好]",
			wantKey:  "base64_record",
			wantURL:  "dGVzdA==",
			wantText: "",
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			found := map[string][]string{}
			out := ProcessCQCodePipeline(tt.input, found, nil)
			got := found[tt.wantKey]
			if len(got) != 1 || got[0] != tt.wantURL {
				t.Fatalf("%s 提取错误: got %v, want [%s]（found=%v）", tt.wantKey, got, tt.wantURL, found)
			}
			if out != tt.wantText {
				t.Fatalf("正文清理错误: got %q, want %q", out, tt.wantText)
			}
			for k, v := range found {
				for _, s := range v {
					if s == "你好" || s == "asr=你好" {
						t.Fatalf("%s 被 asr 参数污染: %v", k, found)
					}
				}
			}
		})
	}
}

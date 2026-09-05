package cqparse

import "testing"

// BenchmarkTokenizer Tokenizer 简单基准（性能抽查：对比 legacy 正则路径不劣化超过 2x）。
func BenchmarkTokenizer(b *testing.B) {
	inputs := []string{
		"看这张图[CQ:image,file=https://example.com/pic.jpg]好看吗",
		"[CQ:markdown,data={\"content\":\"hi\"}]普通文本[CQ:keyboard,data={\"id\":\"k1\"}]",
		"[CQ:image,file=https://x.com/a.png][CQ:image,file=https://x.com/b.png]正文[CQ:reply,id=100]尾",
	}
	for _, in := range inputs {
		b.Run(in[:min(20, len(in))], func(b *testing.B) {
			b.ReportAllocs()
			for i := 0; i < b.N; i++ {
				doc := Tokenize(in)
				if len(doc.Tokens) == 0 {
					b.Fatal("no tokens")
				}
			}
		})
	}
}

// BenchmarkParseFull Parse 全链路基准（Normalize+Resolve+Splice，无 Deps）。
func BenchmarkParseFull(b *testing.B) {
	in := Input{
		Kind:     InputString,
		String:   "[CQ:markdown,data={\"content\":\"hi\"}]普通文本[CQ:keyboard,data={\"id\":\"k1\"}][CQ:image,file=https://x.com/a.png]",
		GroupID:  "g0123456789abcdef0123456789abcde",
		HasGroup: true,
		UserID:   "user-1",
	}
	b.ReportAllocs()
	for i := 0; i < b.N; i++ {
		if _, _, _, err := Parse(in, nil); err != nil {
			b.Fatal(err)
		}
	}
}

func min(a, b int) int {
	if a < b {
		return a
	}
	return b
}

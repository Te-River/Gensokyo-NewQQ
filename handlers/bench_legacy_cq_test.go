package handlers

import "testing"

// BenchmarkLegacyCQCodePipeline legacy 正则管道基准（与 cqparse.BenchmarkTokenizer 对比用）。
func BenchmarkLegacyCQCodePipeline(b *testing.B) {
	inputs := []string{
		"看这张图[CQ:image,file=https://example.com/pic.jpg]好看吗",
		"[CQ:markdown,data={\"content\":\"hi\"}]普通文本[CQ:keyboard,data={\"id\":\"k1\"}]",
		"[CQ:image,file=https://x.com/a.png][CQ:image,file=https://x.com/b.png]正文[CQ:reply,id=100]尾",
	}
	for _, in := range inputs {
		b.Run(in[:minStr(20, len(in))], func(b *testing.B) {
			foundItems := make(map[string][]string)
			b.ReportAllocs()
			b.ResetTimer()
			for i := 0; i < b.N; i++ {
				for k := range foundItems {
					delete(foundItems, k)
				}
				out := ProcessCQCodePipeline(in, foundItems, nil)
				if out == "\x00invalid" {
					b.Fatal("invalid")
				}
			}
		})
	}
}

func minStr(a, b int) int {
	if a < b {
		return a
	}
	return b
}

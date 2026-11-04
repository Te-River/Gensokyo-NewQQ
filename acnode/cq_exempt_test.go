package acnode

// 修 F5 的谓词镜像用例：超限内容中 base64 图片码的大小写/空白变体均可命中豁免，
// 无关字面不误命中。（仅测纯谓词，不触碰包级 AC 自动机全局状态。）

import "testing"

func TestContainsCQImageBase64Exempt(t *testing.T) {
	cases := []struct {
		name string
		in   string
		want bool
	}{
		{"规范字面", "前[CQ:image,file=base64://QUJD]后", true},
		{"段名键名大写", "前[CQ:IMAGE,FILE=base64://QUJD]后", true},
		{"键值两侧空白", "前[CQ: image , file = base64://QUJD]后", true},
		{"制表/换行空白", "前[CQ:image\t,\nfile\r=\f base64://QUJD]后", true},
		// 值内空白剔除后仍按前缀字面命中（谓词只看 `base64://` 之前的形态）
		{"base64 载荷含空白", "前[CQ:image,file=base64://QU JD==]后", true},
		{"非 base64 图片码不豁免", "[CQ:image,file=https://x/a.png]", false},
		{"其它段 base64 不豁免", "[CQ:record,file=base64://QUJD]", false},
		{"无关文本", "普通消息 [CQ:image]", false},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			if got := containsCQImageBase64Exempt(c.in); got != c.want {
				t.Errorf("containsCQImageBase64Exempt(%q): got %v, want %v", c.in, got, c.want)
			}
		})
	}
}

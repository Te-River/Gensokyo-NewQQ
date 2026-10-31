package Processor

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/template"
)

// linkMdLoadConfig 以完整模板为基底注入 MD 相关配置并加载 config 单例。
func linkMdLoadConfig(t *testing.T, nativeMD, linkText string) {
	t.Helper()
	yml := strings.Replace(template.ConfigTemplate, "native_md : false", "native_md : "+nativeMD, 1)
	yml = strings.Replace(yml, `link_text : ""`, `link_text : "`+linkText+`"`, 1)
	path := filepath.Join(t.TempDir(), "config.yml")
	if err := os.WriteFile(path, []byte(yml), 0600); err != nil {
		t.Fatalf("写入测试配置失败: %v", err)
	}
	if _, err := config.LoadConfig(path, false); err != nil {
		t.Fatalf("加载测试配置失败: %v", err)
	}
}

// native_md 关闭时也必须产出原生 content：模板 params 已废弃，
// msg_type=2 只带 params 没有 content 是无效载荷（F5）。
// link 指令与入群自我介绍共用本函数，两者都由此覆盖。
func TestGenerateMdByConfigBuildsNativeContent(t *testing.T) {
	for _, nativeMD := range []string{"false", "true"} {
		t.Run("native_md="+nativeMD, func(t *testing.T) {
			linkMdLoadConfig(t, nativeMD, "友情链接")
			md, _ := generateMdByConfig()
			if md == nil {
				t.Fatalf("应返回 markdown")
			}
			if len(md.Params) != 0 {
				t.Fatalf("不应再发模板 params: %+v", md.Params)
			}
			if strings.TrimSpace(md.Content) == "" {
				t.Fatalf("content 不能为空: %+v", md)
			}
			if !strings.Contains(md.Content, "友情链接") {
				t.Fatalf("content 应包含链接文本, got %q", md.Content)
			}
		})
	}
}

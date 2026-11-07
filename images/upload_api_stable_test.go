package images

import (
	"bytes"
	"encoding/base64"
	"errors"
	"image"
	"image/png"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/imagehosting"
)

// 本文件钉住「出站字段不得出现图床短签名外链」这一不变量：
// 稳定版 UploadBase64ImageToServerStable 在 oss_type=7（ChatGLM）时必须回退本地公网 URL，
// 而原 UploadBase64ImageToServer 仍原样返回短链（Markdown/keyboard 素材链依赖它转 qqbot-media://）。
// 注入缝是包级全局态，本包用例一律串行执行（不使用 t.Parallel）。

// seamTestPNG 构造可供 image.DecodeConfig 读出尺寸的最小合法 PNG。
func seamTestPNG(t *testing.T) []byte {
	t.Helper()
	var buf bytes.Buffer
	if err := png.Encode(&buf, image.NewRGBA(image.Rect(0, 0, 2, 1))); err != nil {
		t.Fatalf("构造测试 PNG 失败: %v", err)
	}
	return buf.Bytes()
}

// withImageUploadSeams 覆写 oss_type 读取、图床上传、本地公网 URL 上传三个注入缝，
// 用例结束恢复；provider/local 传 nil 表示保留原实现。
func withImageUploadSeams(t *testing.T, ossType int, provider func(string, []byte, string) (string, error), local func(string) (string, error)) {
	t.Helper()
	oldOss, oldProvider, oldLocal := getOssType, uploadProvider, localUploadBehavior
	getOssType = func() int { return ossType }
	if provider != nil {
		uploadProvider = provider
	}
	if local != nil {
		localUploadBehavior = local
	}
	t.Cleanup(func() { getOssType, uploadProvider, localUploadBehavior = oldOss, oldProvider, oldLocal })
}

// TestUploadBase64ImageToServerStableChatGLMFallsBackToLocalURL 稳定版：图床回短签名外链 →
// 回退本地公网 URL（warn 分支），返回值不得是 chatglm 域。
func TestUploadBase64ImageToServerStableChatGLMFallsBackToLocalURL(t *testing.T) {
	const shortURL = "https://t1.chatglm.cn/file/x.png?expired_at=1&sign=a"
	const localURL = "https://gensokyo.example.com/uploadpic/stable.png"
	var providerCalls, localCalls int
	withImageUploadSeams(t, config.OssTypeChatGLM,
		func(provider string, _ []byte, _ string) (string, error) {
			providerCalls++
			if provider != "chatglm" {
				t.Errorf("provider 名应为 chatglm: %q", provider)
			}
			return shortURL, nil
		},
		func(string) (string, error) {
			localCalls++
			return localURL, nil
		})

	got, _, _, err := UploadBase64ImageToServerStable(base64.StdEncoding.EncodeToString(seamTestPNG(t)), nil)
	if err != nil {
		t.Fatalf("回退不应报错: %v", err)
	}
	if got != localURL {
		t.Fatalf("必须返回本地公网 URL: got %q want %q", got, localURL)
	}
	if imagehosting.IsShortLivedImageURL(got) {
		t.Fatalf("返回值不得是短签名域: %q", got)
	}
	if providerCalls != 1 || localCalls != 1 {
		t.Fatalf("应各调用一次图床与本地链: provider=%d local=%d", providerCalls, localCalls)
	}
}

// TestUploadBase64ImageToServerKeepsShortURLForMediaChain 原函数语义不变：
// 素材链依赖短链原样返回（再转 qqbot-media://），稳定化不得下沉到本函数。
func TestUploadBase64ImageToServerKeepsShortURLForMediaChain(t *testing.T) {
	const shortURL = "https://t1.chatglm.cn/file/x.png?expired_at=1&sign=a"
	withImageUploadSeams(t, config.OssTypeChatGLM,
		func(string, []byte, string) (string, error) { return shortURL, nil },
		func(string) (string, error) {
			t.Error("原函数不得触达本地回退链")
			return "", nil
		})

	got, _, _, err := UploadBase64ImageToServer(base64.StdEncoding.EncodeToString(seamTestPNG(t)), nil)
	if err != nil {
		t.Fatalf("原函数不应报错: %v", err)
	}
	if got != shortURL {
		t.Fatalf("原函数必须原样返回图床外链: got %q want %q", got, shortURL)
	}
}

// TestUploadBase64ImageToServerStableFallbackFailure 稳定版回退失败 → 返回错误，
// 不得把短签名外链当成功结果交出去。
func TestUploadBase64ImageToServerStableFallbackFailure(t *testing.T) {
	const shortURL = "https://t1.chatglm.cn/file/x.png?expired_at=1&sign=a"
	withImageUploadSeams(t, config.OssTypeChatGLM,
		func(string, []byte, string) (string, error) { return shortURL, nil },
		func(string) (string, error) { return "", errors.New("本地公网地址不可用") })

	got, _, _, err := UploadBase64ImageToServerStable(base64.StdEncoding.EncodeToString(seamTestPNG(t)), nil)
	if err == nil {
		t.Fatalf("回退失败必须返回错误, got url=%q", got)
	}
	if got != "" {
		t.Fatalf("失败时不得返回 URL: %q", got)
	}
}

// TestUploadBase64ImageToServerStableProviderURLUnchanged 非短签名域 provider 行为零变化：
// 稳定版原样返回图床外链，且不触达本地回退链。
func TestUploadBase64ImageToServerStableProviderURLUnchanged(t *testing.T) {
	cases := []struct {
		name      string
		ossType   int
		provider  string
		providerU string
	}{
		{"bilibili", config.OssTypeBilibili, "bilibili", "https://i0.hdslb.com/bfs/a.png"},
		{"COS", config.OssTypeCOS, "cos", "https://cos.example.com/a.png"},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			localCalled := false
			withImageUploadSeams(t, c.ossType,
				func(provider string, _ []byte, _ string) (string, error) {
					if provider != c.provider {
						t.Errorf("provider 名应为 %s: %q", c.provider, provider)
					}
					return c.providerU, nil
				},
				func(string) (string, error) {
					localCalled = true
					return "https://local.example.com/x.png", nil
				})

			got, _, _, err := UploadBase64ImageToServerStable(base64.StdEncoding.EncodeToString(seamTestPNG(t)), nil)
			if err != nil {
				t.Fatalf("非短签名域不应报错: %v", err)
			}
			if got != c.providerU {
				t.Fatalf("非短签名域外链必须原样返回: got %q want %q", got, c.providerU)
			}
			if localCalled {
				t.Fatal("非短签名域不得触达本地回退链")
			}
		})
	}
}

package images

import (
	"bytes"
	"encoding/base64"
	"errors"
	"image"
	"image/png"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/imagehosting"
)

// 本文件钉住「图床短签名外链不得进入出站字段」这一不变量：
// oss_type=7（ChatGLM）时 UploadBase64ImageToServer 必须回退到本地公网 URL 链。
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

// TestUploadBase64ImageToServerChatGLMFallsBackToLocalURL 图床回短签名外链 →
// 必须回退本地公网 URL（warn 分支），返回值和尺寸都来自回退结果，不得是 chatglm 域。
func TestUploadBase64ImageToServerChatGLMFallsBackToLocalURL(t *testing.T) {
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
		func(base64Image string) (string, error) {
			localCalls++
			return localURL, nil
		})

	got, width, height, err := UploadBase64ImageToServer(base64.StdEncoding.EncodeToString(seamTestPNG(t)), nil)
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
	if width != 2 || height != 1 {
		t.Fatalf("尺寸应取自字节: got %dx%d", width, height)
	}
}

// TestUploadBase64ImageToServerChatGLMFallbackFailure 回退失败 → 按现有错误语义返回错误，
// 不得把短签名外链当成功结果交出去。
func TestUploadBase64ImageToServerChatGLMFallbackFailure(t *testing.T) {
	const shortURL = "https://t1.chatglm.cn/file/x.png?expired_at=1&sign=a"
	withImageUploadSeams(t, config.OssTypeChatGLM,
		func(string, []byte, string) (string, error) { return shortURL, nil },
		func(string) (string, error) { return "", errors.New("本地公网地址不可用") })

	got, _, _, err := UploadBase64ImageToServer(base64.StdEncoding.EncodeToString(seamTestPNG(t)), nil)
	if err == nil {
		t.Fatalf("回退失败必须返回错误, got url=%q", got)
	}
	if got != "" {
		t.Fatalf("失败时不得返回 URL: %q", got)
	}
}

// TestUploadBase64ImageToServerProviderURLUnchanged 非短签名域 provider 行为零变化：
// 原样返回图床外链，且不触达本地回退链。
func TestUploadBase64ImageToServerProviderURLUnchanged(t *testing.T) {
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

			got, _, _, err := UploadBase64ImageToServer(base64.StdEncoding.EncodeToString(seamTestPNG(t)), nil)
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

// TestUploadBase64ImageToServerLocalTypeUsesLocalChain oss_type=0 仍走本地公网 URL 链（注入缝生效）。
func TestUploadBase64ImageToServerLocalTypeUsesLocalChain(t *testing.T) {
	pngBytes := seamTestPNG(t)
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("Content-Type", "image/png")
		w.Write(pngBytes)
	}))
	t.Cleanup(srv.Close)
	localURL := srv.URL + "/uploadpic/a.png"
	withImageUploadSeams(t, config.OssTypeLocal,
		func(string, []byte, string) (string, error) {
			t.Error("oss_type=0 不得触达图床 provider")
			return "", nil
		},
		func(string) (string, error) { return localURL, nil })

	got, _, _, err := UploadBase64ImageToServer(base64.StdEncoding.EncodeToString(pngBytes), nil)
	if err != nil {
		t.Fatalf("oss_type=0 不应报错: %v", err)
	}
	if got != localURL {
		t.Fatalf("oss_type=0 必须返回本地公网 URL: got %q", got)
	}
}

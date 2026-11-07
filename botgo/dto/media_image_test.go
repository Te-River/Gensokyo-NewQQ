package dto

// 素材管理 DTO 宽容解析回归（issue #56）：官方 GET /v2/media-images 实发把 total 与
// created_at/expires_at 以 JSON 字符串下发，标准解码直接报
// "json: cannot unmarshal string into Go struct field MediaImageList.total of type int"，
// 导致拉列表失败 → 配额清理不重试 → 白白降级外链。本文件覆盖三字段 string/number/
// 混排/非法/null 与缺省五种形态，并做 Marshal 语义回归。
// 约定：dto 包此前仅有 websocket_event_test.go 一个测试文件，测试惯例沿用标准库、无第三方依赖。

import (
	"encoding/json"
	"strings"
	"testing"
)

// TestMediaImageListUnmarshalTolerantListValues 复刻线上报文（Issue #56 用户日志原文形态）：
// total 与 created_at/expires_at 全部为字符串数字，解析必须成功且数值正确。
func TestMediaImageListUnmarshalTolerantListValues(t *testing.T) {
	payload := `{"images":[{"image_id":"img-1","url":"qqbot-media://image/v1/t1","file_name":"a.png","preview_url":"https://cos/short","created_at":"1730000000","expires_at":"1730777600"}],"next_cursor":"c2","total":"36"}`
	var list MediaImageList
	if err := json.Unmarshal([]byte(payload), &list); err != nil {
		t.Fatalf("字符串数值报文解析失败: %v", err)
	}
	if list.Total != 36 {
		t.Errorf("Total = %d, want 36", list.Total)
	}
	if list.NextCursor != "c2" {
		t.Errorf("NextCursor = %q, want \"c2\"", list.NextCursor)
	}
	if len(list.Images) != 1 {
		t.Fatalf("len(Images) = %d, want 1", len(list.Images))
	}
	img := list.Images[0]
	if img.CreatedAt != 1730000000 || img.ExpiresAt != 1730777600 {
		t.Errorf("时间戳解析错误: CreatedAt=%d ExpiresAt=%d", img.CreatedAt, img.ExpiresAt)
	}
	if img.ImageID != "img-1" || img.FileName != "a.png" {
		t.Errorf("字符串字段被宽容层影响: %+v", img)
	}
}

// TestMediaImageTolerantFields MediaImage 的 created_at/expires_at 五态：
// string / number / 混排 / 非法串报错 / null 与缺省回零值。
func TestMediaImageTolerantFields(t *testing.T) {
	t.Run("字符串数字", func(t *testing.T) {
		var img MediaImage
		if err := json.Unmarshal([]byte(`{"image_id":"i","created_at":"100","expires_at":"200"}`), &img); err != nil {
			t.Fatalf("解析失败: %v", err)
		}
		if img.CreatedAt != 100 || img.ExpiresAt != 200 {
			t.Errorf("CreatedAt=%d ExpiresAt=%d, want 100/200", img.CreatedAt, img.ExpiresAt)
		}
	})
	t.Run("数字", func(t *testing.T) {
		var img MediaImage
		if err := json.Unmarshal([]byte(`{"image_id":"i","created_at":100,"expires_at":200}`), &img); err != nil {
			t.Fatalf("解析失败: %v", err)
		}
		if img.CreatedAt != 100 || img.ExpiresAt != 200 {
			t.Errorf("CreatedAt=%d ExpiresAt=%d, want 100/200", img.CreatedAt, img.ExpiresAt)
		}
	})
	t.Run("混排", func(t *testing.T) {
		var img MediaImage
		if err := json.Unmarshal([]byte(`{"created_at":"100","expires_at":200}`), &img); err != nil {
			t.Fatalf("解析失败: %v", err)
		}
		if img.CreatedAt != 100 || img.ExpiresAt != 200 {
			t.Errorf("CreatedAt=%d ExpiresAt=%d, want 100/200", img.CreatedAt, img.ExpiresAt)
		}
	})
	t.Run("非法字符串仍报错", func(t *testing.T) {
		for _, payload := range []string{
			`{"created_at":"abc"}`,
			`{"expires_at":"1.5"}`,
			`{"created_at":{"v":1}}`,
			`{"expires_at":true}`,
		} {
			var img MediaImage
			err := json.Unmarshal([]byte(payload), &img)
			if err == nil {
				t.Errorf("非法形态 %s 应报错但解析成功", payload)
			}
		}
	})
	t.Run("null与缺省为零值", func(t *testing.T) {
		var img MediaImage
		if err := json.Unmarshal([]byte(`{"image_id":"i","created_at":null,"expires_at":null}`), &img); err != nil {
			t.Fatalf("null 不应报错: %v", err)
		}
		if img.CreatedAt != 0 || img.ExpiresAt != 0 {
			t.Errorf("null 后应为零值: CreatedAt=%d ExpiresAt=%d", img.CreatedAt, img.ExpiresAt)
		}
		var img2 MediaImage
		if err := json.Unmarshal([]byte(`{"image_id":"i"}`), &img2); err != nil {
			t.Fatalf("缺省字段不应报错: %v", err)
		}
		if img2.CreatedAt != 0 || img2.ExpiresAt != 0 {
			t.Errorf("缺省字段应零值: CreatedAt=%d ExpiresAt=%d", img2.CreatedAt, img2.ExpiresAt)
		}
	})
}

// TestMediaImageListTolerantTotal MediaImageList.total 五态。
func TestMediaImageListTolerantTotal(t *testing.T) {
	t.Run("字符串数字", func(t *testing.T) {
		var list MediaImageList
		if err := json.Unmarshal([]byte(`{"total":"100"}`), &list); err != nil {
			t.Fatalf("解析失败: %v", err)
		}
		if list.Total != 100 {
			t.Errorf("Total = %d, want 100", list.Total)
		}
	})
	t.Run("数字", func(t *testing.T) {
		var list MediaImageList
		if err := json.Unmarshal([]byte(`{"total":100}`), &list); err != nil {
			t.Fatalf("解析失败: %v", err)
		}
		if list.Total != 100 {
			t.Errorf("Total = %d, want 100", list.Total)
		}
	})
	t.Run("负数字符串", func(t *testing.T) {
		var list MediaImageList
		if err := json.Unmarshal([]byte(`{"total":"-5"}`), &list); err != nil {
			t.Fatalf("解析失败: %v", err)
		}
		if list.Total != -5 {
			t.Errorf("Total = %d, want -5", list.Total)
		}
	})
	t.Run("非法仍报错", func(t *testing.T) {
		for _, payload := range []string{`{"total":"abc"}`, `{"total":{}}`, `{"total":[1]}`} {
			var list MediaImageList
			if err := json.Unmarshal([]byte(payload), &list); err == nil {
				t.Errorf("非法形态 %s 应报错但解析成功", payload)
			}
		}
	})
	t.Run("null与缺省为零值", func(t *testing.T) {
		var list MediaImageList
		if err := json.Unmarshal([]byte(`{"total":null}`), &list); err != nil {
			t.Fatalf("null 不应报错: %v", err)
		}
		if list.Total != 0 {
			t.Errorf("Total = %d, want 0", list.Total)
		}
		var list2 MediaImageList
		if err := json.Unmarshal([]byte(`{}`), &list2); err != nil {
			t.Fatalf("缺省不应报错: %v", err)
		}
		if list2.Total != 0 {
			t.Errorf("Total = %d, want 0", list2.Total)
		}
	})
}

// TestMediaImageCompleteResponseTolerantExpiresAt CompleteResponse 的 expires_at 宽容，
// 并核查 ttl/expires_in 等其余数值字段保持严格解码（本缺陷链路与官方样例均为 number）。
func TestMediaImageCompleteResponseTolerantExpiresAt(t *testing.T) {
	t.Run("字符串数字", func(t *testing.T) {
		var resp MediaImageCompleteResponse
		if err := json.Unmarshal([]byte(`{"image_id":"i","url":"u","ttl":7776000,"expires_at":"1730777600"}`), &resp); err != nil {
			t.Fatalf("解析失败: %v", err)
		}
		if resp.ExpiresAt != 1730777600 {
			t.Errorf("ExpiresAt = %d, want 1730777600", resp.ExpiresAt)
		}
		if resp.TTL != 7776000 {
			t.Errorf("TTL = %d, want 7776000", resp.TTL)
		}
	})
	t.Run("数字", func(t *testing.T) {
		var resp MediaImageCompleteResponse
		if err := json.Unmarshal([]byte(`{"expires_at":123}`), &resp); err != nil {
			t.Fatalf("解析失败: %v", err)
		}
		if resp.ExpiresAt != 123 {
			t.Errorf("ExpiresAt = %d, want 123", resp.ExpiresAt)
		}
	})
	t.Run("缺失与null为零值", func(t *testing.T) {
		var resp MediaImageCompleteResponse
		if err := json.Unmarshal([]byte(`{"image_id":"i","ttl":10}`), &resp); err != nil {
			t.Fatalf("解析失败: %v", err)
		}
		if resp.ExpiresAt != 0 {
			t.Errorf("缺失时 ExpiresAt = %d, want 0", resp.ExpiresAt)
		}
		var resp2 MediaImageCompleteResponse
		if err := json.Unmarshal([]byte(`{"expires_at":null}`), &resp2); err != nil {
			t.Fatalf("null 不应报错: %v", err)
		}
		if resp2.ExpiresAt != 0 {
			t.Errorf("null 时 ExpiresAt = %d, want 0", resp2.ExpiresAt)
		}
	})
	t.Run("非法仍报错", func(t *testing.T) {
		var resp MediaImageCompleteResponse
		if err := json.Unmarshal([]byte(`{"expires_at":"abc"}`), &resp); err == nil {
			t.Error("非数字字符串应报错")
		}
		var resp2 MediaImageCompleteResponse
		if err := json.Unmarshal([]byte(`{"ttl":"7776000"}`), &resp2); err == nil {
			t.Error("ttl 为字符串应报错（核查结论：非缺陷链路字段，保持严格）")
		}
	})
}

// TestMediaImageFlexMarshalKeepsNumber 回归：自定义 UnmarshalJSON 不改序列化形态，
// 数值字段 Marshal 仍输出 number、omitempty 的 expires_at 零值仍被省略。
func TestMediaImageFlexMarshalKeepsNumber(t *testing.T) {
	data, err := json.Marshal(&MediaImage{ImageID: "i", CreatedAt: 100, ExpiresAt: 200})
	if err != nil {
		t.Fatalf("Marshal 失败: %v", err)
	}
	got := string(data)
	if !strings.Contains(got, `"created_at":100`) || !strings.Contains(got, `"expires_at":200`) {
		t.Errorf("Marshal 应输出 number, got %s", got)
	}
	if strings.Contains(got, `"created_at":"100"`) {
		t.Errorf("Marshal 不应输出字符串, got %s", got)
	}
	// omitempty 语义不变：ExpiresAt=0 时整个 key 省略
	data2, err := json.Marshal(&MediaImageCompleteResponse{ImageID: "i"})
	if err != nil {
		t.Fatalf("Marshal 失败: %v", err)
	}
	if strings.Contains(string(data2), "expires_at") {
		t.Errorf("omitempty 语义被破坏: got %s", data2)
	}
}

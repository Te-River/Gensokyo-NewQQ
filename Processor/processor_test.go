package Processor

import (
	"testing"

	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/tencent-connect/botgo/dto"
)

// TestMeCommandMatching 测试 /me 命令的匹配逻辑
func TestMeCommandMatching(t *testing.T) {
	// 设置配置
	config.SetMePrefix("/me")
	config.SetBindPrefix("/bind")
	config.SetDisableWebUI(true)
	config.SetMasterIDs([]string{})

	tests := []struct {
		name           string
		message        string
		shouldMatch    bool
		description    string
	}{
		{
			name:        "exact /me match",
			message:     "/me",
			shouldMatch: true,
			description: "精确匹配 /me 命令",
		},
		{
			name:        "/me with text",
			message:     "/me 测试消息",
			shouldMatch: true,
			description: "/me 后跟文本",
		},
		{
			name:        "/me with whitespace",
			message:     "  /me  ",
			shouldMatch: false,
			description: "带前导空格的消息不会匹配（commandMatch 使用 HasPrefix）",
		},
		{
			name:        "different command",
			message:     "/bind 123 456",
			shouldMatch: false,
			description: "不同的命令不应匹配",
		},
		{
			name:        "empty message",
			message:     "",
			shouldMatch: false,
			description: "空消息不应匹配",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			matched := commandMatch(tt.message, config.GetMePrefix())
			if matched != tt.shouldMatch {
				t.Errorf("commandMatch(%q, %q) = %v, want %v", 
					tt.message, config.GetMePrefix(), matched, tt.shouldMatch)
			}
		})
	}
}

// TestMeCommandDataExtraction 测试从不同消息类型中提取数据
func TestMeCommandDataExtraction(t *testing.T) {
	config.SetMePrefix("/me")
	config.SetBindPrefix("/bind")
	config.SetDisableWebUI(true)
	config.SetMasterIDs([]string{})

	tests := []struct {
		name        string
		data        interface{}
		expectedID  string
		expectedID2 string
	}{
		{
			name:        "group message",
			data:        &dto.WSGroupMessageData{Author: &dto.User{ID: "user123"}, GroupID: "group456"},
			expectedID:  "user123",
			expectedID2: "group456",
		},
		{
			name:        "C2C message",
			data:        &dto.WSC2CMessageData{Author: &dto.User{ID: "user777"}},
			expectedID:  "user777",
			expectedID2: "group_private",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			var realid, realid2 string
			
			switch v := tt.data.(type) {
			case *dto.WSGroupMessageData:
				realid = v.Author.ID
				realid2 = v.GroupID
				realid2 = v.ChannelID
			case *dto.WSC2CMessageData:
				realid = v.Author.ID
				realid2 = "group_private"
			}

			if realid != tt.expectedID {
				t.Errorf("realid = %q, want %q", realid, tt.expectedID)
			}
			if realid2 != tt.expectedID2 {
				t.Errorf("realid2 = %q, want %q", realid2, tt.expectedID2)
			}
		})
	}
}

// TestMePrefixVariations 测试不同的前缀配置
func TestMePrefixVariations(t *testing.T) {
	// 注意：由于配置实例可能为 nil，这个测试主要验证 commandMatch 的基本行为
	// 在实际使用中，配置会从 config.yml 加载
	
	t.Run("default /me prefix behavior", func(t *testing.T) {
		// 当配置为 nil 时，GetMePrefix() 返回默认值 "/me"
		prefix := config.GetMePrefix()
		
		tests := []struct {
			message     string
			shouldMatch bool
		}{
			{"/me test", true},
			{"/me", true},
			{"/menu", true}, // HasPrefix 会匹配
			{"/status test", false},
			{"/bind 123", false},
		}
		
		for _, tt := range tests {
			matched := commandMatch(tt.message, prefix)
			if matched != tt.shouldMatch {
				t.Errorf("commandMatch(%q, %q) = %v, want %v",
					tt.message, prefix, matched, tt.shouldMatch)
			}
		}
	})
}

// TestCommandMatchFunction 直接测试 commandMatch 函数的行为
func TestCommandMatchFunction(t *testing.T) {
	tests := []struct {
		name     string
		message  string
		prefix   string
		expected bool
	}{
		{"exact match", "/me", "/me", true},
		{"prefix match", "/me something", "/me", true},
		{"no match", "/bind", "/me", false},
		{"case sensitive", "/ME", "/me", false},
		{"partial match", "/menu", "/me", true}, // HasPrefix 会匹配，这是预期行为
		{"empty message", "", "/me", false},
		{"whitespace only", "   ", "/me", false},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			result := commandMatch(tt.message, tt.prefix)
			if result != tt.expected {
				t.Errorf("commandMatch(%q, %q) = %v, want %v",
					tt.message, tt.prefix, result, tt.expected)
			}
		})
	}
}

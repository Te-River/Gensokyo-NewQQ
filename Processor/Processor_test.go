package Processor

import (
	"context"
	"os"
	"strings"
	"testing"
	"time"

	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/structs"
	"github.com/hoshinonyaruko/gensokyo/template"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

// Mock API implementations for testing
type MockOpenAPI struct {
	openapi.OpenAPI // embed standard interface

	postGroupMsgChan chan string
	postMsgChan      chan string
	postC2CMsgChan   chan string
}

func (m *MockOpenAPI) MeGuilds(ctx context.Context, pager *dto.GuildPager) ([]*dto.Guild, error) {
	return []*dto.Guild{
		{ID: "guild123", Name: "Guild123"},
	}, nil
}

func (m *MockOpenAPI) Channels(ctx context.Context, guildID string) ([]*dto.Channel, error) {
	return []*dto.Channel{
		{
			ID: "channel123",
			ChannelValueObject: dto.ChannelValueObject{
				Name: "Channel123",
				Type: dto.ChannelTypeText,
			},
		},
	}, nil
}

func (m *MockOpenAPI) PostMessage(ctx context.Context, channelID string, msg *dto.MessageToCreate) (*dto.Message, error) {
	m.postMsgChan <- msg.Content
	return &dto.Message{}, nil
}

func (m *MockOpenAPI) PostGroupMessage(ctx context.Context, groupID string, msg dto.APIMessage) (*dto.GroupMessageResponse, error) {
	if tc, ok := msg.(*dto.MessageToCreate); ok {
		m.postGroupMsgChan <- tc.Content
	}
	return &dto.GroupMessageResponse{}, nil
}

func (m *MockOpenAPI) PostC2CMessage(ctx context.Context, userID string, msg dto.APIMessage) (*dto.C2CMessageResponse, error) {
	if tc, ok := msg.(*dto.MessageToCreate); ok {
		m.postC2CMsgChan <- tc.Content
	}
	return &dto.C2CMessageResponse{}, nil
}

func TestHandleFrameworkCommandStatusAndBroadcast(t *testing.T) {
	// Write a complete config file from template to prevent restart exits
	configStr := strings.Replace(template.ConfigTemplate, `master_id : ["1","2"]`, `master_id : ["12345"]`, 1)
	err := os.WriteFile("config.yml", []byte(configStr), 0644)
	if err != nil {
		t.Fatalf("failed to write dummy config: %v", err)
	}
	defer os.Remove("config.yml")

	// Load configuration with fastload = false to initialize "instance"
	_, err = config.LoadConfig("config.yml", false)
	if err != nil {
		t.Fatalf("failed to load config: %v", err)
	}

	// Initialize idmap database
	idmap.InitializeDB()
	defer func() {
		idmap.CloseDB()
		os.Remove("idmap.db")
	}()

	mockApi := &MockOpenAPI{
		postGroupMsgChan: make(chan string, 10),
		postMsgChan:      make(chan string, 10),
		postC2CMsgChan:   make(chan string, 10),
	}

	p := &Processors{
		Api:      mockApi,
		Apiv2:    mockApi,
		Settings: &structs.Settings{
			MasterID: []string{"12345"}, // Set administrator ID
		},
	}

	// 1. Test -status command
	data := &dto.WSC2CMessageData{
		ID: "msg123",
		Author: &dto.User{
			ID: "12345", // Admin user
		},
	}

	// Call HandleFrameworkCommand for -status (under type "group_private")
	err = p.HandleFrameworkCommand("-status", data, "group_private")
	if err != nil {
		t.Fatalf("HandleFrameworkCommand failed: %v", err)
	}

	// Verify status message response
	select {
	case content := <-mockApi.postC2CMsgChan:
		if !strings.Contains(content, "Gensokyo Bot Status") {
			t.Errorf("expected status message containing 'Gensokyo Bot Status', got '%s'", content)
		}
	case <-time.After(3 * time.Second):
		t.Fatal("timeout waiting for status message")
	}

	// 2. Test -broadcast command
	err = p.HandleFrameworkCommand("-broadcast Hello World!", data, "group_private")
	if err != nil {
		t.Fatalf("HandleFrameworkCommand failed: %v", err)
	}

	// We expect the broadcast logic to fetch guilds, fetch channels, and post a message to "channel123"
	// Wait a moment for async goroutine to push to channel
	select {
	case content := <-mockApi.postMsgChan:
		if content != "Hello World!" {
			t.Errorf("expected broadcast message 'Hello World!', got '%s'", content)
		}
	case <-time.After(3 * time.Second):
		t.Fatal("timeout waiting for broadcast message")
	}
}

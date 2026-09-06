package handlers

import (
	"context"
	"strings"
	"testing"

	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

// TestDecodeStrictJSONRejectsUnknownField m1 修复:未知字段必须报错并指出字段名,
// 而不是被 Marshal→Unmarshal 静默丢弃后随覆盖式提交清空既有配置。
func TestDecodeStrictJSONRejectsUnknownField(t *testing.T) {
	t.Run("未知字段报错并指出字段名", func(t *testing.T) {
		raw := []byte(`{"items":[{"name":"菜单","type":"link","link":"https://example.com","sub_menus":[]}]}`)
		var menu dto.Menu
		err := decodeStrictJSON(raw, &menu)
		if err == nil {
			t.Fatal("含未知字段(sub_menus)的输入应报错")
		}
		if !strings.Contains(err.Error(), "sub_menus") {
			t.Errorf("错误信息应指出未知字段名: %v", err)
		}
	})
	t.Run("合法字段正常解码", func(t *testing.T) {
		raw := []byte(`{"items":[{"name":"菜单","type":"menu","sub_menu_items":[{"name":"子菜单","type":"link","link":"https://example.com"}]}]}`)
		var menu dto.Menu
		if err := decodeStrictJSON(raw, &menu); err != nil {
			t.Fatalf("合法输入不应报错: %v", err)
		}
		if len(menu.Items) != 1 || len(menu.Items[0].SubMenuItems) != 1 {
			t.Errorf("合法输入解码结果异常: %+v", menu)
		}
	})
}

// mockMenuOpenAPI 仅覆盖 PutMenu,记录调用次数
type mockMenuOpenAPI struct {
	openapi.OpenAPI
	putMenuCalls int
}

func (m *mockMenuOpenAPI) PutMenu(ctx context.Context, req *dto.PutMenuRequest) (*dto.MenuVersionResponse, error) {
	m.putMenuCalls++
	return &dto.MenuVersionResponse{Version: 1}, nil
}

// TestSetCustomMenuRejectsUnknownField handler 级验证:含拼错字段(官方 DTO 为 sub_menu_items)
// 的 menu 参数应返回 retcode 100 且不调用官方 API。
func TestSetCustomMenuRejectsUnknownField(t *testing.T) {
	mock := &mockMenuOpenAPI{}
	client := &groupMemberTestClient{}

	msg := callapiActionMessage("set_custom_menu", map[string]interface{}{
		"menu": map[string]interface{}{
			"items": []interface{}{
				map[string]interface{}{
					"name":      "菜单",
					"type":      "link",
					"link":      "https://example.com",
					"sub_menus": []interface{}{}, // 拼错的字段
				},
			},
		},
	})

	_, err := SetCustomMenu(client, nil, mock, msg)
	if err != nil {
		t.Fatalf("SetCustomMenu 返回错误: %v", err)
	}
	if mock.putMenuCalls != 0 {
		t.Errorf("参数无效时不应调用 PutMenu, got %d", mock.putMenuCalls)
	}
	if rc := responseRetCode(t, client.response); rc != 100 {
		t.Errorf("期望 retcode=100, got %v", client.response)
	}
	respMsg, _ := client.response["message"].(string)
	if !strings.Contains(respMsg, "sub_menus") {
		t.Errorf("回执 message 应指出未知字段名: %q", respMsg)
	}
}

package handlers

// get_avatar 契约测试：user_id 既可能是虚拟号也可能是插件手里的 32 位原生 openid。
// 改前：只走 idmap 反查，插件传真实 openid 一律回 1400「无法反查用户 OpenID」；
// 改后：32 位 openid 直通（与 get_user_info / get_group_info 同口径），虚拟号仍反查。

import (
	"encoding/json"
	"strconv"
	"strings"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/idmap"
)

const getAvatarUserOpenID = "D8FB9A0A5F5E4C3B9A1B2C3D4E5F6071"

func decodeGetAvatarReply(t *testing.T, raw string) map[string]interface{} {
	t.Helper()
	var got map[string]interface{}
	if err := json.Unmarshal([]byte(raw), &got); err != nil {
		t.Fatalf("回执不是合法 JSON: %v (%s)", err, raw)
	}
	return got
}

// TestGetAvatarPassesThroughOpenID 插件的 32 位 openid 直通：不再 1400，头像 URL 用该 openid 拼。
func TestGetAvatarPassesThroughOpenID(t *testing.T) {
	client := &contractTestClient{}

	raw, err := GetAvatar(client, nil, nil, callapi.ActionMessage{
		Action: "get_avatar",
		Params: callapi.ParamsContent{UserID: getAvatarUserOpenID},
		Echo:   "avatar-openid",
	})
	if err != nil {
		t.Fatalf("GetAvatar: %v", err)
	}
	got := decodeGetAvatarReply(t, raw)
	if got["retcode"] != float64(0) || got["status"] != "ok" {
		t.Fatalf("32 位 openid 应成功: %s", raw)
	}
	data, _ := got["data"].(map[string]interface{})
	if data == nil {
		t.Fatalf("data 缺失: %s", raw)
	}
	if url, _ := data["message"].(string); !strings.Contains(url, getAvatarUserOpenID) {
		t.Errorf("头像 URL 应使用直通的 openid: %v", data["message"])
	}
}

// TestGetAvatarResolvesVirtualUserID 虚拟号仍按 idmap 反查出真实 openid。
func TestGetAvatarResolvesVirtualUserID(t *testing.T) {
	virtual, err := idmap.StoreIDv2(getAvatarUserOpenID)
	if err != nil {
		t.Fatalf("StoreIDv2: %v", err)
	}
	client := &contractTestClient{}

	raw, err := GetAvatar(client, nil, nil, callapi.ActionMessage{
		Action: "get_avatar",
		Params: callapi.ParamsContent{UserID: strconv.FormatInt(virtual, 10)},
		Echo:   "avatar-virtual",
	})
	if err != nil {
		t.Fatalf("GetAvatar: %v", err)
	}
	got := decodeGetAvatarReply(t, raw)
	if got["retcode"] != float64(0) {
		t.Fatalf("虚拟号应能反查: %s", raw)
	}
	data, _ := got["data"].(map[string]interface{})
	if url, _ := data["message"].(string); !strings.Contains(url, getAvatarUserOpenID) {
		t.Errorf("头像 URL 应使用反查后的 openid: %v", data["message"])
	}
}

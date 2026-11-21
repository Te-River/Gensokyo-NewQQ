package handlers

// 回归（v016 活体对照）：send_private_msg 的 C2C 回执曾把入参虚拟号再喂给 StoreIDv2，
// 另分配一个号回给调用方（实测 610636458 → 813412877），并在 idmap 里凭空多一条脏映射。

import (
	"strconv"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/tencent-connect/botgo/dto"
)

// TestSendC2CResponseEchoesVirtualUserID 虚拟号入参两次调用都原样回显，且 idmap 不新增行。
func TestSendC2CResponseEchoesVirtualUserID(t *testing.T) {
	const virtualUserID = "610636458"
	client := &contractTestClient{}
	msg := callapi.ActionMessage{
		Action: "send_private_msg",
		Params: callapi.ParamsContent{UserID: virtualUserID},
		Echo:   "echo-c2c-echo",
	}

	for i := 0; i < 2; i++ {
		raw, err := SendC2CResponse(client, nil, &msg, &dto.C2CMessageResponse{
			Message: &dto.Message{ID: "real-c2c-" + strconv.Itoa(i)},
		})
		if err != nil {
			t.Fatalf("SendC2CResponse: %v", err)
		}
		if client.last["user_id"] != float64(610636458) {
			t.Fatalf("第 %d 次回执 user_id = %v, want 原样回显 610636458", i+1, client.last["user_id"])
		}
		if got := decodeActionReceipt(t, raw); got["user_id"] != float64(610636458) {
			t.Errorf("第 %d 次返回体 user_id = %v, want 610636458", i+1, got["user_id"])
		}
	}

	// 虚拟号不得被当成 OpenID 再建映射（脏数据）
	if mapped, err := idmap.RetrieveRowByIDv2(virtualUserID); err == nil && mapped != "" {
		t.Fatalf("虚拟号 %s 被 StoreIDv2 写进了 idmap: %q", virtualUserID, mapped)
	}
}

// TestSendC2CResponseMapsRealOpenID 真实 OpenID 入参仍换算为虚拟号回执（不因上一条修复而退化）。
func TestSendC2CResponseMapsRealOpenID(t *testing.T) {
	const userOpenID = "D8FB9A0A5F5E4C3B9A1B2C3D4E5F6071"
	virtual, err := idmap.StoreIDv2(userOpenID)
	if err != nil {
		t.Fatalf("StoreIDv2(%s): %v", userOpenID, err)
	}

	client := &contractTestClient{}
	msg := callapi.ActionMessage{
		Action: "send_private_msg",
		Params: callapi.ParamsContent{UserID: userOpenID},
		Echo:   "echo-c2c-openid",
	}
	if _, err := SendC2CResponse(client, nil, &msg, &dto.C2CMessageResponse{Message: &dto.Message{ID: "real-c2c-openid"}}); err != nil {
		t.Fatalf("SendC2CResponse: %v", err)
	}
	if client.last["user_id"] != float64(virtual) {
		t.Errorf("回执 user_id = %v, want OpenID 对应的虚拟号 %d", client.last["user_id"], virtual)
	}
}

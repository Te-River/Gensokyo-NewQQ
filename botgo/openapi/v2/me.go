package v2

import (
	"context"

	"github.com/tencent-connect/botgo/dto"
)

// Me 拉取当前用户的信息
func (o *openAPIv2) Me(ctx context.Context) (*dto.User, error) {
	resp, err := o.request(ctx).
		SetResult(dto.User{}).
		Get(o.getURL(userMeURI))
	if err != nil {
		return nil, err
	}

	return resp.Result().(*dto.User), nil
}

// UserInfo 获取用户基本信息（官方 2026-11-16 新增）
func (o *openAPIv2) UserInfo(ctx context.Context, userOpenID string) (*dto.UserInfo, error) {
	resp, err := o.request(ctx).
		SetResult(dto.UserInfo{}).
		SetPathParam("user_id", userOpenID).
		Get(o.getURL(userInfoURI))
	if err != nil {
		return nil, err
	}
	return resp.Result().(*dto.UserInfo), nil
}

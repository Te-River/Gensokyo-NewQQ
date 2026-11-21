package v1

import (
	"context"
	"fmt"

	"github.com/tencent-connect/botgo/dto"
)

// Me 拉取当前用户的信息
func (o *openAPI) Me(ctx context.Context) (*dto.User, error) {
	resp, err := o.request(ctx).
		SetResult(dto.User{}).
		Get(o.getURL(userMeURI))
	if err != nil {
		return nil, err
	}

	return resp.Result().(*dto.User), nil
}

// UserInfo v1 不支持用户基本信息接口（官方仅提供 v2）
func (o *openAPI) UserInfo(ctx context.Context, userOpenID string) (*dto.UserInfo, error) {
	return nil, fmt.Errorf("v1 openapi does not support UserInfo")
}

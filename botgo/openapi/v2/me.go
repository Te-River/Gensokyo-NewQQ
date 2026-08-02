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

package v2

import (
	"context"

	"github.com/tencent-connect/botgo/dto"
)

// GroupInfo 获取群基本信息
func (o *openAPIv2) GroupInfo(ctx context.Context, groupOpenID string) (*dto.GroupInfo, error) {
	resp, err := o.request(ctx).
		SetResult(dto.GroupInfo{}).
		SetPathParam("group_id", groupOpenID).
		Get(o.getURL(groupInfoURI))
	if err != nil {
		return nil, err
	}
	return resp.Result().(*dto.GroupInfo), nil
}

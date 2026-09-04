package v2

import (
	"context"

	"github.com/tencent-connect/botgo/dto"
)

// GetMenu 获取自定义菜单
func (o *openAPIv2) GetMenu(ctx context.Context) (*dto.MenuResponse, error) {
	resp, err := o.request(ctx).
		SetResult(dto.MenuResponse{}).
		Get(o.getURL(menuURI))
	if err != nil {
		return nil, err
	}
	return resp.Result().(*dto.MenuResponse), nil
}

// PutMenu 设置自定义菜单（覆盖式）
func (o *openAPIv2) PutMenu(ctx context.Context, req *dto.PutMenuRequest) (*dto.MenuVersionResponse, error) {
	resp, err := o.request(ctx).
		SetResult(dto.MenuVersionResponse{}).
		SetBody(req).
		Put(o.getURL(menuURI))
	if err != nil {
		return nil, err
	}
	return resp.Result().(*dto.MenuVersionResponse), nil
}

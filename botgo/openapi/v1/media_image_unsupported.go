package v1

import (
	"context"
	"errors"

	"github.com/tencent-connect/botgo/dto"
)

// 素材管理端点族（/v2/media-images/*）只在 OpenAPI v2 提供。
// v1 实现同样要满足 openapi.OpenAPI，因此这里给出显式的不支持错误，
// 而不是让调用方在 v1 上拿到「方法不存在」的编译期意外。
var errMediaImageUnsupportedInV1 = errors.New("素材管理仅 OpenAPI v2 支持")

// PrepareMediaImage 申请素材上传（v1 不支持）
func (o *openAPI) PrepareMediaImage(_ context.Context, _ *dto.MediaImagePrepareRequest) (*dto.MediaImagePrepareResponse, error) {
	return nil, errMediaImageUnsupportedInV1
}

// CompleteMediaImage 完成素材上传（v1 不支持）
func (o *openAPI) CompleteMediaImage(_ context.Context, _ *dto.MediaImageCompleteRequest) (*dto.MediaImageCompleteResponse, error) {
	return nil, errMediaImageUnsupportedInV1
}

// ListMediaImages 素材列表（v1 不支持）
func (o *openAPI) ListMediaImages(_ context.Context, _ string) (*dto.MediaImageList, error) {
	return nil, errMediaImageUnsupportedInV1
}

// DeleteMediaImage 删除素材（v1 不支持）
func (o *openAPI) DeleteMediaImage(_ context.Context, _ string) error {
	return errMediaImageUnsupportedInV1
}

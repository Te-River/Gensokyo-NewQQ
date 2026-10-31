package v2

import (
	"context"

	"github.com/tencent-connect/botgo/dto"
)

// 素材管理端点（官方 2026-10-21 新增，未进 /autogen/index.html 端点表）
// prepare 与 complete 走 OpenAPI 域名并带 QQBot Authorization；
// PUT presigned_url 指向 COS，必须在调用侧用裸 http 客户端发起（见 handlers/media_images.go）。
const (
	mediaImagePrepareURI  uri = "/v2/media-images/prepare"
	mediaImageCompleteURI uri = "/v2/media-images/complete"
	mediaImagesURI        uri = "/v2/media-images"
	mediaImageDeleteURI   uri = "/v2/media-images/{image_id}/delete"
)

// PrepareMediaImage 申请上传：返回 upload_id 与预签名 URL
func (o *openAPIv2) PrepareMediaImage(ctx context.Context, req *dto.MediaImagePrepareRequest) (*dto.MediaImagePrepareResponse, error) {
	resp, err := o.request(ctx).
		SetResult(dto.MediaImagePrepareResponse{}).
		SetBody(req).
		Post(o.getURL(mediaImagePrepareURI))
	if err != nil {
		return nil, err
	}
	return resp.Result().(*dto.MediaImagePrepareResponse), nil
}

// CompleteMediaImage 完成上传：md5 取 PUT 响应 ETag 去引号后的值
func (o *openAPIv2) CompleteMediaImage(ctx context.Context, req *dto.MediaImageCompleteRequest) (*dto.MediaImageCompleteResponse, error) {
	resp, err := o.request(ctx).
		SetResult(dto.MediaImageCompleteResponse{}).
		SetBody(req).
		Post(o.getURL(mediaImageCompleteURI))
	if err != nil {
		return nil, err
	}
	return resp.Result().(*dto.MediaImageCompleteResponse), nil
}

// ListMediaImages 素材图片列表（cursor 为空表示首页，按上传时间倒序）
func (o *openAPIv2) ListMediaImages(ctx context.Context, cursor string) (*dto.MediaImageList, error) {
	request := o.request(ctx).SetResult(dto.MediaImageList{})
	if cursor != "" {
		request = request.SetQueryParam("cursor", cursor)
	}
	resp, err := request.Get(o.getURL(mediaImagesURI))
	if err != nil {
		return nil, err
	}
	return resp.Result().(*dto.MediaImageList), nil
}

// DeleteMediaImage 删除素材图片（释放数量配额，引用链接立即失效）
func (o *openAPIv2) DeleteMediaImage(ctx context.Context, imageID string) error {
	_, err := o.request(ctx).
		SetPathParam("image_id", imageID).
		Delete(o.getURL(mediaImageDeleteURI))
	return err
}

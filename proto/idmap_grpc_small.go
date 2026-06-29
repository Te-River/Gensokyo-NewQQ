//go:build small

package proto

import (
	context "context"
)

// IDMapServiceClient is the client API for IDMapService service (stub for small build).
// This stub avoids importing the entire google.golang.org/grpc package.
type IDMapServiceClient interface {
	StoreIDV2(ctx context.Context, in *StoreIDRequest, opts ...interface{}) (*StoreIDResponse, error)
	RetrieveRowByIDV2(ctx context.Context, in *RetrieveRowByIDRequest, opts ...interface{}) (*RetrieveRowByIDResponse, error)
	WriteConfigV2(ctx context.Context, in *WriteConfigRequest, opts ...interface{}) (*WriteConfigResponse, error)
	ReadConfigV2(ctx context.Context, in *ReadConfigRequest, opts ...interface{}) (*ReadConfigResponse, error)
	UpdateVirtualValueV2(ctx context.Context, in *UpdateVirtualValueRequest, opts ...interface{}) (*UpdateVirtualValueResponse, error)
	RetrieveRealValueV2(ctx context.Context, in *RetrieveRealValueRequest, opts ...interface{}) (*RetrieveRealValueResponse, error)
	RetrieveRealValueV2Pro(ctx context.Context, in *RetrieveRealValueRequestPro, opts ...interface{}) (*RetrieveRealValueResponsePro, error)
	RetrieveVirtualValueV2(ctx context.Context, in *RetrieveVirtualValueRequest, opts ...interface{}) (*RetrieveVirtualValueResponse, error)
	StoreIDV2Pro(ctx context.Context, in *StoreIDProRequest, opts ...interface{}) (*StoreIDProResponse, error)
	RetrieveRowByIDV2Pro(ctx context.Context, in *RetrieveRowByIDProRequest, opts ...interface{}) (*RetrieveRowByIDProResponse, error)
	RetrieveVirtualValueV2Pro(ctx context.Context, in *RetrieveVirtualValueProRequest, opts ...interface{}) (*RetrieveVirtualValueProResponse, error)
	UpdateVirtualValueV2Pro(ctx context.Context, in *UpdateVirtualValueProRequest, opts ...interface{}) (*UpdateVirtualValueProResponse, error)
	SimplifiedStoreIDV2(ctx context.Context, in *SimplifiedStoreIDRequest, opts ...interface{}) (*SimplifiedStoreIDResponse, error)
	FindSubKeysByIdPro(ctx context.Context, in *FindSubKeysRequest, opts ...interface{}) (*FindSubKeysResponse, error)
	DeleteConfigV2(ctx context.Context, in *DeleteConfigRequest, opts ...interface{}) (*DeleteConfigResponse, error)
	StoreCacheV2(ctx context.Context, in *StoreCacheRequest, opts ...interface{}) (*StoreCacheResponse, error)
	RetrieveRowByCacheV2(ctx context.Context, in *RetrieveRowByCacheRequest, opts ...interface{}) (*RetrieveRowByCacheResponse, error)
}

// IDMapServiceServer is the server API for IDMapService service (stub for small build).
type IDMapServiceServer interface {
	StoreIDV2(context.Context, *StoreIDRequest) (*StoreIDResponse, error)
	RetrieveRowByIDV2(context.Context, *RetrieveRowByIDRequest) (*RetrieveRowByIDResponse, error)
	WriteConfigV2(context.Context, *WriteConfigRequest) (*WriteConfigResponse, error)
	ReadConfigV2(context.Context, *ReadConfigRequest) (*ReadConfigResponse, error)
	UpdateVirtualValueV2(context.Context, *UpdateVirtualValueRequest) (*UpdateVirtualValueResponse, error)
	RetrieveRealValueV2(context.Context, *RetrieveRealValueRequest) (*RetrieveRealValueResponse, error)
	RetrieveRealValueV2Pro(context.Context, *RetrieveRealValueRequestPro) (*RetrieveRealValueResponsePro, error)
	RetrieveVirtualValueV2(context.Context, *RetrieveVirtualValueRequest) (*RetrieveVirtualValueResponse, error)
	StoreIDV2Pro(context.Context, *StoreIDProRequest) (*StoreIDProResponse, error)
	RetrieveRowByIDV2Pro(context.Context, *RetrieveRowByIDProRequest) (*RetrieveRowByIDProResponse, error)
	RetrieveVirtualValueV2Pro(context.Context, *RetrieveVirtualValueProRequest) (*RetrieveVirtualValueProResponse, error)
	UpdateVirtualValueV2Pro(context.Context, *UpdateVirtualValueProRequest) (*UpdateVirtualValueProResponse, error)
	SimplifiedStoreIDV2(context.Context, *SimplifiedStoreIDRequest) (*SimplifiedStoreIDResponse, error)
	FindSubKeysByIdPro(context.Context, *FindSubKeysRequest) (*FindSubKeysResponse, error)
	DeleteConfigV2(context.Context, *DeleteConfigRequest) (*DeleteConfigResponse, error)
	StoreCacheV2(context.Context, *StoreCacheRequest) (*StoreCacheResponse, error)
	RetrieveRowByCacheV2(context.Context, *RetrieveRowByCacheRequest) (*RetrieveRowByCacheResponse, error)
	mustEmbedUnimplementedIDMapServiceServer()
}

// UnimplementedIDMapServiceServer must be embedded to have forward compatible implementations.
type UnimplementedIDMapServiceServer struct{}

func (UnimplementedIDMapServiceServer) mustEmbedUnimplementedIDMapServiceServer() {}

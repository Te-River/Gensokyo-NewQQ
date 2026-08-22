package v1

import (
	"context"
	"fmt"

	"github.com/tencent-connect/botgo/dto"
)

// GroupInfo v1 不支持群聊接口
func (o *openAPI) GroupInfo(ctx context.Context, groupOpenID string) (*dto.GroupInfo, error) {
	return nil, fmt.Errorf("v1 openapi does not support GroupInfo")
}

package httpapi

import (
	"github.com/gin-gonic/gin"
	"github.com/tencent-connect/botgo/openapi"
)

// RouteEntry 定义一条 HTTP API 路由
type RouteEntry struct {
	Path    string
	Handler func(c *gin.Context, api openapi.OpenAPI, apiV2 openapi.OpenAPI)
}

// RouteTable 路由表
var RouteTable []RouteEntry

// RegisterRoute 注册一条 HTTP API 路由
func RegisterRoute(path string, handler func(c *gin.Context, api openapi.OpenAPI, apiV2 openapi.OpenAPI)) {
	RouteTable = append(RouteTable, RouteEntry{Path: path, Handler: handler})
}

// RegisterAllRoutes 注册所有 API 路由到 Gin 引擎
func RegisterAllRoutes(r *gin.Engine, api openapi.OpenAPI, apiV2 openapi.OpenAPI) {
	for _, entry := range RouteTable {
		path := entry.Path
		h := entry.Handler
		r.Any(path, func(c *gin.Context) {
			h(c, api, apiV2)
		})
	}
}

// init 初始化路由表
func init() {
	// 注册所有标准 OneBot API 路由
	RegisterRoute("/send_group_msg", func(c *gin.Context, api openapi.OpenAPI, apiV2 openapi.OpenAPI) {
		handleSendGroupMessage(c, api, apiV2)
	})
	RegisterRoute("/send_group_msg_raw", func(c *gin.Context, api openapi.OpenAPI, apiV2 openapi.OpenAPI) {
		handleSendGroupMessageRaw(c, api, apiV2)
	})
	RegisterRoute("/send_private_msg", func(c *gin.Context, api openapi.OpenAPI, apiV2 openapi.OpenAPI) {
		handleSendPrivateMessage(c, api, apiV2)
	})
	RegisterRoute("/send_private_msg_sse", func(c *gin.Context, api openapi.OpenAPI, apiV2 openapi.OpenAPI) {
		handleSendPrivateMessageSSE(c, api, apiV2)
	})
	RegisterRoute("/send_guild_channel_msg", func(c *gin.Context, api openapi.OpenAPI, apiV2 openapi.OpenAPI) {
		handleSendGuildChannelMessage(c, api, apiV2)
	})
	RegisterRoute("/get_group_list", func(c *gin.Context, api openapi.OpenAPI, apiV2 openapi.OpenAPI) {
		handleGetGroupList(c, api, apiV2)
	})
	RegisterRoute("/get_friend_list", func(c *gin.Context, api openapi.OpenAPI, apiV2 openapi.OpenAPI) {
		handleGetFriendList(c, api, apiV2)
	})
	RegisterRoute("/put_interaction", func(c *gin.Context, api openapi.OpenAPI, apiV2 openapi.OpenAPI) {
		handlePutInteraction(c, api, apiV2)
	})
	RegisterRoute("/delete_msg", func(c *gin.Context, api openapi.OpenAPI, apiV2 openapi.OpenAPI) {
		handleDeleteMsg(c, api, apiV2)
	})
	RegisterRoute("/delete_group_msg", func(c *gin.Context, api openapi.OpenAPI, apiV2 openapi.OpenAPI) {
		handleDeleteGroupMsg(c, api, apiV2)
	})
	RegisterRoute("/get_avatar", func(c *gin.Context, api openapi.OpenAPI, apiV2 openapi.OpenAPI) {
		handleGetAvatar(c, api, apiV2)
	})
	RegisterRoute("/get_login_info", func(c *gin.Context, api openapi.OpenAPI, apiV2 openapi.OpenAPI) {
		handleGetLoginInfo(c, api, apiV2)
	})
}
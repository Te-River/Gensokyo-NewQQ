package webui

import (
	"embed"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/openapi"
)

//go:embed dist/*
//go:embed dist/css/*
//go:embed dist/fonts/*
//go:embed dist/icons/*
//go:embed dist/js/*
var content embed.FS

// NewCombinedMiddleware 创建并返回一个带有依赖的中间件闭包
func CombinedMiddleware(api openapi.OpenAPI, apiV2 openapi.OpenAPI) gin.HandlerFunc {
	return func(c *gin.Context) {
		if strings.HasPrefix(c.Request.URL.Path, "/webui/api") {
			// 处理API请求
			appIDStr := config.GetAppIDStr()

			// 对除 /api/login 和 /api/check-login-status 之外的 API 路径进行 Cookie 认证
			filepath := c.Param("filepath")
			isLoginPath := (filepath == "/api/login" && c.Request.Method == http.MethodPost)
			isCheckLoginPath := (filepath == "/api/check-login-status" && c.Request.Method == http.MethodGet)
			if !isLoginPath && !isCheckLoginPath {
				cookieValue, err := c.Cookie("login_cookie")
				if err != nil {
					c.JSON(http.StatusUnauthorized, gin.H{"error": "Missing authentication cookie"})
					c.Abort()
					return
				}
				isValid, err := ValidateCookie(cookieValue)
				if err != nil || !isValid {
					c.JSON(http.StatusUnauthorized, gin.H{"error": "Invalid or expired cookie"})
					c.Abort()
					return
				}
			}
			//todo 完善logs的 get方法 来获取历史日志
			// 检查路径是否匹配 `/api/{uin}/process/logs`
			if strings.HasPrefix(c.Param("filepath"), "/api/") && strings.HasSuffix(c.Param("filepath"), "/process/logs") {
				if c.GetHeader("Upgrade") == "websocket" {
					mylog.WsHandlerWithDependencies(c)
				} else {
					getProcessLogs(c)
				}
				return
			}
			//主页日志
			if c.Param("filepath") == "/api/logs" {
				if c.GetHeader("Upgrade") == "websocket" {
					mylog.WsHandlerWithDependencies(c)
				} else {
					getProcessLogs(c)
				}
				return
			}
			// 如果请求路径与appIDStr匹配，并且请求方法为PUT
			if c.Param("filepath") == appIDStr && c.Request.Method == http.MethodPut {
				HandleAppIDRequest(c)
				return
			}
			//获取状态
			if c.Param("filepath") == "/api/"+appIDStr+"/process/status" {
				HandleProcessStatusRequest(c)
				return
			}
			//获取机器人列表
			if c.Param("filepath") == "/api/accounts" {
				HandleAccountsRequest(c)
				return
			}
			//获取当前选中机器人的配置
			if c.Param("filepath") == "/api/"+appIDStr+"/config" && c.Request.Method == http.MethodGet {
				AccountConfigReadHandler(c)
				return
			}
			//删除当前选中机器人的配置并生成新的配置
			if c.Param("filepath") == "/api/"+appIDStr+"/config" && c.Request.Method == http.MethodDelete {
				handleDeleteConfig(c)
				return
			}
			//结束当前实例的进程
			if c.Param("filepath") == "/api/"+appIDStr+"/process" && c.Request.Method == http.MethodDelete {
				// 正常退出
				os.Exit(0)
				return
			}
			//进程监控
			if c.Param("filepath") == "/api/status" && c.Request.Method == http.MethodGet {
				// 检查操作系统是否不为Android
				if runtime.GOOS != "android" {
					handleSysInfo(c)
				}
				return
			}
			//更新当前选中机器人的配置并重启应用(保持地址不变)
			if c.Param("filepath") == "/api/"+appIDStr+"/config" && c.Request.Method == http.MethodPatch {
				handlePatchConfig(c)
				return
			}
			// 处理/api/login的POST请求
			if c.Param("filepath") == "/api/login" && c.Request.Method == http.MethodPost {
				HandleLoginRequest(c)
				return
			}
			// 处理/api/check-login-status的GET请求
			if c.Param("filepath") == "/api/check-login-status" && c.Request.Method == http.MethodGet {
				HandleCheckLoginStatusRequest(c)
				return
			}
			// 设备信息编辑（QDVC 导入/导出）
			if c.Param("filepath") == "/api/"+appIDStr+"/device" && c.Request.Method == http.MethodGet {
				handleDeviceRead(c, appIDStr)
				return
			}
			if c.Param("filepath") == "/api/"+appIDStr+"/device" && c.Request.Method == http.MethodPatch {
				handleDeviceWrite(c, appIDStr)
				return
			}
			// session token 文件编辑（QDVC 导入/导出）
			if c.Param("filepath") == "/api/"+appIDStr+"/session" && c.Request.Method == http.MethodGet {
				handleSessionRead(c, appIDStr)
				return
			}
			if c.Param("filepath") == "/api/"+appIDStr+"/session" && c.Request.Method == http.MethodPatch {
				handleSessionWrite(c, appIDStr)
				return
			}
			// 根据api名称处理请求
			if c.Param("filepath") == "/api/"+appIDStr+"/api" && c.Request.Method == http.MethodPost {
				handleAccountAPIRelay(c, api, apiV2)
				return
			}
		} else {
			// 否则，处理静态文件请求
			// 如果请求是 "/webui/" ，默认为 "index.html"
			filepathRequested := c.Param("filepath")
			if filepathRequested == "" || filepathRequested == "/" {
				filepathRequested = "index.html"
			}

			// 使用 embed.FS 读取文件内容
			filepathRequested = strings.TrimPrefix(filepathRequested, "/")
			data, err := content.ReadFile("dist/" + filepathRequested)
			if err != nil {
				fmt.Println("Error reading file:", err)
				c.Status(http.StatusNotFound)
				return
			}

			mimeType := getContentType(filepathRequested)

			c.Data(http.StatusOK, mimeType, data)
		}
		// 调用c.Next()以继续处理请求链
		c.Next()
	}
}

// SendMessageRequest 定义了发送消息请求的数据结构
type SendMessageRequest struct {
	Message string `json:"message"`
	ID      string `json:"id"`
}

func getContentType(path string) string {
	switch filepath.Ext(path) {
	case ".html", ".htm":
		return "text/html"
	case ".js", ".mjs":
		return "application/javascript"
	case ".css":
		return "text/css"
	case ".json":
		return "application/json"
	case ".png":
		return "image/png"
	case ".jpg", ".jpeg":
		return "image/jpeg"
	case ".gif":
		return "image/gif"
	case ".webp":
		return "image/webp"
	case ".svg":
		return "image/svg+xml"
	case ".ico":
		return "image/x-icon"
	case ".woff":
		return "font/woff"
	case ".woff2":
		return "font/woff2"
	case ".ttf":
		return "font/ttf"
	case ".otf":
		return "font/otf"
	case ".txt":
		return "text/plain; charset=utf-8"
	default:
		return "application/octet-stream"
	}
}

type ResponseData struct {
	UIN      int64  `json:"uin"`
	Password string `json:"password"`
	Protocol int    `json:"protocol"`
}

type RequestData struct {
	Password string `json:"password"`
}

func HandleAccountsRequest(c *gin.Context) {
	responseData := []gin.H{
		{
			"uin":             config.GetAppID(),
			"predefined":      false,
			"process_created": true,
		},
	}

	c.JSON(http.StatusOK, responseData)
}

// HandleProcessStatusRequest 返回当前实例进程状态
func HandleProcessStatusRequest(c *gin.Context) {
	responseData := gin.H{
		"status":     "running",
		"total_logs": 0,
		"restarts":   0,
		"qr_uri":     nil,
		"details": gin.H{
			"pid":         os.Getpid(),
			"status":      "running",
			"memory_used": getProcessMemoryUsage(),
			"cpu_percent": 0.0,
			"start_time":  getProcessStartTime(),
		},
	}
	c.JSON(http.StatusOK, responseData)
}

// 待完善 从mylog通道取出日志信息,然后一股脑返回
func getProcessLogs(c *gin.Context) {
	c.JSON(200, []interface{}{})
}

func HandleAppIDRequest(c *gin.Context) {
	appIDStr := config.GetAppIDStr()

	// 将 appIDStr 转换为 int64
	uin, err := strconv.ParseInt(appIDStr, 10, 64)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Internal server error"})
		return
	}

	// 解析请求体中的JSON数据
	var requestData RequestData
	if err := c.ShouldBindJSON(&requestData); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": "Invalid request data"})
		return
	}

	// 创建响应数据
	responseData := ResponseData{
		UIN:      uin,
		Password: requestData.Password,
		Protocol: 5,
	}

	// 发送响应
	c.JSON(http.StatusOK, responseData)
}

// AccountConfigReadHandler 是用来处理读取配置文件的HTTP请求的
func AccountConfigReadHandler(c *gin.Context) {
	// 读取config.yml文件
	yamlFile, err := os.ReadFile("config.yml")
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Unable to read config file"})
		return
	}

	// 创建JSON响应
	jsonResponse := gin.H{
		"content": string(yamlFile),
	}

	// 将JSON响应发送回客户端
	c.JSON(http.StatusOK, jsonResponse)
}

// 删除配置的处理函数
func handleDeleteConfig(c *gin.Context) {
	// 这里调用删除配置的函数
	err := config.DeleteConfig() // 假设DeleteConfig接受文件路径作为参数
	if err != nil {
		// 如果删除出现错误，返回服务器错误状态码
		c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
		return
	}
	// 删除成功，返回204 No Content状态码
	c.Status(http.StatusNoContent)
}

// handlePatchConfig 用来处理PATCH请求，更新config.yml文件的内容
func handlePatchConfig(c *gin.Context) {
	// 解析请求体中的JSON数据
	var jsonBody struct {
		Content string `json:"content"`
	}
	if err := c.BindJSON(&jsonBody); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": "Invalid request body"})
		return
	}

	// 使用WriteYAMLToFile将content写入config.yml
	if err := config.WriteYAMLToFile(jsonBody.Content); err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to write to config file"})
		return
	}

	// 如果没有错误，返回成功响应
	c.JSON(http.StatusOK, gin.H{"message": "Config updated successfully"})
}

// HandleLoginRequest处理登录请求
func HandleLoginRequest(c *gin.Context) {
	var json struct {
		Username string `json:"username" binding:"required"`
		Password string `json:"password" binding:"required"`
	}

	if err := c.ShouldBindJSON(&json); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": err.Error()})
		return
	}

	if checkCredentials(json.Username, json.Password) {
		// 如果验证成功，设置cookie
		cookieValue, err := GenerateCookie()
		if err != nil {
			c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to generate cookie"})
			return
		}

		c.SetCookie("login_cookie", cookieValue, 3600*24, "/", "", false, true)

		c.JSON(http.StatusOK, gin.H{
			"isLoggedIn": true,
			"cookie":     cookieValue,
		})
	} else {
		c.JSON(http.StatusUnauthorized, gin.H{
			"isLoggedIn": false,
		})
	}
}

func checkCredentials(username, password string) bool {
	serverUsername := config.GetServerUserName()
	serverPassword := config.GetServerUserPassword()

	return username == serverUsername && password == serverPassword
}

// HandleCheckLoginStatusRequest 检查登录状态的处理函数
func HandleCheckLoginStatusRequest(c *gin.Context) {
	// 从请求中获取cookie
	cookieValue, err := c.Cookie("login_cookie")
	if err != nil {
		// 如果cookie不存在，而不是返回BadRequest(400)，我们返回一个OK(200)的响应
		c.JSON(http.StatusOK, gin.H{"isLoggedIn": false, "error": "Cookie not provided"})
		return
	}

	// 使用ValidateCookie函数验证cookie
	isValid, err := ValidateCookie(cookieValue)
	if err != nil {
		switch err {
		case ErrCookieNotFound:
			c.JSON(http.StatusOK, gin.H{"isLoggedIn": false, "error": "Cookie not found"})
		case ErrCookieExpired:
			c.JSON(http.StatusOK, gin.H{"isLoggedIn": false, "error": "Cookie has expired"})
		default:
			c.JSON(http.StatusInternalServerError, gin.H{"isLoggedIn": false, "error": "Internal server error"})
		}
		return
	}

	if isValid {
		c.JSON(http.StatusOK, gin.H{"isLoggedIn": true})
	} else {
		c.JSON(http.StatusOK, gin.H{"isLoggedIn": false, "error": "Invalid cookie"})
	}
}

// ---------- 进程状态辅助 ----------

var processStartTime = time.Now().Unix()

func getProcessStartTime() int64 {
	return processStartTime
}

// getProcessMemoryUsage 返回当前进程内存使用量（字节）
func getProcessMemoryUsage() int64 {
	var m runtime.MemStats
	runtime.ReadMemStats(&m)
	return int64(m.Alloc)
}

// ---------- 设备信息 / session 文件（QDVC 导入导出） ----------

func deviceConfigPath(uin string) string {
	return filepath.Join(".", "data", "webui-device-"+uin+".json")
}

func sessionConfigPath(uin string) string {
	return filepath.Join(".", "data", "webui-session-"+uin+".json")
}

func ensureDataDir() error {
	return os.MkdirAll(filepath.Join(".", "data"), 0o755)
}

// handleDeviceRead 读取设备信息文件
func handleDeviceRead(c *gin.Context, uin string) {
	data, err := os.ReadFile(deviceConfigPath(uin))
	if err != nil {
		if os.IsNotExist(err) {
			c.JSON(http.StatusOK, gin.H{})
			return
		}
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Unable to read device file"})
		return
	}
	c.Data(http.StatusOK, "application/json", data)
}

// handleDeviceWrite 写入设备信息文件
func handleDeviceWrite(c *gin.Context, uin string) {
	body, err := io.ReadAll(c.Request.Body)
	if err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": "Invalid request body"})
		return
	}
	if err := ensureDataDir(); err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to create data directory"})
		return
	}
	if err := os.WriteFile(deviceConfigPath(uin), body, 0o644); err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to write device file"})
		return
	}
	c.JSON(http.StatusOK, gin.H{"message": "Device config saved successfully"})
}

// handleSessionRead 读取 session token 文件
func handleSessionRead(c *gin.Context, uin string) {
	data, err := os.ReadFile(sessionConfigPath(uin))
	if err != nil {
		if os.IsNotExist(err) {
			c.JSON(http.StatusOK, gin.H{"base64_content": ""})
			return
		}
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Unable to read session file"})
		return
	}
	c.Data(http.StatusOK, "application/json", data)
}

// handleSessionWrite 写入 session token 文件
func handleSessionWrite(c *gin.Context, uin string) {
	body, err := io.ReadAll(c.Request.Body)
	if err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": "Invalid request body"})
		return
	}
	if err := ensureDataDir(); err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to create data directory"})
		return
	}
	if err := os.WriteFile(sessionConfigPath(uin), body, 0o644); err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to write session file"})
		return
	}
	c.JSON(http.StatusOK, gin.H{"message": "Session token saved successfully"})
}

// ---------- /api/{uin}/api OneBot action 代理 ----------

// webuiClient 实现 callapi.Client 接口，捕获 handler 发送的响应
type webuiClient struct {
	response map[string]interface{}
}

func (w *webuiClient) SendMessage(message map[string]interface{}) error {
	w.response = message
	return nil
}

// handleAccountAPIRelay 将前端 POST /api/{uin}/api?name=xxx 的请求代理到对应 OneBot handler
func handleAccountAPIRelay(c *gin.Context, api openapi.OpenAPI, apiv2 openapi.OpenAPI) {
	apiName := c.Query("name")
	if apiName == "" {
		c.JSON(http.StatusBadRequest, gin.H{"error": "Missing API name"})
		return
	}

	// 解析请求体为 params
	body, err := io.ReadAll(c.Request.Body)
	if err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": "Invalid request body"})
		return
	}
	var params map[string]interface{}
	if len(body) > 0 {
		if err := json.Unmarshal(body, &params); err != nil {
			c.JSON(http.StatusBadRequest, gin.H{"error": "Invalid request body JSON"})
			return
		}
	}

	// 构造 ActionMessage 并调用注册的 handler
	message := callapi.ActionMessage{
		Action: apiName,
		Params: buildParamsContent(params),
	}
	client := &webuiClient{}
	result := callapi.CallAPIFromDict(client, api, apiv2, message)
	if result == "" && client.response != nil {
		if b, err := json.Marshal(client.response); err == nil {
			result = string(b)
		}
	}
	if result == "" {
		c.JSON(http.StatusBadRequest, gin.H{"error": "Unsupported or failed API: " + apiName})
		return
	}
	c.Header("Content-Type", "application/json")
	c.String(http.StatusOK, result)
}

// buildParamsContent 将前端传递的 params map 映射为 callapi.ParamsContent
func buildParamsContent(params map[string]interface{}) callapi.ParamsContent {
	var pc callapi.ParamsContent
	if params == nil {
		return pc
	}
	if v, ok := params["group_id"]; ok {
		pc.GroupID = v
	}
	if v, ok := params["user_id"]; ok {
		pc.UserID = v
	}
	if v, ok := params["message_id"]; ok {
		pc.MessageID = v
	}
	if v, ok := params["channel_id"]; ok {
		pc.ChannelID = v
	}
	if v, ok := params["guild_id"]; ok {
		pc.GuildID = v
	}
	if v, ok := params["message"]; ok {
		pc.Message = v
	}
	if v, ok := params["messages"]; ok {
		pc.Messages = v
	}
	if v, ok := params["duration"]; ok {
		if n, ok := v.(float64); ok {
			pc.Duration = int(n)
		}
	}
	if v, ok := params["enable"]; ok {
		if b, ok := v.(bool); ok {
			pc.Enable = b
		}
	}
	if v, ok := params["approve"]; ok {
		if b, ok := v.(bool); ok {
			pc.Approve = b
		}
	}
	if v, ok := params["flag"].(string); ok {
		pc.Flag = v
	}
	if v, ok := params["reason"].(string); ok {
		pc.Reason = v
	}
	return pc
}

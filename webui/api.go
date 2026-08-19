package webui

import (
	"embed"
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"time"

	"github.com/gin-gonic/gin"
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
			// 根据api名称处理请求
			if c.Param("filepath") == "/api/"+appIDStr+"/api" && c.Request.Method == http.MethodPost {
			 apiName := c.Query("name")
			 switch apiName {
			 default:
			  // 处理其他或未知的api名称
			  c.JSON(http.StatusBadRequest, gin.H{"error": "Invalid API name"})
				}
				return
			}
			// 如果还有其他API端点，可以在这里继续添加...
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
	// todo 根据需要增加更多的 MIME 类型
	switch filepath.Ext(path) {
	case ".html":
		return "text/html"
	case ".js":
		return "application/javascript"
	case ".css":
		return "text/css"
	case ".png":
		return "image/png"
	case ".jpg", ".jpeg":
		return "image/jpeg"
	default:
		return "text/plain"
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

func HandleProcessStatusRequest(c *gin.Context) {
	responseData := gin.H{
		"status":     "running",
		"total_logs": 0,
		"restarts":   0,
		"qr_uri":     nil,
		"details": gin.H{
			"pid":         0,
			"status":      "running",
			"memory_used": 19361792,          // 示例内存使用量
			"cpu_percent": 0.0,               // 示例CPU使用率
			"start_time":  time.Now().Unix(), // 10位时间戳
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

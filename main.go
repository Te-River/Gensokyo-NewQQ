package main

import (
	"bufio"
	"bytes"
	"context"
	"errors"
	"flag"
	"fmt"
	"io"
	"log"
	"net/http"
	"os"
	"os/signal"
	"runtime"
	"strings"
	"sync/atomic"
	"syscall"
	"time"

	"github.com/fatih/color"
	"github.com/fsnotify/fsnotify"
	"github.com/hoshinonyaruko/gensokyo/Processor"
	"github.com/hoshinonyaruko/gensokyo/acnode"
	"github.com/hoshinonyaruko/gensokyo/botstats"
	"github.com/hoshinonyaruko/gensokyo/buildinfo"
	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/echo"
	"github.com/hoshinonyaruko/gensokyo/handlers"
	"github.com/hoshinonyaruko/gensokyo/httpapi"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/hoshinonyaruko/gensokyo/server"
	"github.com/hoshinonyaruko/gensokyo/sys"
	"github.com/hoshinonyaruko/gensokyo/template"
	"github.com/hoshinonyaruko/gensokyo/url"
	"github.com/hoshinonyaruko/gensokyo/webui"
	"github.com/hoshinonyaruko/gensokyo/wsclient"
	"github.com/tencent-connect/botgo/sessions/multi"

	"github.com/gin-gonic/gin"
	"github.com/tencent-connect/botgo"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/event"
	"github.com/tencent-connect/botgo/openapi"
	"github.com/tencent-connect/botgo/token"
	"github.com/tencent-connect/botgo/websocket"
)

// 消息处理器，持有 openapi 对象
var p *Processor.Processors

func main() {
	args := os.Args[1:]
	if len(args) > 0 {
		switch args[0] {
		case "version", "-version", "--version":
			fmt.Println(buildinfo.Version())
			return
		case "run":
			args = args[1:]
		}
	}

	fastStart := flag.Bool("faststart", false, "start without initialization if set")
	tidy := flag.Bool("tidy", false, "backup and tidy your config.yml")
	cleanids := flag.Bool("clean_ids", false, "clean msg_id in ids bucket.")
	delids := flag.Bool("del_ids", false, "delete ids bucket, must backup idmap.db first!")
	delcache := flag.Bool("del_cache", false, "delete cache bucket, it is safe")
	compaction := flag.Bool("compaction", false, "compaction for apply db changes.")
	m := flag.Bool("m", false, "Maintenance mode")
	localLogger := flag.String("local-logger", "", "set to enable to write local log files")

	if err := flag.CommandLine.Parse(args); err != nil {
		log.Fatalf("error parsing flags: %v", err)
	}

	if !*fastStart {
		sys.InitBase()
	}
	if *tidy {
		config.CreateAndWriteConfigTemp()
		log.Println("配置文件已更新为新版,当前配置文件已备份.如产生问题请到群196173384反馈开发者。")
		return
	}
	if _, err := os.Stat("config.yml"); os.IsNotExist(err) {
		var ip string
		var err error
		if runtime.GOOS == "android" {
			ip = "127.0.0.1"
		} else {
			ip, err = sys.GetLocalIP()
			if err != nil {
				log.Println("Error retrieving the local IP address:", err)
				ip = "127.0.0.1"
			}
		}
		configData := strings.Replace(template.ConfigTemplate, "<YOUR_SERVER_DIR>", ip, -1)

		err = os.WriteFile("config.yml", []byte(configData), 0644)
		if err != nil {
			log.Println("Error writing config.yml:", err)
			return
		}

		log.Println("请配置config.yml然后再次运行.")
		log.Print("按下 Enter 继续...")
		bufio.NewReader(os.Stdin).ReadBytes('\n')
		os.Exit(0)
	}

	conf, err := config.LoadConfig("config.yml", false)
	if err != nil {
		log.Fatalf("error: %v", err)
	}

	go setupConfigWatcher("config.yml")

	sys.SetTitle(conf.Settings.Title)
	webuiURL := config.ComposeWebUIURL(conf.Settings.Lotus)
	webuiURLv2 := config.ComposeWebUIURLv2(conf.Settings.Lotus)

	var api openapi.OpenAPI
	var apiV2 openapi.OpenAPI
	var wsClients []*wsclient.WebSocketClient
	var nologin bool

	logLevel := mylog.GetLogLevelFromConfig(config.GetLogLevel())
	localFileLogger := config.GetSaveLogs() || strings.EqualFold(strings.TrimSpace(*localLogger), "enable")
	loggerAdapter := mylog.NewMyLogAdapter(logLevel, localFileLogger)
	mylog.SetLogLevel(logLevel)
	botgo.SetLogger(loggerAdapter)

	if *m {
		conf.Settings.WsAddress = []string{"ws://127.0.0.1:50000"}
		conf.Settings.EnableWsServer = false
	}

	webui.InitializeDB()
	defer webui.CloseDB()

	if conf.Settings.AppID == 12345 {
		cyan := color.New(color.FgCyan)
		cyan.Printf("欢迎来到Gensokyo, 控制台地址: %s\n", webuiURL)
		log.Println("请完成机器人配置后重启框架。")

	} else {
		token := token.BotToken(conf.Settings.AppID, conf.Settings.ClientSecret, conf.Settings.Token, token.TypeQQBot)

		ctx := context.Background()
		if err := token.InitToken(ctx); err != nil {
			log.Fatalln(err)
		}

		if len(conf.Settings.TextIntent) == 0 {
			panic(errors.New("TextIntent is empty, at least one intent should be specified"))
		}

		if !conf.Settings.SandBoxMode {
			if err := botgo.SelectOpenAPIVersion(openapi.APIv1); err != nil {
				log.Fatalln(err)
			}
			api = botgo.NewOpenAPI(token).WithTimeout(120 * time.Second)
			log.Println("创建 apiv1 成功")

			if err := botgo.SelectOpenAPIVersion(openapi.APIv2); err != nil {
				log.Fatalln(err)
			}
			apiV2 = botgo.NewOpenAPI(token).WithTimeout(120 * time.Second)
			log.Println("创建 apiv2 成功")
		} else {
			if err := botgo.SelectOpenAPIVersion(openapi.APIv1); err != nil {
				log.Fatalln(err)
			}
			api = botgo.NewSandboxOpenAPI(token).WithTimeout(120 * time.Second)
			log.Println("创建 沙箱 apiv1 成功")

			if err := botgo.SelectOpenAPIVersion(openapi.APIv2); err != nil {
				log.Fatalln(err)
			}
			apiV2 = botgo.NewSandboxOpenAPI(token).WithTimeout(120 * time.Second)
			log.Println("创建 沙箱 apiv2 成功")
		}

		configURL := config.GetDevelop_Acdir()
		fix11300 := config.GetFix11300()
		var me *dto.User
		if configURL == "" && !fix11300 {
			me, err = api.Me(ctx)
			if err != nil {
				log.Printf("Error fetching bot details: %v\n", err)
				nologin = true
			}
			log.Printf("Bot details: %+v\n", me)
		} else {
			log.Printf("自定义ac地址模式...请从日志手动获取bot的真实id并设置,不然at会不正常")
		}

		if !nologin {
			idmap.InitializeDB()
			botstats.InitializeDB()

			defer idmap.CloseDB()
			defer botstats.CloseDB()

			if *delids {
				mylog.Printf("开始删除ids\n")
				idmap.DeleteBucket("ids")
				mylog.Printf("ids删除完成\n")
				return
			}
			if *delcache {
				mylog.Printf("开始删除cache\n")
				idmap.DeleteBucket("cache")
				mylog.Printf("cache删除完成\n")
				return
			}
			if *cleanids {
				mylog.Printf("开始清理ids中的msg_id\n")
				idmap.CleanBucket("ids")
				mylog.Printf("ids清理完成\n")
				return
			}
			if *compaction {
				mylog.Printf("开始整理idmap.db\n")
				idmap.CompactionIdmap()
				mylog.Printf("idmap.db整理完成\n")
				return
			}

			if configURL == "" && !fix11300 {
				handlers.BotID = me.ID
			} else {
				handlers.BotID = config.GetDevBotid()
			}

			handlers.AppID = fmt.Sprintf("%d", conf.Settings.AppID)

			wsInfo, err := apiV2.WS(ctx, nil, "")
			if err != nil {
				log.Fatalln(err)
			}
			fmt.Printf("分片建议\n")
			fmt.Printf("建议的分片数量:%d\n", wsInfo.Shards)
			fmt.Printf("每 24 小时可创建 Session 数:%d\n", wsInfo.SessionStartLimit.Total)
			fmt.Printf("目前还可以创建的 Session 数:%d\n", wsInfo.SessionStartLimit.Remaining)
			fmt.Printf("重置计数的剩余时间(ms):%d\n", wsInfo.SessionStartLimit.ResetAfter)
			fmt.Printf("每 5s 可以创建的 Session 数:%d\n", wsInfo.SessionStartLimit.MaxConcurrency)

			var intent dto.Intent = 0
			enabledHandlers := make(map[string]bool)

			for _, handlerName := range conf.Settings.TextIntent {
				handler, ok := getHandlerByName(handlerName)
				if !ok {
					log.Printf("Unknown handler: %s\n", handlerName)
					continue
				}
				enabledHandlers[handlerName] = true
				intent |= websocket.RegisterHandlers(handler)
			}

			if config.GetDiscoverUnknownEvents() {
				unknownBits := []int{6, 7, 8, 11, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 31}
				for _, bit := range unknownBits {
					intent |= dto.Intent(1 << bit)
				}
				log.Printf("发现未知事件模式已启用，额外订阅的 intent 位: %v", unknownBits)
			}

			if config.GetGlobalGroupMsgRejectReciveEventToMessage() {
				for _, name := range []string{"GroupMsgRejectHandler", "GroupMsgReceiveHandler"} {
					if handler, ok := getHandlerByName(name); ok {
						enabledHandlers[name] = true
						intent |= websocket.RegisterHandlers(handler)
						log.Printf("自动订阅 intent: %s（global_group_msg_rre_to_message 开启）", name)
					}
				}
			}
			intent = applyDisallowedIntentPolicy(intent, enabledHandlers)
			log.Printf("注册 intents: %v\n", intent)

			p = Processor.NewProcessorV2(api, apiV2, &conf.Settings)

			idmap.StartMigration()

			if conf.Settings.ShardCount == 1 {
				go func() {
					wsInfo.Shards = uint32(conf.Settings.ShardNum)
					if wsInfo.Shards == 1 {
						if err = botgo.NewSessionManager().Start(wsInfo, token, &intent); err != nil {
							log.Fatalln(err)
						}
					} else {
						multi.NewShardManager(wsInfo, token, &intent).StartAllShards()
					}
				}()
				log.Printf("不使用分片,所有信息都由当前gensokyo处理...\n")
			} else {
				go func() {
					wsInfoSingle := &dto.WebsocketAPSingle{
						URL:               wsInfo.URL,
						ShardCount:        uint32(conf.Settings.ShardCount),
						ShardID:           uint32(conf.Settings.ShardID),
						SessionStartLimit: wsInfo.SessionStartLimit,
					}
					if err = botgo.NewSessionManager().StartSingle(wsInfoSingle, token, &intent); err != nil {
						log.Fatalln(err)
					}
				}()
				log.Printf("使用%d个分片,当前是第%d个分片\n", conf.Settings.ShardCount, conf.Settings.ShardID)
			}

			if !allEmpty(conf.Settings.WsAddress) {
				wsClientChan := make(chan *wsclient.WebSocketClient, len(conf.Settings.WsAddress))
				errorChan := make(chan error, len(conf.Settings.WsAddress))
				attemptedConnections := 0
				for _, wsAddr := range conf.Settings.WsAddress {
					if wsAddr == "" {
						continue
					}
					attemptedConnections++
					go func(address string) {
						retry := config.GetLaunchReconectTimes()
						var BotID uint64
						if config.GetUseUin() {
							BotID = uint64(config.GetUinint64())
						} else {
							BotID = conf.Settings.AppID
						}
						wsClient, err := wsclient.NewWebSocketClient(address, BotID, api, apiV2, retry)
						if err != nil {
							log.Printf("Error creating WebSocketClient for address(连接到反向ws失败) %s: %v\n", address, err)
							errorChan <- err
							return
						}
						wsClientChan <- wsClient
					}(wsAddr)
				}
				for i := 0; i < attemptedConnections; i++ {
					select {
					case wsClient := <-wsClientChan:
						wsClients = append(wsClients, wsClient)
					case err := <-errorChan:
						log.Printf("Error encountered while initializing WebSocketClient: %v\n", err)
					}
				}

				if len(wsClients) == 0 {
					log.Println("Error: Not all wsClients are initialized!(反向ws未设置或全部连接失败)")
					p = Processor.NewProcessorV2(api, apiV2, &conf.Settings)
				} else {
					log.Println("All wsClients are successfully initialized.")
					p = Processor.NewProcessor(api, apiV2, &conf.Settings, wsClients)
				}
			} else {
				p = Processor.NewProcessorV2(api, apiV2, &conf.Settings)
				if !conf.Settings.EnableWsServer {
					if conf.Settings.HttpAddress != "" {
						conf.Settings.HttpOnlyBot = true
						log.Println("提示,目前只启动了httpapi,正反向ws均未配置.")
					} else {
						log.Println("提示,目前你配置了个寂寞,httpapi没设置,正反ws都没配置.")
					}
				} else {
					if conf.Settings.HttpAddress != "" {
						log.Println("提示,目前启动了正向ws和httpapi,未连接反向ws")
					} else {
						log.Println("提示,目前启动了正向ws,未连接反向ws,httpapi未开启")
					}
				}
			}
		} else {
			red := color.New(color.FgRed)
			red.Println("请设置正确的appid、token、clientsecret再试")
		}
	}

	rateLimiter := server.NewRateLimiter()
	var serverPort string
	if !conf.Settings.Lotus {
		serverPort = conf.Settings.Port
	} else {
		serverPort = conf.Settings.BackupPort
	}
	var r *gin.Engine
	var hr *gin.Engine
	if config.GetDeveloperLog() {
		r = gin.Default()
		hr = gin.Default()
	} else {
		r = gin.New()
		r.Use(gin.Recovery())
		hr = gin.New()
		hr.Use(gin.Recovery())
	}
	if !initLotusGrpc(conf.Settings.Lotus, conf.Settings.LotusGrpc, conf.Settings.LotusGrpcPort) {
		r.GET("/getid", server.IDMapAuthMiddleware(), server.GetIDHandler)
	}

	webhookHandler := server.NewWebhookHandler(5000)

	go webhookHandler.ListenAndProcessMessages()

	uploadAuth := server.UploadAuthMiddleware()

	r.GET("/updateport", uploadAuth, server.HandleIpupdate)
	r.POST("/delpic", uploadAuth, server.DeleteImageHandler(rateLimiter))
	r.GET("/healthz", uploadAuth, HealthzHandler)
	r.GET("/readyz", uploadAuth, HealthzHandler)
	r.GET("/metrics", uploadAuth, MetricsHandler)
	r.POST("/uploadpic", uploadAuth, server.UploadBase64ImageHandler(rateLimiter))
	r.POST("/uploadpicv2", uploadAuth, server.UploadBase64ImageHandlerV2(rateLimiter, apiV2))
	r.POST("/uploadpicv3", uploadAuth, server.UploadBase64ImageHandlerV3(rateLimiter, api))
	r.POST("/uploadrecord", uploadAuth, server.UploadBase64RecordHandler(rateLimiter))

	server.InitPrivateKey(conf.Settings.ClientSecret)

	r.POST("/"+conf.Settings.WebhookPath, UnionFanout(server.CreateHandleValidationSafe(webhookHandler)))

	r.Static("/channel_temp", "./channel_temp")
	if config.GetFrpPort() == "0" && !config.GetDisableWebui() {
		webuiGroup := r.Group("/webui")
		{
			webuiGroup.GET("/*filepath", webui.CombinedMiddleware(api, apiV2))
			webuiGroup.POST("/*filepath", webui.CombinedMiddleware(api, apiV2))
			webuiGroup.PUT("/*filepath", webui.CombinedMiddleware(api, apiV2))
			webuiGroup.DELETE("/*filepath", webui.CombinedMiddleware(api, apiV2))
			webuiGroup.PATCH("/*filepath", webui.CombinedMiddleware(api, apiV2))
		}
	} else {
		mylog.Println("Either FRP port is set to '0' or WebUI is disabled.")
	}

	http_api_address := config.GetHttpAddress()
	if http_api_address != "" {
		mylog.Println("正向http api启动成功,监听" + http_api_address + "若有需要,请对外放通端口...")
		hr.GET("/metrics", MetricsHandler)
		hr.NoRoute(httpapi.CombinedMiddleware(api, apiV2))
	}

	if conf.Settings.AppID != 12345 {
		if conf.Settings.EnableWsServer {
			wspath := config.GetWsServerPath()
			if wspath == "nil" {
				r.GET("", server.WsHandlerWithDependencies(api, apiV2, p))
				mylog.Println("正向ws启动成功,监听0.0.0.0:" + serverPort + "请注意设置ws_server_token(可空),并对外放通端口...")
			} else {
				r.GET("/"+wspath, server.WsHandlerWithDependencies(api, apiV2, p))
				mylog.Println("正向ws启动成功,监听0.0.0.0:" + serverPort + "/" + wspath + "请注意设置ws_server_token(可空),并对外放通端口...")
			}
		}
	}
	r.POST("/url", url.CreateShortURLHandler)
	r.GET("/url/:shortURL", url.RedirectFromShortURLHandler)
	if config.GetIdentifyFile() {
		appIDStr := config.GetAppIDStr()
		fileName := appIDStr + ".json"
		r.GET("/"+fileName, func(c *gin.Context) {
			content := fmt.Sprintf(`{"bot_appid":%d}`, config.GetAppID())
			c.Header("Content-Type", "application/json")
			c.String(200, content)
		})

		identifyAppids := config.GetIdentifyAppids()
		if len(identifyAppids) >= 1 {
			var filteredAppids []int64
			for _, appid := range identifyAppids {
				if appid != int64(config.GetAppID()) {
					filteredAppids = append(filteredAppids, appid)
				}
			}
			for _, appid := range filteredAppids {
				fileName := fmt.Sprintf("%d.json", appid)
				r.GET("/"+fileName, func(c *gin.Context) {
					content := fmt.Sprintf(`{"bot_appid":%d}`, appid)
					c.Header("Content-Type", "application/json")
					c.String(200, content)
				})
			}
		}
	}

	httpServer := &http.Server{
		Addr:    "0.0.0.0:" + serverPort,
		Handler: r,
	}
	mylog.Printf("gin运行在%v端口", serverPort)
	go func() {
		if serverPort == "443" || conf.Settings.ForceSSL {
			crtPath := config.GetCrtPath()
			keyPath := config.GetKeyPath()
			if crtPath == "" || keyPath == "" {
				log.Fatalf("crt or key path is missing for HTTPS")
				return
			}
			if err := httpServer.ListenAndServeTLS(crtPath, keyPath); err != nil && err != http.ErrServerClosed {
				log.Fatalf("listen (HTTPS): %s\n", err)
			}
		} else {
			if err := httpServer.ListenAndServe(); err != nil && err != http.ErrServerClosed {
				log.Fatalf("listen: %s\n", err)
			}
		}
	}()

	if serverPort == "443" || conf.Settings.ForceSSL {
		go func() {
			httpServerHttpPortAfterSSL := &http.Server{
				Addr:    "0.0.0.0:" + conf.Settings.HttpPortAfterSSL,
				Handler: r,
			}
			if err := httpServerHttpPortAfterSSL.ListenAndServe(); err != nil && err != http.ErrServerClosed {
				log.Fatalf("listen (HTTP %s): %s\n", conf.Settings.HttpPortAfterSSL, err)
			}
		}()
	}

	if http_api_address != "" {
		go func() {
			httpServerHttpApi := &http.Server{
				Addr:    http_api_address,
				Handler: hr,
			}
			if err := httpServerHttpApi.ListenAndServe(); err != nil && err != http.ErrServerClosed {
				log.Fatalf("http apilisten: %s\n", err)
			}
		}()
	}

	if conf.Settings.MemoryMsgid {
		echo.StartCleanupRoutine()
	}
	idmap.StartUsernameCacheCleanup()

	cyan := color.New(color.FgCyan)
	cyan.Printf("欢迎来到Gensokyo, 控制台地址: %s\n", webuiURL)
	cyan.Printf("%s\n", template.Logo)
	cyan.Printf("欢迎来到Gensokyo, 公网控制台地址(需开放端口): %s\n", webuiURLv2)

	sigCh := make(chan os.Signal, 1)
	signal.Notify(sigCh, syscall.SIGINT, syscall.SIGTERM)

	<-sigCh

	for _, client := range wsClients {
		err := client.Close()
		if err != nil {
			log.Printf("Error closing WebSocket connection: %v\n", err)
		}
	}

	if conf.Settings.MemoryMsgid {
		echo.StopCleanupRoutine()
	}

	url.CloseDB()
	idmap.CloseDB()

	for _, wsClient := range p.WsServerClients {
		if err := wsClient.Close(); err != nil {
			log.Printf("Error closing WebSocket server client: %v\n", err)
		}
	}

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if err := httpServer.Shutdown(ctx); err != nil {
		log.Fatal("Server forced to shutdown:", err)
	}
}

func applyDisallowedIntentPolicy(intent dto.Intent, enabledHandlers map[string]bool) dto.Intent {
	if !config.GetSuppressDisallowedIntents() {
		return intent
	}

	before := intent
	var suppressed []string

	groupMemberIntent := dto.EventToIntent(dto.EventGroupMemberAdd, dto.EventGroupMemberRemove)
	if enabledHandlers["GroupMemberAddEventHandler"] || enabledHandlers["GroupMemberRemoveEventHandler"] {
		if intent&groupMemberIntent != 0 {
			intent &^= groupMemberIntent
			suppressed = append(suppressed, fmt.Sprintf("GroupMemberAdd/Remove=%d", groupMemberIntent))
		}
	}

	guildMessageIntent := dto.EventToIntent(dto.EventMessageCreate, dto.EventMessageDelete)
	if enabledHandlers["CreateMessageHandler"] {
		if intent&guildMessageIntent != 0 {
			intent &^= guildMessageIntent
			suppressed = append(suppressed, fmt.Sprintf("CreateMessageHandler/IntentGuildMessages=%d", guildMessageIntent))
		}
	}

	groupMessageIntent := dto.EventToIntent(dto.EventGroupMessageCreate)
	if enabledHandlers["GroupMessageEventHandler"] && intent&groupMessageIntent != 0 {
		if hasAnyHandler(enabledHandlers,
			"GroupATMessageEventHandler",
			"C2CMessageEventHandler",
			"GroupAddRobotEventHandler",
			"GroupDelRobotEventHandler",
			"GroupMsgRejectHandler",
			"GroupMsgReceiveHandler",
			"FriendAddEventHandler",
			"FriendDelEventHandler",
			"C2CMsgRejectHandler",
			"C2CMsgReceiveHandler",
		) {
			log.Printf("suppress_disallowed_intents: GroupMessageEventHandler 仅注册本地处理器，复用现有 IntentGroupMessages=%d", groupMessageIntent)
		} else {
			intent &^= groupMessageIntent
			suppressed = append(suppressed, fmt.Sprintf("GroupMessageEventHandler/IntentGroupMessages=%d", groupMessageIntent))
		}
	}

	if len(suppressed) > 0 {
		log.Printf("suppress_disallowed_intents: 已从 Identify intents 屏蔽 %s，before=%d after=%d", strings.Join(suppressed, ", "), before, intent)
	}
	return intent
}

func hasAnyHandler(enabledHandlers map[string]bool, names ...string) bool {
	for _, name := range names {
		if enabledHandlers[name] {
			return true
		}
	}
	return false
}

func allEmpty(addresses []string) bool {
	for _, addr := range addresses {
		if addr != "" {
			return false
		}
	}
	return true
}

func setupConfigWatcher(configFilePath string) {
	watcher, err := fsnotify.NewWatcher()
	if err != nil {
		log.Fatalf("Error setting up watcher: %v", err)
	}

	go func() {
		for {
			select {
			case event, ok := <-watcher.Events:
				if !ok {
					return
				}
				if event.Op&fsnotify.Write == fsnotify.Write {
					fmt.Println("检测到配置文件变动:", event.Name)
					config.LoadConfig(configFilePath, true)
				}
			case err, ok := <-watcher.Errors:
				if !ok {
					return
				}
				log.Println("Watcher error:", err)
			}
		}
	}()

	err = watcher.Add(configFilePath)
	if err != nil {
		log.Fatalf("Error adding watcher: %v", err)
	}
}

var sensitiveHeaders = map[string]bool{
	"authorization":    true,
	"cookie":           true,
	"set-cookie":       true,
	"x-token":          true,
	"x-signature":      true,
	"x-signature-256":  true,
}

func UnionFanout(base gin.HandlerFunc) gin.HandlerFunc {
	return func(c *gin.Context) {
		body, err := io.ReadAll(c.Request.Body)
		if err != nil {
			c.AbortWithStatusJSON(http.StatusInternalServerError, gin.H{"error": "read body failed"})
			return
		}
		c.Request.Body = io.NopCloser(bytes.NewReader(body))

		if uw := config.GetUnionWebhook(); uw != "" && uw != "0" {
			method := c.Request.Method
			headers := c.Request.Header.Clone()

			for k := range headers {
				if sensitiveHeaders[strings.ToLower(k)] {
					delete(headers, k)
				}
			}

			go func(method, url string, headers http.Header, payload []byte) {
				defer func() { _ = recover() }()

				ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
				defer cancel()

				req, err := http.NewRequestWithContext(ctx, method, url, bytes.NewReader(payload))
				if err != nil {
					return
				}
				for k, vs := range headers {
					for _, v := range vs {
						req.Header.Add(k, v)
					}
				}
				_, _ = http.DefaultClient.Do(req)
			}(method, uw, headers, body)
		}

		base(c)
	}
}

func MetricsHandler(c *gin.Context) {
	c.Header("Content-Type", "text/plain; version=0.0.4; charset=utf-8")

	msgRecv := atomic.LoadUint64(&mylog.MetricMsgReceived)
	msgSent := atomic.LoadUint64(&mylog.MetricMsgSent)
	errCount := atomic.LoadUint64(&mylog.MetricErrorCount)
	slowEvents := atomic.LoadUint64(&mylog.MetricSlowEvents)

	uptime := time.Since(mylog.StartTime).Seconds()

	var m runtime.MemStats
	runtime.ReadMemStats(&m)
	memAlloc := m.Alloc

	output := fmt.Sprintf(
		"# HELP gensokyo_uptime_seconds Uptime of the bot in seconds.\n"+
			"# TYPE gensokyo_uptime_seconds gauge\n"+
			"gensokyo_uptime_seconds %.2f\n"+
			"# HELP gensokyo_messages_received_total Total number of received messages.\n"+
			"# TYPE gensokyo_messages_received_total counter\n"+
			"gensokyo_messages_received_total %d\n"+
			"# HELP gensokyo_messages_sent_total Total number of sent messages.\n"+
			"# TYPE gensokyo_messages_sent_total counter\n"+
			"gensokyo_messages_sent_total %d\n"+
			"# HELP gensokyo_errors_total Total number of log errors.\n"+
			"# TYPE gensokyo_errors_total counter\n"+
			"gensokyo_errors_total %d\n"+
			"# HELP gensokyo_slow_events_total Total number of slow processing events.\n"+
			"# TYPE gensokyo_slow_events_total counter\n"+
			"gensokyo_slow_events_total %d\n"+
			"# HELP gensokyo_memory_allocated_bytes Memory currently allocated in bytes.\n"+
			"# TYPE gensokyo_memory_allocated_bytes gauge\n"+
			"gensokyo_memory_allocated_bytes %d\n",
		uptime, msgRecv, msgSent, errCount, slowEvents, memAlloc,
	)
	c.String(http.StatusOK, output)
}

func HealthzHandler(c *gin.Context) {
	var m runtime.MemStats
	runtime.ReadMemStats(&m)
	c.JSON(http.StatusOK, gin.H{
		"status":     "ok",
		"uptime":     time.Since(mylog.StartTime).Seconds(),
		"goroutines": runtime.NumGoroutine(),
		"memory_mb":  float64(m.Alloc) / 1024 / 1024,
	})
}
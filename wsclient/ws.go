package wsclient

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"log"
	"net/http"
	"net/url"
	"sync"
	"sync/atomic"
	"time"

	"github.com/gorilla/websocket"
	"github.com/hoshinonyaruko/gensokyo/botstats"
	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/openapi"
)

// errWebSocketClosed 表示客户端已关闭，写请求不会再有消费者
var errWebSocketClosed = errors.New("websocket client closed")

type WebSocketClient struct {
	conn           *websocket.Conn
	api            openapi.OpenAPI
	apiv2          openapi.OpenAPI
	botID          uint64
	urlStr         string
	cancel         context.CancelFunc
	isReconnecting atomic.Bool              // 重连门闩（跨 goroutine 读写）
	sendFailures   []map[string]interface{} // 存储失败的消息
	writeCh        chan writeRequest        // 写请求通道
	closeCh        chan struct{}            // 用于关闭的通道
	closed         atomic.Bool              // Close 之后为 true
	closeOnce      sync.Once                // 保证 Close 幂等
}

type writeRequest struct {
	messageType int
	data        []byte
}

// SendMessage 发送消息，将写请求发送到写 Goroutine
func (client *WebSocketClient) SendMessage(message map[string]interface{}) error {
	// 关闭后写 Goroutine 已退出，writeCh 仍有缓冲空间，必须先短路
	// 否则消息会被静默丢弃（select 两个 case 均就绪时随机选择）
	if client.closed.Load() {
		return errWebSocketClosed
	}

	// 序列化消息
	msgBytes, err := json.Marshal(message)
	if err != nil {
		log.Println("Error marshalling message:", err)
		return err
	}

	// 入队即交给写 Goroutine；关闭瞬间可能已入队但无人消费，入队后复查避免"假成功"
	select {
	case client.writeCh <- writeRequest{messageType: websocket.TextMessage, data: msgBytes}:
		if client.closed.Load() {
			return errWebSocketClosed
		}
		return nil
	case <-client.closeCh:
		return errWebSocketClosed
	}
}

// Close 关闭 WebSocketClient，停止写 Goroutine（可重复调用）
func (client *WebSocketClient) Close() error {
	client.closeOnce.Do(func() {
		client.closed.Store(true)
		// writeCh 不关闭：写 Goroutine 只靠 closeCh 退出，避免并发 SendMessage panic
		close(client.closeCh)
		if client.conn != nil {
			if err := client.conn.Close(); err != nil {
				log.Println("Error closing WebSocket connection:", err)
			}
		}
	})
	return nil
}

// startWriter 专用的写 Goroutine
func (client *WebSocketClient) startWriter() {
	for {
		select {
		case req := <-client.writeCh:
			// 执行写操作
			err := client.conn.WriteMessage(req.messageType, req.data)
			if err != nil {
				log.Println("Error sending message:", err)
				if !config.GetDisableErrorChan() {
					client.sendFailures = append(client.sendFailures, map[string]interface{}{"message": req.data}) // 记录失败的消息
				}
			}
		case <-client.closeCh:
			return
		}
	}
}

// 处理onebotv11应用端发来的信息
func (client *WebSocketClient) handleIncomingMessages(cancel context.CancelFunc) {
	for {
		_, msg, err := client.conn.ReadMessage()
		if err != nil {
			mylog.Println("WebSocket connection closed:", err)
			cancel() // 取消心跳 goroutine
			if !client.isReconnecting.Load() {
				go client.Reconnect()
			}
			return // 退出循环，不再尝试读取消息
		}

		go client.recvMessage(msg)
	}
}

// 断线重连
func (client *WebSocketClient) Reconnect() {
	// 已关闭则不再重连（Close 后 handleIncomingMessages 仍可能触发一次重连）
	if client.closed.Load() {
		return
	}

	client.isReconnecting.Store(true)

	addresses := config.GetWsAddress()
	tokens := config.GetWsToken()

	var token string
	for index, address := range addresses {
		if address == client.urlStr && index < len(tokens) {
			token = tokens[index]
			break
		}
	}

	// 检查URL中是否有access_token参数
	mp := getParamsFromURI(client.urlStr)
	if val, ok := mp["access_token"]; ok {
		token = val
	}

	headers := http.Header{
		"User-Agent":    []string{"CQHttp/4.15.0"},
		"X-Client-Role": []string{"Universal"},
		"X-Self-ID":     []string{fmt.Sprintf("%d", client.botID)},
	}

	if token != "" {
		headers["Authorization"] = []string{"Token " + token}
	}
	mylog.Printf("准备使用token[%s]重新连接到[%s]\n", redactToken(token), client.urlStr)
	dialer := websocket.Dialer{
		Proxy:            http.ProxyFromEnvironment,
		HandshakeTimeout: 45 * time.Second,
	}

	var conn *websocket.Conn
	var err error

	maxRetryAttempts := config.GetReconnecTimes()
	retryCount := 0
	for {
		mylog.Println("Dialing URL:", client.urlStr)
		conn, _, err = dialer.Dial(client.urlStr, headers)
		if err != nil {
			retryCount++
			if retryCount > maxRetryAttempts {
				mylog.Printf("Exceeded maximum retry attempts for WebSocket[%v]: %v\n", client.urlStr, err)
				return
			}
			mylog.Printf("Failed to connect to WebSocket[%v]: %v, retrying in 5 seconds...\n", client.urlStr, err)
			time.Sleep(5 * time.Second) // sleep for 5 seconds before retrying
		} else {
			mylog.Printf("Successfully connected to %s.\n", client.urlStr) // 输出连接成功提示
			break                                                          // successfully connected, break the loop
		}
	}
	// dial 期间可能已被 Close：关闭新连接并直接退出，避免关闭后继续发送元事件
	if client.closed.Load() {
		if err := conn.Close(); err != nil {
			mylog.Printf("Error closing reconnected WebSocket: %v\n", err)
		}
		return
	}

	// 复用现有的client完成重连
	client.conn = conn

	// 再次发送元事件
	message := map[string]interface{}{
		"meta_event_type": "lifecycle",
		"post_type":       "meta_event",
		"self_id":         client.botID,
		"sub_type":        "connect",
		"time":            int(time.Now().Unix()),
	}

	mylog.Printf("Message: %+v\n", message)

	err = client.SendMessage(message)
	if err != nil {
		// handle error
		mylog.Printf("Error sending message: %v\n", err)
	}

	//退出老的sendHeartbeat和handleIncomingMessages
	client.cancel()

	// Starting goroutine for heartbeats and another for listening to messages
	ctx, cancel := context.WithCancel(context.Background())

	client.cancel = cancel
	heartbeatinterval := config.GetHeartBeatInterval()
	go client.sendHeartbeat(ctx, client.botID, heartbeatinterval)
	go client.handleIncomingMessages(cancel)

	defer func() {
		client.isReconnecting.Store(false)
	}()

	mylog.Printf("Successfully reconnected to WebSocket.")

}

// 处理发送失败的消息
func (client *WebSocketClient) processFailedMessages() {
	for _, failedMessage := range client.sendFailures {
		// 尝试重新发送消息
		err := client.SendMessage(failedMessage)
		if err != nil {
			mylog.Printf("Error resending message: %v\n", err)
		}
	}
	// 清空失败消息列表
	client.sendFailures = []map[string]interface{}{}
}

// 处理信息,调用腾讯api
func (client *WebSocketClient) recvMessage(msg []byte) {
	var message callapi.ActionMessage
	//mylog.Println("Received from onebotv11 server raw:", string(msg))
	err := json.Unmarshal(msg, &message)
	if err != nil {
		mylog.Printf("Error unmarshalling message: %v, Original message: %s", err, string(msg))
		return
	}
	mylog.Println("Received from onebotv11 server:", TruncateMessage(message, 800))
	// 调用callapi
	go callapi.CallAPIFromDict(client, client.api, client.apiv2, message)
}

// 截断信息
func TruncateMessage(message callapi.ActionMessage, maxLength int) string {
	paramsStr, err := json.Marshal(message.Params)
	if err != nil {
		return "Error marshalling Params for truncation."
	}

	// Truncate Params if its length exceeds maxLength
	truncatedParams := string(paramsStr)
	if len(truncatedParams) > maxLength {
		truncatedParams = truncatedParams[:maxLength] + "..."
	}

	return fmt.Sprintf("Action: %s, Params: %s, Echo: %v", message.Action, truncatedParams, message.Echo)
}

// 发送心跳包
func (client *WebSocketClient) sendHeartbeat(ctx context.Context, botID uint64, heartbeatinterval int) {
	for {
		select {
		case <-ctx.Done():
			return
		case <-time.After(time.Duration(heartbeatinterval) * time.Second):
			messageReceived, messageSent, lastMessageTime, err := botstats.GetStats()
			if err != nil {
				mylog.Printf("心跳错误,获取机器人发信状态错误:%v", err)
			}
			message := map[string]interface{}{
				"post_type":       "meta_event",
				"meta_event_type": "heartbeat",
				"time":            int(time.Now().Unix()),
				"self_id":         botID,
				"status": map[string]interface{}{
					"app_enabled":     true,
					"app_good":        true,
					"app_initialized": true,
					"good":            true,
					"online":          true,
					"plugins_good":    nil,
					"stat": map[string]int{
						"packet_received":   34933,
						"packet_sent":       8513,
						"packet_lost":       0,
						"message_received":  messageReceived,
						"message_sent":      messageSent,
						"disconnect_times":  0,
						"lost_times":        0,
						"last_message_time": int(lastMessageTime),
					},
				},
				"interval": 5000, // 以毫秒为单位
			}
			client.SendMessage(message)
			// 重发失败的消息
			client.processFailedMessages()
		}
	}
}

// NewWebSocketClient 创建 WebSocketClient 实例，接受 WebSocket URL、botID 和 openapi.OpenAPI 实例
func NewWebSocketClient(urlStr string, botID uint64, api openapi.OpenAPI, apiv2 openapi.OpenAPI, maxRetryAttempts int) (*WebSocketClient, error) {
	addresses := config.GetWsAddress()
	tokens := config.GetWsToken()

	var token string
	for index, address := range addresses {
		if address == urlStr && index < len(tokens) {
			token = tokens[index]
			break
		}
	}

	// 检查URL中是否有access_token参数
	mp := getParamsFromURI(urlStr)
	if val, ok := mp["access_token"]; ok {
		token = val
	}

	headers := http.Header{
		"User-Agent":    []string{"CQHttp/4.15.0"},
		"X-Client-Role": []string{"Universal"},
		"X-Self-ID":     []string{fmt.Sprintf("%d", botID)},
	}

	if token != "" {
		headers["Authorization"] = []string{"Token " + token}
	}
	mylog.Printf("准备使用token[%s]连接到[%s]\n", redactToken(token), urlStr)
	dialer := websocket.Dialer{
		Proxy:            http.ProxyFromEnvironment,
		HandshakeTimeout: 45 * time.Second,
	}

	var conn *websocket.Conn
	var err error

	retryCount := 0
	for {
		mylog.Println("Dialing URL:", urlStr)
		conn, _, err = dialer.Dial(urlStr, headers)
		if err != nil {
			retryCount++
			if retryCount > maxRetryAttempts {
				mylog.Printf("Exceeded maximum retry attempts for WebSocket[%v]: %v\n", urlStr, err)
				return nil, err
			}
			mylog.Printf("Failed to connect to WebSocket[%v]: %v, retrying in 5 seconds...\n", urlStr, err)
			time.Sleep(5 * time.Second) // sleep for 5 seconds before retrying
		} else {
			mylog.Printf("Successfully connected to %s.\n", urlStr) // 输出连接成功提示
			break                                                   // successfully connected, break the loop
		}
	}
	client := &WebSocketClient{
		conn:         conn,
		api:          api,
		apiv2:        apiv2,
		botID:        botID,
		urlStr:       urlStr,
		sendFailures: []map[string]interface{}{},
		writeCh:      make(chan writeRequest, 5000), // 缓冲区大小可以根据需求调整
		closeCh:      make(chan struct{}),
	}
	go client.startWriter() // 启动写 Goroutine

	// Sending initial message similar to your setupB function
	message := map[string]interface{}{
		"meta_event_type": "lifecycle",
		"post_type":       "meta_event",
		"self_id":         botID,
		"sub_type":        "connect",
		"time":            int(time.Now().Unix()),
	}

	mylog.Printf("Message: %+v\n", message)

	err = client.SendMessage(message)
	if err != nil {
		// handle error
		mylog.Printf("Error sending message: %v\n", err)
	}

	// Starting goroutine for heartbeats and another for listening to messages
	ctx, cancel := context.WithCancel(context.Background())

	client.cancel = cancel
	heartbeatinterval := config.GetHeartBeatInterval()
	go client.sendHeartbeat(ctx, botID, heartbeatinterval)
	go client.handleIncomingMessages(cancel)

	return client, nil
}

// getParamsFromURI 解析给定URI中的查询参数，并返回一个映射（map）
func getParamsFromURI(uriStr string) map[string]string {
	params := make(map[string]string)

	u, err := url.Parse(uriStr)
	if err != nil {
		mylog.Printf("Error parsing the URL: %v\n", err)
		return params
	}

	// 遍历查询参数并将其添加到返回的映射中
	for key, values := range u.Query() {
		if len(values) > 0 {
			params[key] = values[0] // 如果一个参数有多个值，这里只选择第一个。可以根据需求进行调整。
		}
	}

	return params
}

// redactToken 对令牌进行脱敏，只显示前 4 位
func redactToken(s string) string {
	if len(s) <= 4 {
		return "****"
	}
	return s[:4] + "****"
}

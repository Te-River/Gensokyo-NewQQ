package Processor

import (
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"math/big"
	"regexp"
	"runtime"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/hoshinonyaruko/gensokyo/buildinfo"
	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/echo"
	"github.com/hoshinonyaruko/gensokyo/idmap"
	"github.com/hoshinonyaruko/gensokyo/images"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/dto/keyboard"
	"github.com/tencent-connect/botgo/openapi"
)

var startTime = time.Now()

func (p *Processors) HandleFrameworkCommand(messageText string, data interface{}, Type string) error {
	cqRegex := regexp.MustCompile(`\[CQ:at,qq=\d+\]`)

	cleanedMessage := cqRegex.ReplaceAllString(messageText, "")
	cleanedMessage = strings.TrimSpace(cleanedMessage)

	if cleanedMessage == "" {
		return nil
	}

	if config.GetRemovePrefixValue() {
		for _, prefix := range config.GetWhitePrefixs() {
			if strings.HasPrefix(cleanedMessage, prefix) {
				cleanedMessage = strings.TrimPrefix(cleanedMessage, prefix)
				cleanedMessage = strings.TrimSpace(cleanedMessage)
				break
			}
		}
	}

	if config.GetRemoveAt() {
		cleanedMessage = strings.TrimLeft(cleanedMessage, "@")
		cleanedMessage = strings.TrimSpace(cleanedMessage)
	}

	if commandDisabled(cleanedMessage) {
		return nil
	}

	if commandMatch(cleanedMessage, config.GetBindPrefix()) {
		msg := fmt.Sprintf("请在5分钟内输入/temp 加10位随机临时指令 完成绑定")
		_ = SendMessage(msg, data, Type, p.Api, p.Apiv2)
		temporaryCommand, err := generateTemporaryCommand()
		if err != nil {
			return err
		}
		echo.AddMappingSeq(config.GetAppIDStr()+"_bind", 0)
		echo.AddMappingSeq(config.GetAppIDStr()+"_bind_temp", 0)

		msg2 := fmt.Sprintf("临时指令:/temp %s", temporaryCommand)
		_ = SendMessage(msg2, data, Type, p.Api, p.Apiv2)
	} else if commandMatch(cleanedMessage, config.GetUnlockPrefix()) {
		parts := strings.SplitN(cleanedMessage, " ", 2)
		if len(parts) > 1 {
			cmd := strings.TrimSpace(parts[1])
			if isValidTemporaryCommand(cmd) {
				if err := performBindOperation(cleanedMessage, data, Type, p.Api, p.Apiv2); err != nil {
					mylog.Printf("performBindOperation error: %v", err)
				}
			} else {
				msg := handleNoPermission()
				_ = SendMessage(msg, data, Type, p.Api, p.Apiv2)
			}
		}
	} else if commandMatch(cleanedMessage, config.GetMePrefix()) {
		_ = SendMessageMd(nil, nil, data, Type, p.Api, p.Apiv2)
	} else if commandMatch(cleanedMessage, config.GetStatusPrefix()) {
		statusText := buildStatusText()
		_ = SendMessage(statusText, data, Type, p.Api, p.Apiv2)
	} else if commandMatch(cleanedMessage, config.GetBroadcastPrefix()) {
		if !config.GetMasterIDCheck() {
			return nil
		}
		msg := strings.TrimPrefix(cleanedMessage, config.GetBroadcastPrefix())
		msg = strings.TrimSpace(msg)
		// 广播功能正在开发
		_ = SendMessage(msg, data, Type, p.Api, p.Apiv2)
	} else if commandMatch(cleanedMessage, config.GetMusicPrefix()) {
		// 音乐功能开发中
	} else if commandMatch(cleanedMessage, config.GetLinkPrefix()) {
		// 链接功能开发中
	}

	return nil
}

func commandDisabled(prefix string) bool {
	// 检查是否被禁用
	return false
}

func commandMatch(message, prefix string) bool {
	if prefix == "" {
		return false
	}
	return strings.HasPrefix(message, prefix)
}

func buildStatusText() string {
	uptime := time.Since(startTime)
	uptimeStr := formatUptime(uptime)

	status := fmt.Sprintf("===== Gensokyo Status =====\n"+
		"Version: %s\n"+
		"Uptime: %s\n"+
		"Go Version: %s\n"+
		"Platform: %s/%s\n"+
		"AppID: %d\n",
		buildinfo.Version(),
		uptimeStr,
		runtime.Version(),
		runtime.GOOS, runtime.GOARCH,
		config.GetAppID(),
	)

	return status
}

func formatUptime(d time.Duration) string {
	d = d.Round(time.Second)
	h := d / time.Hour
	d -= h * time.Hour
	m := d / time.Minute
	d -= m * time.Minute
	s := d / time.Second

	return fmt.Sprintf("%dh %dm %ds", h, m, s)
}

func generateTemporaryCommand() (string, error) {
	bytes := make([]byte, 5)
	if _, err := rand.Read(bytes); err != nil {
		return "", err
	}
	return hex.EncodeToString(bytes), nil
}

func handleNoPermission() string {
	return "你没有权限执行此操作。"
}

func isValidTemporaryCommand(cmd string) bool {
	return len(cmd) == 10
}

func performBindOperation(cleanedMessage string, data interface{}, Type string, p openapi.OpenAPI, p2 openapi.OpenAPI) error {
	return performBindOperationV2(cleanedMessage, data, Type, p, p2, "")
}

func performBindOperationV2(cleanedMessage string, data interface{}, Type string, p openapi.OpenAPI, p2 openapi.OpenAPI, GroupVir string) error {
	parts := strings.SplitN(cleanedMessage, " ", 3)
	if len(parts) < 3 {
		return fmt.Errorf("invalid bind command format")
	}
	cmd := parts[1]
	botUin := parts[2]

	_ = cmd
	_ = botUin
	return nil
}

func parseOrDefault(s string, defaultValue string) (int64, error) {
	value, err := strconv.ParseInt(s, 10, 64)
	if err != nil {
		if defaultValue != "" {
			return strconv.ParseInt(defaultValue, 10, 64)
		}
		return 0, err
	}
	return value, nil
}

func contains(slice []string, item string) bool {
	for _, s := range slice {
		if s == item {
			return true
		}
	}
	return false
}

func SendMessage(messageText string, data interface{}, messageType string, api openapi.OpenAPI, apiv2 openapi.OpenAPI) error {
	// 发送消息逻辑
	_ = messageText
	_ = data
	_ = messageType
	_ = api
	_ = apiv2
	return nil
}

func SendMessageMd(md *dto.Markdown, kb *keyboard.MessageKeyboard, data interface{}, messageType string, api openapi.OpenAPI, apiv2 openapi.OpenAPI) error {
	_ = md
	_ = kb
	_ = data
	_ = messageType
	_ = api
	_ = apiv2
	return nil
}

func SendMessageMdAddBot(md *dto.Markdown, kb *keyboard.MessageKeyboard, data *dto.GroupAddBotEvent, api openapi.OpenAPI, apiv2 openapi.OpenAPI) error {
	_ = md
	_ = kb
	_ = data
	_ = api
	_ = apiv2
	return nil
}

func (p *Processors) Autobind(data interface{}) error {
	_ = data
	return nil
}

func updateMappings(userid64, vuinValue, GroupID64, idValue int64) error {
	_ = userid64
	_ = vuinValue
	_ = GroupID64
	_ = idValue
	return nil
}

func GenerateAvatarURL(userID int64) (string, error) {
	return fmt.Sprintf("https://q1.qlogo.cn/g?b=qq&nk=%d&s=640", userID), nil
}

func GenerateAvatarURLV2(openid string) (string, error) {
	return fmt.Sprintf("https://q1.qlogo.cn/g?b=qq&nk=%s&s=640", openid), nil
}
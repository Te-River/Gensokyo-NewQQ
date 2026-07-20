package config

import (
	"fmt"
	"strings"

	"github.com/hoshinonyaruko/gensokyo/structs"
	"github.com/hoshinonyaruko/gensokyo/sys"
)

// 获取ws地址数组
func GetWsAddress() []string {
	mu.RLock()
	defer mu.RUnlock()
	if instance != nil {
		return instance.Settings.WsAddress
	}
	return nil // 返回nil，如果instance为nil
}

// 获取gensokyo服务的地址
func GetServer_dir() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get upload directory.")
		return ""
	}
	return instance.Settings.Server_dir
}

// 获取DevBotid
func GetDevBotid() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get DevBotid.")
		return "1234"
	}
	return instance.Settings.DevBotid
}

// 获取GetForwardMsgLimit
func GetForwardMsgLimit() int {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get GetForwardMsgLimit.")
		return 3
	}
	return instance.Settings.ForwardMsgLimit
}

// 获取Develop_Acdir服务的地址
func GetDevelop_Acdir() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get DevlopAcDir.")
		return ""
	}
	return instance.Settings.DevlopAcDir
}

// 获取lotus的值
func GetLotusValue() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get lotus value.")
		return false
	}
	return instance.Settings.Lotus
}

// 获取双向ehco
func GetTwoWayEcho() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get lotus value.")
		return false
	}
	return instance.Settings.TwoWayEcho
}

// 获取白名单开启状态
func GetWhitePrefixMode() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to GetWhitePrefixModes value.")
		return false
	}
	return instance.Settings.WhitePrefixMode
}

// 获取白名单指令数组
func GetWhitePrefixs() []string {
	mu.RLock()
	defer mu.RUnlock()
	if instance != nil {
		return instance.Settings.WhitePrefixs
	}
	return nil // 返回nil，如果instance为nil
}

// 获取黑名单开启状态
func GetBlackPrefixMode() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to GetBlackPrefixMode value.")
		return false
	}
	return instance.Settings.BlackPrefixMode
}

// 获取黑名单指令数组
func GetBlackPrefixs() []string {
	mu.RLock()
	defer mu.RUnlock()
	if instance != nil {
		return instance.Settings.BlackPrefixs
	}
	return nil // 返回nil，如果instance为nil
}

// 获取IPurl显示开启状态
func GetVisibleIP() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to GetVisibleIP value.")
		return false
	}
	return instance.Settings.VisibleIp
}

// 修改 GetVisualkPrefixs 函数以返回新类型
func GetVisualkPrefixs() []structs.VisualPrefixConfig {
	mu.RLock()
	defer mu.RUnlock()
	if instance != nil {
		var varvisualPrefixes []structs.VisualPrefixConfig
		for _, vp := range instance.Settings.VisualPrefixs {
			varvisualPrefixes = append(varvisualPrefixes, structs.VisualPrefixConfig{
				Prefix:          vp.Prefix,
				WhiteList:       vp.WhiteList,
				NoWhiteResponse: vp.NoWhiteResponse,
			})
		}
		return varvisualPrefixes
	}
	return nil // 返回nil，如果instance为nil
}

// 获取LazyMessageId状态
func GetLazyMessageId() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get LazyMessageId value.")
		return false
	}
	return instance.Settings.LazyMessageId
}

// 获取HashID
func GetHashIDValue() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get hashid value.")
		return false
	}
	return instance.Settings.HashID
}

// 获取RemoveAt的值
func GetRemoveAt() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get RemoveAt value.")
		return false
	}
	return instance.Settings.RemoveAt
}

// 获取ConvertOtherAt的值
func GetConvertOtherAt() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get ConvertOtherAt value.")
		return false
	}
	return instance.Settings.ConvertOtherAt
}

// 获取port的值
func GetPortValue() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get port value.")
		return ""
	}
	return instance.Settings.Port
}

// 获取Array的值
func GetArrayValue() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get array value.")
		return false
	}
	return instance.Settings.Array
}

// 获取AppID
func GetAppID() uint64 {
	mu.RLock()
	defer mu.RUnlock()
	if instance != nil {
		return instance.Settings.AppID
	}
	return 0 // or whatever default value you'd like to return if instance is nil
}

// 获取AppID String
func GetAppIDStr() string {
	mu.RLock()
	defer mu.RUnlock()
	if instance != nil {
		return fmt.Sprintf("%d", instance.Settings.AppID)
	}
	return "0"
}

// 获取WsToken
func GetWsToken() []string {
	mu.RLock()
	defer mu.RUnlock()
	if instance != nil {
		return instance.Settings.WsToken
	}
	return nil // 返回nil，如果instance为nil
}

// 获取MasterID数组
func GetMasterID() []string {
	mu.RLock()
	defer mu.RUnlock()
	if instance != nil {
		return instance.Settings.MasterID
	}
	return nil // 返回nil，如果instance为nil
}

// 获取port的值
func GetEnableWsServer() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get port value.")
		return false
	}
	return instance.Settings.EnableWsServer
}

// 获取WsServerToken的值
func GetWsServerToken() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get WsServerToken value.")
		return ""
	}
	return instance.Settings.WsServerToken
}

// 获取identify_file的值
func GetIdentifyFile() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get identify file name.")
		return false
	}
	return instance.Settings.IdentifyFile
}

// 获取crt路径
func GetCrtPath() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get crt path.")
		return ""
	}
	return instance.Settings.Crt
}

// 获取key路径
func GetKeyPath() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get key path.")
		return ""
	}
	return instance.Settings.Key
}

// 开发者日志
func GetDeveloperLog() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get developer log status.")
		return false
	}
	return instance.Settings.DeveloperLog
}

// ComposeWebUIURL 组合webui的完整访问地址
// 参数 useBackupPort 控制是否使用备用端口
func ComposeWebUIURL(useBackupPort bool) string {
	serverDir := GetServer_dir()

	var port string
	if useBackupPort {
		port = GetBackupPort()
	} else {
		port = GetPortValue()
	}

	// 判断端口是不是443，如果是，则使用https协议
	protocol := "http"
	if port == "443" {
		protocol = "https"
	}

	// 组合出完整的URL
	return fmt.Sprintf("%s://%s:%s/webui", protocol, serverDir, port)
}

// ComposeWebUIURLv2 组合webui的完整访问地址
// 参数 useBackupPort 控制是否使用备用端口
func ComposeWebUIURLv2(useBackupPort bool) string {
	ip, _ := sys.GetPublicIP()

	var port string
	if useBackupPort {
		port = GetBackupPort()
	} else {
		port = GetPortValue()
	}

	// 判断端口是不是443，如果是，则使用https协议
	protocol := "http"
	if port == "443" {
		protocol = "https"
	}

	// 组合出完整的URL
	return fmt.Sprintf("%s://%s:%s/webui", protocol, ip, port)
}

// GetServerUserName 获取服务器用户名
func GetServerUserName() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get server user name.")
		return ""
	}
	return instance.Settings.Username
}

// GetServerUserPassword 获取服务器用户密码
func GetServerUserPassword() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get server user password.")
		return ""
	}
	return instance.Settings.Password
}

// GetImageLimit 返回 ImageLimit 的值
func GetImageLimit() int {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get image limit value.")
		return 0 // 或者返回一个默认的 ImageLimit 值
	}

	return instance.Settings.ImageLimit
}

// GetRemovePrefixValue 函数用于获取 remove_prefix 的配置值
func GetRemovePrefixValue() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get remove_prefix value.")
		return false // 或者可能是默认值，取决于您的应用程序逻辑
	}
	return instance.Settings.RemovePrefix
}

// GetLotusPort retrieves the LotusPort setting from your singleton instance.
func GetBackupPort() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get LotusPort.")
		return ""
	}

	return instance.Settings.BackupPort
}

// 获取GetDevMsgID的值
func GetDevMsgID() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to GetDevMsgID value.")
		return false
	}
	return instance.Settings.DevMessgeID
}

// 获取GetSaveLogs的值
func GetSaveLogs() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to GetSaveLogs value.")
		return false
	}
	return instance.Settings.SaveLogs
}

// 获取GetSaveLogs的值
func GetLogLevel() int {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to GetLogLevel value.")
		return 2
	}
	return instance.Settings.LogLevel
}

// 获取GetBindPrefix的值
func GetBindPrefix() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to GetBindPrefix value.")
		return "/bind"
	}
	return instance.Settings.BindPrefix
}

// 获取GetMePrefix的值
func GetMePrefix() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to GetMePrefix value.")
		return "/me"
	}
	return instance.Settings.MePrefix
}

// GetStatusPrefix 获取状态指令前缀。
func GetStatusPrefix() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to GetStatusPrefix value.")
		return "/gskstatus"
	}
	return instance.Settings.StatusPrefix
}

// GetBroadcastPrefix 获取广播指令前缀。
func GetBroadcastPrefix() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to GetBroadcastPrefix value.")
		return "/gskbroadcast"
	}
	return instance.Settings.BroadcastPrefix
}

// 获取FrpPort的值
func GetFrpPort() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to GetFrpPort value.")
		return "0"
	}
	return instance.Settings.FrpPort
}

// 获取GetRemoveBotAtGroup的值
func GetRemoveBotAtGroup() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to GetRemoveBotAtGroup value.")
		return false
	}
	return instance.Settings.RemoveBotAtGroup
}

// 获取ImageLimitB的值
func GetImageLimitB() int {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to ImageLimitB value.")
		return 100
	}
	return instance.Settings.ImageLimitB
}

// GetRecordSampleRate 返回 RecordSampleRate的值
func GetRecordSampleRate() int {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to GetRecordSampleRate value.")
		return 0 // 或者返回一个默认的 ImageLimit 值
	}

	return instance.Settings.RecordSampleRate
}

// GetRecordBitRate 返回 RecordBitRate
func GetRecordBitRate() int {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to GetRecordBitRate value.")
		return 0 // 或者返回一个默认的 ImageLimit 值
	}

	return instance.Settings.RecordBitRate
}

// 获取NoWhiteResponse的值
func GetNoWhiteResponse() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to NoWhiteResponse value.")
		return ""
	}
	return instance.Settings.NoWhiteResponse
}

// 获取GetSendError的值
func GetSendError() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to GetSendError value.")
		return true
	}
	return instance.Settings.SendError
}

// 获取GetSaveError的值
func GetSaveError() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to GetSaveError value.")
		return true
	}
	return instance.Settings.SaveError
}

// 获取GetAddAtGroup的值
func GetAddAtGroup() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to GetAddGroupAt value.")
		return true
	}
	return instance.Settings.AddAtGroup
}

// 获取GetUrlPicTransfer的值
func GetUrlPicTransfer() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to GetUrlPicTransfer value.")
		return true
	}
	return instance.Settings.UrlPicTransfer
}

// 获取GetLotusPassword的值
func GetLotusPassword() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to GetLotusPassword value.")
		return ""
	}
	return instance.Settings.LotusPassword
}

// 获取GetWsServerPath的值
func GetWsServerPath() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to GetWsServerPath value.")
		return ""
	}
	return instance.Settings.WsServerPath
}

// GetIdmapPro 已废弃。MultiMap idmap 始终启用，旧 idmap_pro 分支保持关闭。
func GetIdmapPro() bool {
	return false
}

func GetOpUserIDType() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to OpUserIDType value.")
		return "vuin"
	}
	value := strings.ToLower(strings.TrimSpace(instance.Settings.OpUserIDType))
	switch value {
	case "raw", "ruin", "vuin":
		return value
	default:
		return "vuin"
	}
}

func GetMsgIDTTLSeconds() int {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to MsgIDTTLSeconds value.")
		return 3600
	}
	if instance.Settings.MsgIDTTLSeconds <= 0 {
		return 3600
	}
	return instance.Settings.MsgIDTTLSeconds
}

// 获取GetCardAndNick的值
func GetCardAndNick() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to GetCardAndNick value.")
		return ""
	}
	return instance.Settings.CardAndNick
}

// 获取GetAutoBind的值
func GetAutoBind() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to GetAutoBind value.")
		return false
	}
	return instance.Settings.AutoBind
}

// 获取GetCustomBotName的值
func GetCustomBotName() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to GetCustomBotName value.")
		return "Gensokyo全域机器人"
	}
	return instance.Settings.CustomBotName
}

// 获取send_delay的值
func GetSendDelay() int {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to GetSendDelay value.")
		return 300
	}
	return instance.Settings.SendDelay
}

// 获取GetAtoPCount的值
func GetAtoPCount() int {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to AtoPCount value.")
		return 5
	}
	return instance.Settings.AtoPCount
}

// 获取GetReconnecTimes的值
func GetReconnecTimes() int {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to ReconnecTimes value.")
		return 50
	}
	return instance.Settings.ReconnecTimes
}

// 获取GetHeartBeatInterval的值
func GetHeartBeatInterval() int {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to HeartBeatInterval value.")
		return 5
	}
	return instance.Settings.HeartBeatInterval
}

// 获取LaunchReconectTimes
func GetLaunchReconectTimes() int {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to LaunchReconectTimes value.")
		return 3
	}
	return instance.Settings.LaunchReconectTimes
}

// 获取GetUnlockPrefix
func GetUnlockPrefix() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to UnlockPrefix value.")
		return "/unlock"
	}
	return instance.Settings.UnlockPrefix
}

// 获取白名单例外群数组
func GetWhiteBypass() []int64 {
	mu.RLock()
	defer mu.RUnlock()
	if instance != nil {
		return instance.Settings.WhiteBypass
	}
	return nil // 返回nil，如果instance为nil
}

// 获取GetTransferUrl的值
func GetTransferUrl() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to GetTransferUrl value.")
		return false
	}
	return instance.Settings.TransferUrl
}

// 获取 HTTP 地址
func GetHttpAddress() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get HTTP address.")
		return ""
	}
	return instance.Settings.HttpAddress
}

// 获取 HTTP 访问令牌
func GetHTTPAccessToken() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get HTTP access token.")
		return ""
	}
	return instance.Settings.AccessToken
}

// 获取 HTTP 版本
func GetHttpVersion() int {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get HTTP version.")
		return 11
	}
	return instance.Settings.HttpVersion
}

// 获取 HTTP 超时时间
func GetHttpTimeOut() int {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get HTTP timeout.")
		return 5
	}
	return instance.Settings.HttpTimeOut
}

// 获取 POST URL 数组
func GetPostUrl() []string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get POST URL.")
		return nil
	}
	return instance.Settings.PostUrl
}

// 获取 POST 密钥数组
func GetPostSecret() []string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get POST secret.")
		return nil
	}
	return instance.Settings.PostSecret
}

// 获取 VisualPrefixsBypass
func GetVisualPrefixsBypass() []string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to getVisualPrefixsBypass.")
		return nil
	}
	return instance.Settings.VisualPrefixsBypass
}

// 获取 POST 最大重试次数数组
func GetPostMaxRetries() []int {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get POST max retries.")
		return nil
	}
	return instance.Settings.PostMaxRetries
}

// 获取 POST 重试间隔数组
func GetPostRetriesInterval() []int {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get POST retries interval.")
		return nil
	}
	return instance.Settings.PostRetriesInterval
}

// 获取GetTransferUrl的值
func GetNativeOb11() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to NativeOb11 value.")
		return false
	}
	return instance.Settings.NativeOb11
}

// 获取GetRamDomSeq的值
func GetRamDomSeq() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to GetRamDomSeq value.")
		return false
	}
	return instance.Settings.RamDomSeq
}

// 获取GetUrlToQrimage的值
func GetUrlToQrimage() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to GetUrlToQrimage value.")
		return false
	}
	return instance.Settings.UrlToQrimage
}

func GetUseUin() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to UseUin value.")
		return false
	}
	return instance.Settings.UseUin
}

func GetIdmapIsolation() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to IdmapIsolation value.")
		return false
	}
	return instance.Settings.IdmapIsolation
}

func GetIdmapLegacyCompat() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to IdmapLegacyCompat value.")
		return false
	}
	return instance.Settings.IdmapLegacyCompat
}

// 获取GetQrSize的值
func GetQrSize() int {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to QrSize value.")
		return 200
	}
	return instance.Settings.QrSize
}

// 获取GetWhiteBypassRevers的值
func GetWhiteBypassRevers() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to GetWhiteBypassRevers value.")
		return false
	}
	return instance.Settings.WhiteBypassRevers
}

// 获取GetGuildUrlImageToBase64的值
func GetGuildUrlImageToBase64() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to GuildUrlImageToBase64 value.")
		return false
	}
	return instance.Settings.GuildUrlImageToBase64
}

// GetTencentBucketURL 获取 TencentBucketURL
func GetTencentBucketURL() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get TencentBucketURL.")
		return ""
	}

	bucketName := instance.Settings.TencentBucketName
	bucketRegion := instance.Settings.TencentBucketRegion

	// 构建并返回URL
	if bucketName == "" || bucketRegion == "" {
		fmt.Println("Warning: Tencent bucket name or region is not configured.")
		return ""
	}

	return fmt.Sprintf("https://%s.cos.%s.myqcloud.com", bucketName, bucketRegion)
}

// GetTencentCosSecretid 获取 TencentCosSecretid
func GetTencentCosSecretid() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get TencentCosSecretid.")
		return ""
	}
	return instance.Settings.TencentCosSecretid
}

// GetTencentSecretKey 获取 TencentSecretKey
func GetTencentSecretKey() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get TencentSecretKey.")
		return ""
	}
	return instance.Settings.TencentSecretKey
}

// 获取GetTencentAudit的值
func GetTencentAudit() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to TencentAudit value.")
		return false
	}
	return instance.Settings.TencentAudit
}

// 获取 Oss 模式
func GetOssType() int {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get ExtraPicAuditingType version.")
		return 0
	}
	return instance.Settings.OssType
}

// GetOssTypeName 返回 oss_type 的可读名称
func GetOssTypeName(t int) string {
	switch t {
	case OssTypeLocal:
		return "local"
	case OssTypeTencent:
		return "tencent_cos"
	case OssTypeBaidu:
		return "baidu_bos"
	case OssTypeAliyun:
		return "aliyun_oss"
	case OssTypeCOS:
		return "cos"
	case OssTypeBilibili:
		return "bilibili"
	case OssTypeQQChannel:
		return "qq_channel"
	case OssTypeChatGLM:
		return "chatglm"
	case OssTypeUkaka:
		return "ukaka"
	case OssTypeXingye:
		return "xingye"
	case OssTypeNature:
		return "nature"
	default:
		return "unknown"
	}
}

// 获取BaiduBOSBucketName
func GetBaiduBOSBucketName() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get BaiduBOSBucketName.")
		return ""
	}
	return instance.Settings.BaiduBOSBucketName
}

// 获取BaiduBCEAK
func GetBaiduBCEAK() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get BaiduBCEAK.")
		return ""
	}
	return instance.Settings.BaiduBCEAK
}

// 获取BaiduBCESK
func GetBaiduBCESK() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get BaiduBCESK.")
		return ""
	}
	return instance.Settings.BaiduBCESK
}

// 获取BaiduAudit
func GetBaiduAudit() int {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get BaiduAudit.")
		return 0
	}
	return instance.Settings.BaiduAudit
}

// 获取阿里云的oss地址 外网的
func GetAliyunEndpoint() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get AliyunEndpoint.")
		return ""
	}
	return instance.Settings.AliyunEndpoint
}

// GetRegionID 从 AliyunEndpoint 获取 regionId
func GetRegionID() string {
	endpoint := GetAliyunEndpoint()
	if endpoint == "" {
		return ""
	}

	// 去除协议头（如 "https://"）
	endpoint = strings.TrimPrefix(endpoint, "http://")
	endpoint = strings.TrimPrefix(endpoint, "https://")

	// 将 endpoint 按照 "." 分割
	parts := strings.Split(endpoint, ".")
	if len(parts) >= 2 {
		// 第一部分应该是包含 regionId 的信息（例如 "oss-cn-hangzhou"）
		regionInfo := parts[0]
		// 进一步提取 regionId
		regionParts := strings.SplitN(regionInfo, "-", 3)
		if len(regionParts) >= 3 {
			// 返回 "cn-hangzhou" 部分
			return regionParts[1] + "-" + regionParts[2]
		}
	}
	return ""
}

// GetAliyunAccessKeyId 获取阿里云OSS的AccessKeyId
func GetAliyunAccessKeyId() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get AliyunAccessKeyId.")
		return ""
	}
	return instance.Settings.AliyunAccessKeyId
}

// GetAliyunAccessKeySecret 获取阿里云OSS的AccessKeySecret
func GetAliyunAccessKeySecret() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get AliyunAccessKeySecret.")
		return ""
	}
	return instance.Settings.AliyunAccessKeySecret
}

// GetAliyunBucketName 获取阿里云OSS的AliyunBucketName
func GetAliyunBucketName() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get AliyunBucketName.")
		return ""
	}
	return instance.Settings.AliyunBucketName
}

// 获取GetAliyunAudit的值
func GetAliyunAudit() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to AliyunAudit value.")
		return false
	}
	return instance.Settings.AliyunAudit
}

// GetDiscoverUnknownEvents 获取是否订阅所有未知 intent 位
func GetDiscoverUnknownEvents() bool {
	mu.RLock()
	defer mu.RUnlock()
	if instance == nil {
		return false
	}
	return instance.Settings.DiscoverUnknownEvents
}

func GetSuppressDisallowedIntents() bool {
	mu.RLock()
	defer mu.RUnlock()
	if instance == nil {
		return false
	}
	return instance.Settings.SuppressDisallowedIntents
}

// 获取Alias的值
func GetAlias() []string {
	mu.RLock()
	defer mu.RUnlock()
	if instance != nil {
		return instance.Settings.Alias
	}
	return nil // 返回nil，如果instance为nil
}

// 获取SelfIntroduce的值
func GetSelfIntroduce() []string {
	mu.RLock()
	defer mu.RUnlock()
	if instance != nil {
		return instance.Settings.SelfIntroduce
	}
	return nil // 返回nil，如果instance为nil
}

// 获取WhiteEnable的值
func GetWhiteEnable(index int) bool {
	mu.RLock()
	defer mu.RUnlock()

	// 检查instance或instance.Settings.WhiteEnable是否为nil
	if instance == nil || instance.Settings.WhiteEnable == nil {
		return true // 如果为nil，返回默认值true
	}

	// 调整索引以符合从0开始的数组索引
	adjustedIndex := index - 1

	// 检查索引是否在数组范围内
	if adjustedIndex >= 0 && adjustedIndex < len(instance.Settings.WhiteEnable) {
		return instance.Settings.WhiteEnable[adjustedIndex]
	}

	// 如果索引超出范围，返回默认值true
	return true
}

// 获取IdentifyAppids的值
func GetIdentifyAppids() []int64 {
	mu.RLock()
	defer mu.RUnlock()
	if instance != nil {
		return instance.Settings.IdentifyAppids
	}
	return nil // 返回nil，如果instance为nil
}

// 获取 TransFormApiIds 的值
func GetTransFormApiIds() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to TransFormApiIds value.")
		return false
	}
	return instance.Settings.TransFormApiIds
}

// 获取 CustomTemplateID 的值
func GetCustomTemplateID() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get CustomTemplateID.")
		return ""
	}
	return instance.Settings.CustomTemplateID
}

// 获取 KeyBoardIDD 的值
func GetKeyBoardID() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get KeyBoardID.")
		return ""
	}
	return instance.Settings.KeyBoardID
}

// 获取Uin int64
func GetUinint64() int64 {
	mu.RLock()
	defer mu.RUnlock()
	if instance != nil {
		return instance.Settings.Uin
	}
	return 0
}

// 获取Uin String
func GetUinStr() string {
	mu.RLock()
	defer mu.RUnlock()
	if instance != nil {
		return fmt.Sprintf("%d", instance.Settings.Uin)
	}
	return "0"
}

// 获取 VV GetVwhitePrefixMode 的值
func GetVwhitePrefixMode() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to VwhitePrefixMode value.")
		return false
	}
	return instance.Settings.VwhitePrefixMode
}

// 获取Enters的值
func GetEnters() []string {
	mu.RLock()
	defer mu.RUnlock()
	if instance != nil {
		return instance.Settings.Enters
	}
	return nil // 返回nil，如果instance为nil
}

// 获取EntersExcept
func GetEntersExcept() []string {
	mu.RLock()
	defer mu.RUnlock()
	if instance != nil {
		return instance.Settings.EntersExcept
	}
	return nil // 返回nil，如果instance为nil
}

// 获取 LinkPrefix
func GetLinkPrefix() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get LinkPrefix.")
		return ""
	}
	return instance.Settings.LinkPrefix
}

// 获取 LinkBots 数组
func GetLinkBots() []string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get LinkBots.")
		return nil
	}
	return instance.Settings.LinkBots
}

// 获取 LinkText
func GetLinkText() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get LinkText.")
		return ""
	}
	return instance.Settings.LinkText
}

// 获取 LinkPic
func GetLinkPic() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get LinkPic.")
		return ""
	}
	return instance.Settings.LinkPic
}

// 获取 GetMusicPrefix
func GetMusicPrefix() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get MusicPrefix.")
		return ""
	}
	return instance.Settings.MusicPrefix
}

// 获取 GetDisableWebui 的值
func GetDisableWebui() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to GetDisableWebui value.")
		return false
	}
	return instance.Settings.DisableWebui
}

// 获取 GetBotForumTitle
func GetBotForumTitle() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get BotForumTitle.")
		return ""
	}
	return instance.Settings.BotForumTitle
}

// 获取 GetGlobalInteractionToMessage 的值
func GetGlobalInteractionToMessage() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to GlobalInteractionToMessage value.")
		return false
	}
	return instance.Settings.GlobalInteractionToMessage
}

// 获取 AutoPutInteraction 的值
func GetAutoPutInteraction() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to AutoPutInteraction value.")
		return false
	}
	return instance.Settings.AutoPutInteraction
}

// 获取 PutInteractionDelay 延迟
func GetPutInteractionDelay() int {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get PutInteractionDelay.")
		return 0
	}
	return instance.Settings.PutInteractionDelay
}

// 获取Fix11300开关
func GetFix11300() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to Fix11300 value.")
		return false
	}
	return instance.Settings.Fix11300
}

// 获取LotusWithoutIdmaps开关
func GetLotusWithoutIdmaps() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to LotusWithoutIdmaps value.")
		return false
	}
	return instance.Settings.LotusWithoutIdmaps
}

// 获取GetGroupListAllGuilds开关
func GetGroupListAllGuilds() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to GetGroupListAllGuilds value.")
		return false
	}
	return instance.Settings.GetGroupListAllGuilds
}

// 获取 GetGroupListGuilds  数量
func GetGetGroupListGuilds() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get GetGroupListGuilds.")
		return "10"
	}
	return instance.Settings.GetGroupListGuilds
}

// 获取GetGroupListReturnGuilds开关
func GetGroupListReturnGuilds() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to GetGroupListReturnGuilds value.")
		return false
	}
	return instance.Settings.GetGroupListReturnGuilds
}

// 获取 GetGroupListGuidsType  数量
func GetGroupListGuidsType() int {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get GetGroupListGuidsType.")
		return 0
	}
	return instance.Settings.GetGroupListGuidsType
}

// 获取 GetGroupListDelay  数量
func GetGroupListDelay() int {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get GetGroupListDelay.")
		return 0
	}
	return instance.Settings.GetGroupListDelay
}

// 获取GetGlobalServerTempQQguild开关
func GetGlobalServerTempQQguild() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to GlobalServerTempQQguild value.")
		return false
	}
	return instance.Settings.GlobalServerTempQQguild
}

// ---------- 图床配置 ----------

// GetImageHostingCOS 获取 COS 图床配置
func GetImageHostingCOS() structs.ImageHostingCOS {
	mu.RLock()
	defer mu.RUnlock()
	if instance == nil {
		return structs.ImageHostingCOS{}
	}
	return instance.Settings.COS
}

// GetImageHostingBilibili 获取 Bilibili 图床配置
func GetImageHostingBilibili() structs.ImageHostingBilibili {
	mu.RLock()
	defer mu.RUnlock()
	if instance == nil {
		return structs.ImageHostingBilibili{}
	}
	return instance.Settings.Bilibili
}

// GetImageHostingQQChannel 获取 QQ频道 图床配置
func GetImageHostingQQChannel() structs.ImageHostingQQChannel {
	mu.RLock()
	defer mu.RUnlock()
	if instance == nil {
		return structs.ImageHostingQQChannel{}
	}
	return instance.Settings.QQChannel
}

// GetImageHostingQQChannelToken 获取 QQ频道 图床的 Authorization token
func GetImageHostingQQChannelToken() string {
	mu.RLock()
	defer mu.RUnlock()
	if instance == nil {
		return ""
	}
	return instance.Settings.QQChannel.Token
}

// GetImageHostingChatGLM 获取 ChatGLM 图床配置
func GetImageHostingChatGLM() structs.ImageHostingSimple {
	mu.RLock()
	defer mu.RUnlock()
	if instance == nil {
		return structs.ImageHostingSimple{}
	}
	return instance.Settings.ChatGLM
}

// GetImageHostingUkaka 获取 Ukaka 图床配置
func GetImageHostingUkaka() structs.ImageHostingSimple {
	mu.RLock()
	defer mu.RUnlock()
	if instance == nil {
		return structs.ImageHostingSimple{}
	}
	return instance.Settings.Ukaka
}

// GetImageHostingXingye 获取 星野 图床配置
func GetImageHostingXingye() structs.ImageHostingSimple {
	mu.RLock()
	defer mu.RUnlock()
	if instance == nil {
		return structs.ImageHostingSimple{}
	}
	return instance.Settings.Xingye
}

// GetImageHostingNature 获取 Nature 图床配置
func GetImageHostingNature() structs.ImageHostingSimple {
	mu.RLock()
	defer mu.RUnlock()
	if instance == nil {
		return structs.ImageHostingSimple{}
	}
	return instance.Settings.Nature
}

// 获取ServerTempQQguild
func GetServerTempQQguild() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to ServerTempQQguild value.")
		return "0"
	}
	return instance.Settings.ServerTempQQguild
}

// 获取ServerTempQQguildPool
func GetServerTempQQguildPool() []string {
	mu.RLock()
	defer mu.RUnlock()
	if instance != nil {
		return instance.Settings.ServerTempQQguildPool
	}
	return nil // 返回nil，如果instance为nil
}

// 获取 AutoWithdraw 数组
func GetAutoWithdraw() []string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get AutoWithdraw.")
		return nil
	}
	return instance.Settings.AutoWithdraw
}

// 获取 GetAutoWithdrawTime  数量
func GetAutoWithdrawTime() int {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get AutoWithdrawTime.")
		return 0
	}
	return instance.Settings.AutoWithdrawTime
}

// 获取DefaultChangeWord
func GetDefaultChangeWord() string {
	mu.RLock()
	defer mu.RUnlock()
	if instance != nil {
		return instance.Settings.DefaultChangeWord
	}
	return "*"
}

// 获取敏感词替换状态
func GetEnableChangeWord() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to EnableChangeWord.")
		return false
	}
	return instance.Settings.EnableChangeWord
}

// 获取GlobalGroupMsgRejectReciveEventToMessage状态
func GetGlobalGroupMsgRejectReciveEventToMessage() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to GlobalGroupMsgRejectReciveEventToMessage.")
		return false
	}
	return instance.Settings.GlobalGroupMsgRejectReciveEventToMessage
}

// 获取GlobalGroupMsgRejectMessage
func GetGlobalGroupMsgRejectMessage() string {
	mu.RLock()
	defer mu.RUnlock()
	if instance != nil {
		return instance.Settings.GlobalGroupMsgRejectMessage
	}
	return ""
}

// 获取GlobalGroupMsgRejectMessage
func GetGlobalGroupMsgReceiveMessage() string {
	mu.RLock()
	defer mu.RUnlock()
	if instance != nil {
		return instance.Settings.GlobalGroupMsgReceiveMessage
	}
	return ""
}

// 获取EntersAsBlock状态
func GetEntersAsBlock() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to EntersAsBlock.")
		return false
	}
	return instance.Settings.EntersAsBlock
}

// 获取NativeMD状态
func GetNativeMD() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to NativeMD.")
		return false
	}
	return instance.Settings.NativeMD
}

// 获取DowntimeMessage
func GetDowntimeMessage() string {
	mu.RLock()
	defer mu.RUnlock()
	if instance != nil {
		return instance.Settings.DowntimeMessage
	}
	return ""
}

// 获取GetAutoLink的值
func GetAutoLink() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to AutoLink value.")
		return false
	}
	return instance.Settings.AutoLink
}

// 获取GetLinkLines的值
func GetLinkLines() int {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to LinkLines value.")
		return 2 //默认2个一行
	}

	return instance.Settings.LinkLines
}

// 获取GetLinkNum的值
func GetLinkNum() int {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to LinkNum value.")
		return 6 //默认6个
	}

	return instance.Settings.LinkNum
}

// 获取GetDoNotReplaceAppid的值
func GetDoNotReplaceAppid() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to DoNotReplaceAppid value.")
		return false
	}
	return instance.Settings.DoNotReplaceAppid
}

// 获取GetMemoryMsgid的值
func GetMemoryMsgid() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to MemoryMsgid value.")
		return false
	}
	return instance.Settings.MemoryMsgid
}

// 获取GetLotusGrpc的值
func GetLotusGrpc() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to LotusGrpc value.")
		return false
	}
	return instance.Settings.LotusGrpc
}

// 获取LotusWithoutUploadPic的值
func GetLotusWithoutUploadPic() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to LotusWithoutUploadPic value.")
		return false
	}
	return instance.Settings.LotusWithoutUploadPic
}

// 获取DisableErrorChan的值
func GetDisableErrorChan() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to DisableErrorChan value.")
		return false
	}
	return instance.Settings.DisableErrorChan
}

// 获取StringOb11的值
func GetStringOb11() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to StringOb11 value.")
		return false
	}
	return instance.Settings.StringOb11
}

// 获取StringAction的值
func GetStringAction() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to StringAction value.")
		return false
	}
	return instance.Settings.StringAction
}

// 获取 PutInteractionExcept 数组
func GetPutInteractionExcept() []string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get PutInteractionExcept.")
		return nil
	}
	return instance.Settings.PutInteractionExcept
}

// 获取 LogSuffixPerMins
func GetLogSuffixPerMins() int {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get LogSuffixPerMins.")
		return 0
	}
	return instance.Settings.LogSuffixPerMins
}

// 获取ThreadsRetMsg的值
func GetThreadsRetMsg() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to ThreadsRetMsg value.")
		return false
	}
	return instance.Settings.ThreadsRetMsg
}

// 获取NoRetMsg的值
func GetNoRetMsg() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to NoRetMsg value.")
		return false
	}
	return instance.Settings.NoRetMsg
}

func GetForceSsl() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to ForceSSL value.")
		return false
	}
	return instance.Settings.ForceSSL
}

func GetHttpPortAfterSsl() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to get HttpPortAfterSSL.")
		return "444" // 或者返回一个默认的 ImageLimit 值
	}

	return instance.Settings.HttpPortAfterSSL
}

// 获取UnionWebhook的值
func GetUnionWebhook() string {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to UnionWebhook value.")
		return "0"
	}
	return instance.Settings.UnionWebhook
}

func GetUnionID() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to UnionID value.")
		return false
	}
	return instance.Settings.UnionID
}

// GetGlobalC2CMsgSwitchEventToMessage 获取是否将C2C开关事件转换为消息
func GetGlobalC2CMsgSwitchEventToMessage() bool {
	mu.RLock()
	defer mu.RUnlock()

	if instance == nil {
		fmt.Println("Warning: instance is nil when trying to GetGlobalC2CMsgSwitchEventToMessage.")
		return false
	}
	return instance.Settings.GlobalC2CMsgSwitchEventToMessage
}

// GetGlobalC2CMsgRejectMessage 获取C2C拒绝时的自定义文本
func GetGlobalC2CMsgRejectMessage() string {
	mu.RLock()
	defer mu.RUnlock()
	if instance != nil {
		return instance.Settings.GlobalC2CMsgRejectMessage
	}
	return ""
}

// GetGlobalC2CMsgReceiveMessage 获取C2C开启时的自定义文本
func GetGlobalC2CMsgReceiveMessage() string {
	mu.RLock()
	defer mu.RUnlock()
	if instance != nil {
		return instance.Settings.GlobalC2CMsgReceiveMessage
	}
	return ""
}

// GetLogColorEnabled 获取是否开启日志彩色
func GetLogColorEnabled() bool {
	mu.RLock()
	defer mu.RUnlock()
	if instance == nil {
		return true // 默认开启
	}
	return instance.Settings.LogColorEnabled
}

// GetLogMaxAgeDays 获取日志最大保存天数
func GetLogMaxAgeDays() int {
	mu.RLock()
	defer mu.RUnlock()
	if instance == nil {
		return 7
	}
	if instance.Settings.LogMaxAgeDays <= 0 {
		return 7
	}
	return instance.Settings.LogMaxAgeDays
}

// GetLogMaxSizeMB 获取日志单文件最大大小
func GetLogMaxSizeMB() int {
	mu.RLock()
	defer mu.RUnlock()
	if instance == nil {
		return 24
	}
	if instance.Settings.LogMaxSizeMB <= 0 {
		return 24
	}
	return instance.Settings.LogMaxSizeMB
}

// GetLogKeepFiles 获取本地旧日志文件最大保留个数
func GetLogKeepFiles() int {
	mu.RLock()
	defer mu.RUnlock()
	if instance == nil {
		return 12
	}
	if instance.Settings.LogKeepFiles <= 0 {
		return 12
	}
	return instance.Settings.LogKeepFiles
}

// GetLogSlowEventThresholdMS 获取慢事件阈值
func GetLogSlowEventThresholdMS() int {
	mu.RLock()
	defer mu.RUnlock()
	if instance == nil {
		return 500
	}
	if instance.Settings.LogSlowEventThresholdMS <= 0 {
		return 500
	}
	return instance.Settings.LogSlowEventThresholdMS
}
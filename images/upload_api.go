package images

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"image"
	"io"
	"net"
	"net/http"
	"net/url"
	"strings"
	"sync"
	"time"

	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/imagehosting"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/hoshinonyaruko/gensokyo/oss"
	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

// 包级私有变量，用于存储当前URL索引
var (
	currentURLIndex int
	urlsMutex       sync.Mutex
)

// createAndUploadMediaMessage 创建并上传媒体消息
func CreateAndUploadMediaMessage(ctx context.Context, base64EncodedData, eventID string, fileType uint64, srvSendMsg bool, content string, groupID string, messageID string, msgseq int, apiv2 openapi.OpenAPI) (*dto.MessageToCreate, error) {

	// 构造RichMediaMessage对象
	richMediaMessage := &dto.RichMediaMessage{
		EventID:    eventID,
		FileType:   fileType,
		FileData:   base64EncodedData,
		SrvSendMsg: srvSendMsg,
		Content:    content,
		FileName:   content, // 文件类型时 content 即为文件名
	}

	// 调用uploadMedia函数上传媒体
	fileInfo, err := uploadMedia(ctx, groupID, richMediaMessage, apiv2)
	if err != nil {
		return nil, err
	}

	// 构造返回的MessageToCreate对象
	groupMessage := &dto.MessageToCreate{
		Content: content,
		Media: &dto.Media{
			FileInfo: fileInfo,
		},
		MsgID:   messageID,
		EventID: eventID,
		MsgSeq:  msgseq,
		MsgType: 7, // 假设7是组合消息类型
	}

	return groupMessage, nil
}

// createAndUploadMediaMessagePrivate 创建并上传媒体消息给私人聊天
func CreateAndUploadMediaMessagePrivate(ctx context.Context, base64EncodedData, eventID string, fileType uint64, srvSendMsg bool, content string, userID string, messageID string, msgseq int, apiv2 openapi.OpenAPI) (*dto.MessageToCreate, error) {

	// 构造RichMediaMessage对象
	richMediaMessage := &dto.RichMediaMessage{
		EventID:    eventID,
		FileType:   fileType,
		FileData:   base64EncodedData,
		SrvSendMsg: srvSendMsg,
		Content:    content,
		FileName:   content, // 文件类型时 content 即为文件名
	}

	// 调用uploadMediaPrivate函数上传媒体
	fileInfo, err := uploadMediaPrivate(ctx, userID, richMediaMessage, apiv2)
	if err != nil {
		return nil, err
	}

	// 构造返回的MessageToCreate对象
	privateMessage := &dto.MessageToCreate{
		Content: content,
		Media: &dto.Media{
			FileInfo: fileInfo,
		},
		MsgID:   messageID,
		EventID: eventID,
		MsgSeq:  msgseq,
		MsgType: 7, // 假设7是组合消息类型
	}

	return privateMessage, nil
}

// uploadMedia 上传媒体并返回FileInfo
// 使用 300s 超时 context 避免大文件上传超时
func uploadMedia(ctx context.Context, groupID string, richMediaMessage *dto.RichMediaMessage, apiv2 openapi.OpenAPI) (string, error) {
	uploadCtx, cancel := context.WithTimeout(context.Background(), 300*time.Second)
	defer cancel()
	// 调用API来上传媒体
	messageReturn, err := apiv2.PostGroupMessage(uploadCtx, groupID, richMediaMessage)
	if err != nil {
		return "", err
	}
	// 返回上传后的FileInfo
	return messageReturn.MediaResponse.FileInfo, nil
}

// uploadMediaPrivate 上传媒体并返回FileInfo
// 使用 300s 超时 context 避免大文件上传超时
func uploadMediaPrivate(ctx context.Context, UserID string, richMediaMessage *dto.RichMediaMessage, apiv2 openapi.OpenAPI) (string, error) {
	uploadCtx, cancel := context.WithTimeout(context.Background(), 300*time.Second)
	defer cancel()
	// 调用API来上传媒体
	messageReturn, err := apiv2.PostC2CMessage(uploadCtx, UserID, richMediaMessage)
	if err != nil {
		return "", err
	}
	// 返回上传后的FileInfo
	return messageReturn.MediaResponse.FileInfo, nil
}

// UploadBase64ImageToServer 将base64图片转换成公开URL
// 根据 oss_type 选择上传后端：0~3 走本地/云OSS，4~10 走 imagehosting 对应图床
func UploadBase64ImageToServer(base64Image string, apiv2 openapi.OpenAPI) (string, int, int, error) {
	imageBytes, decErr := base64.StdEncoding.DecodeString(base64Image)
	if decErr != nil {
		return "", 0, 0, fmt.Errorf("base64 解码失败: %w", decErr)
	}

	var picURL string
	var err error
	extraPicAuditingType := config.GetOssType()
	switch extraPicAuditingType {
	case config.OssTypeLocal:
		picURL, err = originalUploadBehavior(base64Image)
	case config.OssTypeTencent:
		picURL, err = oss.UploadAndAuditImage(base64Image) // 腾讯
	case config.OssTypeBaidu:
		picURL, err = oss.UploadAndAuditImageB(base64Image) // 百度
	case config.OssTypeAliyun:
		picURL, err = oss.UploadAndAuditImageA(base64Image) // 阿里
	case config.OssTypeCOS, config.OssTypeBilibili, config.OssTypeQQChannel,
		config.OssTypeChatGLM, config.OssTypeUkaka, config.OssTypeXingye, config.OssTypeNature:
		provider := config.GetOssTypeName(extraPicAuditingType)
		picURL, err = imagehosting.UploadProvider(provider, imageBytes, "image.png")
	default:
		return "", 0, 0, errors.New("invalid oss_type")
	}
	if err != nil {
		return "", 0, 0, err
	}

	// 本地/云OSS 上传后通过 URL 获取尺寸；imagehosting 后端可直接从 bytes 获取
	var width, height int
	if extraPicAuditingType >= config.OssTypeCOS {
		if img, _, err := image.DecodeConfig(bytes.NewReader(imageBytes)); err == nil {
			width, height = img.Width, img.Height
		}
	} else {
		height, width, err = GetImageDimensions(picURL)
		if err != nil {
			mylog.Printf("获取图片宽高出错")
		}
	}

	return picURL, width, height, nil
}

// UploadBase64RecordToServer 将base64语音转换成公开URL
// oss_type 只影响图片上传；语音仍走本地服务器或原有云 OSS（1~3）
func UploadBase64RecordToServer(base64Record string) (string, error) {
	extraPicAuditingType := config.GetOssType()

	switch extraPicAuditingType {
	case config.OssTypeTencent, config.OssTypeBaidu, config.OssTypeAliyun:
		return oss.UploadAndAuditRecord(base64Record) // 云OSS
	case config.OssTypeCOS, config.OssTypeBilibili, config.OssTypeQQChannel,
		config.OssTypeChatGLM, config.OssTypeUkaka, config.OssTypeXingye, config.OssTypeNature:
		mylog.Printf("当前 oss_type=%d 仅支持图片，语音回退到本机上传", extraPicAuditingType)
		return originalUploadBehaviorRecord(base64Record)
	default:
		// 0 或其他未知值均走本地服务器
		return originalUploadBehaviorRecord(base64Record)
	}
}

func originalUploadBehavior(base64Image string) (string, error) {
	// 原有的UploadBase64ImageToServer函数的实现
	protocol := "http"
	serverPort := config.GetPortValue()
	if serverPort == "443" ||config.GetForceSsl(){
		protocol = "https"
	}

	// 如果lotus为真
	if config.GetLotusValue() && !config.GetLotusWithoutUploadPic() {
		serverDir := config.GetServer_dir()
		url := fmt.Sprintf("%s://%s:%s/uploadpic", protocol, serverDir, serverPort)

		resp, err := postImageToServer(base64Image, url)
		if err != nil {
			return "", err
		}
		return resp, nil
	}

	serverDir := config.GetServer_dir()
	if serverPort == "443" ||config.GetForceSsl(){
		protocol = "http"
		serverPort = config.GetHttpPortAfterSsl()
	}

	if isPublicAddress(serverDir) {
		url := fmt.Sprintf("%s://127.0.0.1:%s/uploadpic", protocol, serverPort)

		resp, err := postImageToServer(base64Image, url)
		if err != nil {
			return "", err
		}
		return resp, nil
	}
	return "", errors.New("local server uses a private address; image upload failed")
}

// 将base64语音通过lotus转换成url
func originalUploadBehaviorRecord(base64Image string) (string, error) {
	// 根据serverPort确定协议
	protocol := "http"
	serverPort := config.GetPortValue()
	if serverPort == "443" ||config.GetForceSsl(){
		protocol = "https"
	}

	if config.GetLotusValue() && !config.GetLotusWithoutUploadPic() {
		serverDir := config.GetServer_dir()
		url := fmt.Sprintf("%s://%s:%s/uploadrecord", protocol, serverDir, serverPort)

		resp, err := postRecordToServer(base64Image, url)
		if err != nil {
			return "", err
		}
		return resp, nil
	}

	serverDir := config.GetServer_dir()
	// 当端口是443时，使用HTTP和444端口
	if serverPort == "443" ||config.GetForceSsl(){
		protocol = "http"
		serverPort = config.GetHttpPortAfterSsl()
	}

	if isPublicAddress(serverDir) {
		url := fmt.Sprintf("%s://127.0.0.1:%s/uploadrecord", protocol, serverPort)

		resp, err := postRecordToServer(base64Image, url)
		if err != nil {
			return "", err
		}
		return resp, nil
	}
	return "", errors.New("local server uses a private address; image record failed")
}

// 请求图床api(图床就是lolus为false的gensokyo)
func postImageToServer(base64Image, targetURL string) (string, error) {
	data := url.Values{}
	data.Set("base64Image", base64Image) // 修改字段名以与服务器匹配

	resp, err := http.PostForm(targetURL, data)
	if err != nil {
		return "", fmt.Errorf("failed to send request: %v", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return "", fmt.Errorf("error response from server: %s", resp.Status)
	}

	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return "", fmt.Errorf("failed to read response body: %v", err)
	}

	var responseMap map[string]interface{}
	if err := json.Unmarshal(body, &responseMap); err != nil {
		return "", fmt.Errorf("failed to unmarshal response: %v", err)
	}

	if value, ok := responseMap["url"]; ok {
		return fmt.Sprintf("%v", value), nil
	}

	return "", fmt.Errorf("URL not found in response")
}

// 请求语音床api(图床就是lolus为false的gensokyo)
func postRecordToServer(base64Image, targetURL string) (string, error) {
	data := url.Values{}
	data.Set("base64Record", base64Image) // 修改字段名以与服务器匹配

	resp, err := http.PostForm(targetURL, data)
	if err != nil {
		return "", fmt.Errorf("failed to send request: %v", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return "", fmt.Errorf("error response from server: %s", resp.Status)
	}

	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return "", fmt.Errorf("failed to read response body: %v", err)
	}

	var responseMap map[string]interface{}
	if err := json.Unmarshal(body, &responseMap); err != nil {
		return "", fmt.Errorf("failed to unmarshal response: %v", err)
	}

	if value, ok := responseMap["url"]; ok {
		return fmt.Sprintf("%v", value), nil
	}

	return "", fmt.Errorf("URL not found in response")
}

// 判断是否公网ip 填写域名也会被认为是公网,但需要用户自己确保域名正确解析到gensokyo所在的ip地址
func isPublicAddress(addr string) bool {
	if strings.Contains(addr, "localhost") || strings.HasPrefix(addr, "127.") || strings.HasPrefix(addr, "192.168.") {
		return false
	}
	if net.ParseIP(addr) != nil {
		return true
	}
	// If it's not a recognized IP address format, consider it a domain name (public).
	return true
}

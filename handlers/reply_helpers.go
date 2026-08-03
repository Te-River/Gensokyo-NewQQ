package handlers

import (
	"encoding/base64"
	"fmt"
	"io"
	"net/http"
	"os"
	"time"

	"github.com/hoshinonyaruko/gensokyo/images"

	"github.com/tencent-connect/botgo/dto"
	"github.com/tencent-connect/botgo/openapi"
)

// GenerateReplyMessage 根据消息内容生成回复消息对象
func GenerateReplyMessage(id string, foundItems map[string][]string, messageText string, msgseq int, api2 openapi.OpenAPI) (*dto.MessageToCreate, bool) {
	var reply dto.MessageToCreate
	var isBase64 bool

	if imageURLs, ok := foundItems["local_image"]; ok && len(imageURLs) > 0 {
		// 从本地图路径读取图片
		imageData, err := readLocalImage(imageURLs[0])
		if err != nil {
			reply = dto.MessageToCreate{
				Content: "错误: 图片文件不存在",
				MsgID:   id,
				MsgSeq:  msgseq,
				MsgType: 0,
			}
			return &reply, false
		}
		compressedData, err := images.CompressSingleImage(imageData)
		if err != nil {
			reply = dto.MessageToCreate{
				Content: "错误: 压缩图片失败",
				MsgID:   id,
				MsgSeq:  msgseq,
				MsgType: 0,
			}
			return &reply, false
		}
		base64Encoded := base64.StdEncoding.EncodeToString(compressedData)
		reply = dto.MessageToCreate{
			Content: base64Encoded,
			MsgID:   id,
			MsgSeq:  msgseq,
			MsgType: 0,
		}
		isBase64 = true
	} else if imageURLs, ok := foundItems["url_image"]; ok && len(imageURLs) > 0 {
		// 判断是否需要将图片转换为 base64 编码
		base64Image, err := downloadImageAndConvertToBase64("http://" + imageURLs[0])
		if err == nil {
			reply = dto.MessageToCreate{
				Content: base64Image,
				MsgID:   id,
				MsgSeq:  msgseq,
				MsgType: 0,
			}
			isBase64 = true
		} else {
			reply = dto.MessageToCreate{
				Image:   "http://" + imageURLs[0],
				MsgID:   id,
				MsgSeq:  msgseq,
				MsgType: 0,
			}
		}
	} else if imageURLs, ok := foundItems["url_images"]; ok && len(imageURLs) > 0 {
		// 判断是否需要将图片转换为 base64 编码
		base64Image, err := downloadImageAndConvertToBase64("https://" + imageURLs[0])
		if err == nil {
			reply = dto.MessageToCreate{
				Content: base64Image,
				MsgID:   id,
				MsgSeq:  msgseq,
				MsgType: 0,
			}
			isBase64 = true
		} else {
			reply = dto.MessageToCreate{
				Image:   "https://" + imageURLs[0],
				MsgID:   id,
				MsgSeq:  msgseq,
				MsgType: 0,
			}
		}
	} else if base64_image, ok := foundItems["base64_image"]; ok && len(base64_image) > 0 {
		// base64图片
		reply = dto.MessageToCreate{
			Content: base64_image[0],
			MsgID:   id,
			MsgSeq:  msgseq,
			MsgType: 0,
		}
		isBase64 = true
	} else {
		// 发文本信息
		reply = dto.MessageToCreate{
			Content: messageText,
			MsgID:   id,
			MsgSeq:  msgseq,
			MsgType: 0,
		}
	}

	return &reply, isBase64
}

// downloadImageAndConvertToBase64 下载图片并转换为 base64 编码字符串
func downloadImageAndConvertToBase64(url string) (string, error) {
	// SSRF 校验：禁止访问私有地址、回环地址、链路本地地址
	if isPrivateOrLoopback(url) {
		return "", fmt.Errorf("SSRF 阻止: 目标地址为私有地址: %s", url)
	}

	// 设置带超时的 HTTP Client（30 秒超时）
	client := &http.Client{
		Timeout: 30 * time.Second,
	}

	// 发送 HTTP GET 请求以获取图片数据
	resp, err := client.Get(url)
	if err != nil {
		return "", err
	}
	defer resp.Body.Close()

	// 读取响应的内容
	data, err := io.ReadAll(resp.Body)
	if err != nil {
		return "", err
	}

	// 将图片数据转换为 base64 编码
	base64Image := base64.StdEncoding.EncodeToString(data)
	return base64Image, nil
}

// readLocalImage 读取本地图片文件
func readLocalImage(path string) ([]byte, error) {
	return os.ReadFile(path)
}

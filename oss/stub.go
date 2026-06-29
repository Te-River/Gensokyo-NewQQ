//go:build small

package oss

import "github.com/hoshinonyaruko/gensokyo/mylog"

func UploadAndAuditImage(base64Data string) (string, error) {
	mylog.Printf("[OSS] 云存储未启用（small build），图片将使用本地存储")
	return "", nil
}

func UploadAndAuditImageA(base64Data string) (string, error) {
	mylog.Printf("[OSS] 阿里云OSS未启用（small build），图片将使用本地存储")
	return "", nil
}

func UploadAndAuditImageB(base64Data string) (string, error) {
	mylog.Printf("[OSS] 百度云BOS未启用（small build），图片将使用本地存储")
	return "", nil
}

func UploadAndAuditRecord(base64Data string) (string, error) {
	mylog.Printf("[OSS] 云存储未启用（small build），语音将使用本地存储")
	return "", nil
}

func UploadAndAuditRecordA(base64Data string) (string, error) {
	mylog.Printf("[OSS] 阿里云OSS未启用（small build），语音将使用本地存储")
	return "", nil
}

func UploadAndAuditRecordB(base64Data string) (string, error) {
	mylog.Printf("[OSS] 百度云BOS未启用（small build），语音将使用本地存储")
	return "", nil
}

func CheckText(text string) (bool, error) {
	return true, nil
}

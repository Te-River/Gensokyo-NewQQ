//go:build !small

package handlers

import (
	"encoding/base64"

	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/skip2/go-qrcode"
)

// generateQRCode 将URL转换为QR码的base64图片数据
func generateQRCode(originalURL string) (string, error) {
	qrCodeGenerator, err := qrcode.New(originalURL, qrcode.High)
	if err != nil {
		return "", err
	}
	qrCodeGenerator.DisableBorder = true
	qrSize := config.GetQrSize()
	pngBytes, err := qrCodeGenerator.PNG(qrSize)
	if err != nil {
		return "", err
	}
	return base64.StdEncoding.EncodeToString(pngBytes), nil
}

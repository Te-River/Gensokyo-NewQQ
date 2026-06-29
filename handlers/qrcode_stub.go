//go:build small

package handlers

import "errors"

// generateQRCode 小型构建: 不依赖 go-qrcode 库，返回错误
func generateQRCode(originalURL string) (string, error) {
	return "", errors.New("QR code generation disabled in small build")
}

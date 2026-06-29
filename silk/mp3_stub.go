//go:build small

package silk

import "errors"

// mp3ToPcm 小型构建: 不依赖 go-mp3 库，返回错误
func mp3ToPcm(data []byte, targetSampleRate int) ([]byte, error) {
	return nil, errors.New("MP3 decoding disabled in small build")
}

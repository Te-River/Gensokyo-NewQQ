//go:build !small

package silk

import (
	"bytes"
	"fmt"
	"io"
	"math"

	"github.com/hajimehoshi/go-mp3"
)

// mp3ToPcm 使用纯 Go 解码 MP3 为 16-bit 单声道 PCM
func mp3ToPcm(data []byte, targetSampleRate int) ([]byte, error) {
	decoder, err := mp3.NewDecoder(bytes.NewReader(data))
	if err != nil {
		return nil, fmt.Errorf("mp3 decoder init: %w", err)
	}
	srcRate := decoder.SampleRate()

	var buf bytes.Buffer
	_, err = io.CopyN(&buf, decoder, int64(math.MaxInt32)) // 限制最大约 2GB
	if err != nil && err != io.EOF {
		return nil, fmt.Errorf("mp3 decode: %w", err)
	}
	pcm := buf.Bytes() // 16-bit 立体声交错

	// MP3 输出始终为立体声
	mono := stereoToMono(pcm)
	return resamplePcm(mono, srcRate, targetSampleRate), nil
}

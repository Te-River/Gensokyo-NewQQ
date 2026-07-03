package silk

import (
	"bytes"
	"crypto/md5"
	"embed"
	"encoding/binary"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"os/exec"
	"path"
	"runtime"
	"strconv"
	"strings"

	"github.com/hoshinonyaruko/gensokyo/config"
	"github.com/hoshinonyaruko/gensokyo/mylog"
)

//go:embed exec/*
var silkCodecs embed.FS

const (
	// HeaderAmr AMR文件头
	HeaderAmr = "#!AMR"
	// HeaderSilk Silkv3文件头
	HeaderSilk = "\x02#!SILK_V3"
)

const silkCachePath = "data/cache"

const limit = 4 * 1024

func getSilkCodecPath() (string, error) {
	var codecFileName string
	switch runtime.GOOS {
	case "windows":
		switch runtime.GOARCH {
		case "amd64":
			codecFileName = "silk_codec-windows-static-x64.exe"
		case "386":
			codecFileName = "silk_codec-windows-static-x86.exe"
		default:
			return "", fmt.Errorf("unsupported architecture: %s", runtime.GOARCH)
		}
	case "linux":
		switch runtime.GOARCH {
		case "amd64":
			codecFileName = "silk_codec-linux-x64"
		case "arm64":
			codecFileName = "silk_codec-linux-arm64"
		default:
			return "", fmt.Errorf("unsupported architecture for Linux: %s", runtime.GOARCH)
		}
	case "darwin":
		switch runtime.GOARCH {
		case "amd64":
			codecFileName = "silk_codec-macos"
		case "arm64":
			codecFileName = "silk_codec-macos"
		default:
			return "", fmt.Errorf("unsupported architecture for macOS: %s", runtime.GOARCH)
		}
	case "android":
		switch runtime.GOARCH {
		case "arm64":
			codecFileName = "silk_codec-android-arm64"
		case "x86":
			codecFileName = "silk_codec-android-x86"
		case "x86_64":
			codecFileName = "silk_codec-android-x86_64"
		default:
			return "", fmt.Errorf("unsupported architecture for macOS: %s", runtime.GOARCH)
		}
	default:
		return "", fmt.Errorf("unsupported platform: %s", runtime.GOOS)
	}

	return "exec/" + codecFileName, nil
}

// EncoderSilk 将音频编码为Silk
func EncoderSilk(data []byte) []byte {
	h := md5.New()
	_, err := h.Write(data)
	if err != nil {
		mylog.Printf("calc md5 failed")
		return nil
	}
	tempName := hex.EncodeToString(h.Sum(nil))
	slk := encode(data, tempName)
	return slk
}

// EncodeMP4 将给定视频文件编码为MP4
func EncodeMP4(src string, dst string) error {
	cmd1 := exec.Command("ffmpeg", "-i", src, "-y", "-c", "copy", "-map", "0", dst)
	if errors.Is(cmd1.Err, exec.ErrDot) {
		cmd1.Err = nil
	}
	err := cmd1.Run()
	if err != nil {
		cmd2 := exec.Command("ffmpeg", "-i", src, "-y", "-c:v", "h264", "-c:a", "mp3", dst)
		if errors.Is(cmd2.Err, exec.ErrDot) {
			cmd2.Err = nil
		}
		mylog.Printf("convert mp4 failed")
		return err
	}
	return err
}

// ExtractCover 获取给定视频文件的Cover
func ExtractCover(src string, target string) error {
	cmd := exec.Command("ffmpeg", "-i", src, "-y", "-ss", "0", "-frames:v", "1", target)
	if errors.Is(cmd.Err, exec.ErrDot) {
		cmd.Err = nil
	}
	mylog.Printf("extract video cover failed")
	return nil
}

// createDirectoryIfNotExists 检查目录是否存在，如果不存在则创建它
func createDirectoryIfNotExists(directoryPath string) error {
	_, err := os.Stat(directoryPath)
	if os.IsNotExist(err) {
		// 目录不存在，创建它
		err := os.MkdirAll(directoryPath, os.ModePerm)
		if err != nil {
			return err
		}
	}
	return nil
}

// detectAudioFormat 通过魔数检测音频格式，返回格式名称
func detectAudioFormat(data []byte) string {
	if len(data) < 4 {
		return "unknown"
	}
	// WAV
	if len(data) >= 12 && bytes.Equal(data[0:4], []byte("RIFF")) && bytes.Equal(data[8:12], []byte("WAVE")) {
		return "WAV"
	}
	// MP3 (MPEG ADTS, layer III sync word)
	if data[0] == 0xFF && (data[1]&0xE0) == 0xE0 {
		return "MP3"
	}
	// MP3 with ID3v2 header (ID3 tags before MPEG data)
	if len(data) >= 3 && bytes.Equal(data[:3], []byte("ID3")) {
		return "MP3"
	}
	// OGG
	if bytes.HasPrefix(data, []byte("OggS")) {
		return "OGG"
	}
	// FLAC
	if bytes.HasPrefix(data, []byte("fLaC")) {
		return "FLAC"
	}
	// AMR
	if bytes.HasPrefix(data, []byte("#!AMR")) {
		return "AMR"
	}
	// SILK (Tencent variant)
	if bytes.HasPrefix(data, []byte("\x02#!SILK_V3")) || bytes.HasPrefix(data, []byte("#!SILK_V3")) {
		return "SILK"
	}
	// MP4/AAC
	if len(data) >= 12 && bytes.Equal(data[4:8], []byte("ftyp")) {
		return "MP4/AAC"
	}
	return fmt.Sprintf("unknown(hex:%X)", data[:min(16, len(data))])
}

// wavToPcm 直接从 WAV 数据中提取 PCM 数据，无需 ffmpeg
// 仅支持 16-bit 单声道 PCM WAV，且采样率需匹配 targetSampleRate
// 如果数据不是 WAV 或不满足条件，返回 nil
func wavToPcm(data []byte, targetSampleRate int) []byte {
	if len(data) < 44 {
		return nil
	}
	// 检查 RIFF/WAVE 头
	if !bytes.Equal(data[0:4], []byte("RIFF")) || !bytes.Equal(data[8:12], []byte("WAVE")) {
		return nil
	}

	var sampleRate, channels, bitsPerSample int
	var dataChunk []byte
	offset := 12
	for offset < len(data)-8 {
		chunkID := string(data[offset : offset+4])
		chunkSize := int(binary.LittleEndian.Uint32(data[offset+4 : offset+8]))
		if offset+8+chunkSize > len(data) {
			break
		}
		switch chunkID {
		case "fmt ":
			if chunkSize < 16 {
				return nil
			}
			audioFormat := binary.LittleEndian.Uint16(data[offset+8 : offset+10])
			if audioFormat != 1 { // 1 = PCM (未压缩)
				return nil // 非 PCM 格式需要 ffmpeg
			}
			channels = int(binary.LittleEndian.Uint16(data[offset+10 : offset+12]))
			sampleRate = int(binary.LittleEndian.Uint32(data[offset+12 : offset+16]))
			bitsPerSample = int(binary.LittleEndian.Uint16(data[offset+22 : offset+24]))
		case "data":
			dataChunk = data[offset+8 : offset+8+chunkSize]
		}
		offset += 8 + chunkSize
		if chunkSize%2 == 1 { // RIFF 对齐填充
			offset++
		}
	}

	if dataChunk == nil || sampleRate == 0 {
		return nil
	}

	// 如果采样率、声道数或位深不匹配，无法直接使用
	if sampleRate != targetSampleRate || channels != 1 || bitsPerSample != 16 {
		return nil
	}

	return dataChunk
}

// resamplePcm 对 16-bit 单声道 PCM 数据进行线性重采样
func resamplePcm(data []byte, srcRate, dstRate int) []byte {
	if srcRate == dstRate || len(data) < 2 {
		return data
	}
	srcLen := len(data) / 2
	dstLen := srcLen * dstRate / srcRate
	if dstLen < 1 {
		dstLen = 1
	}
	dst := make([]byte, dstLen*2)
	for i := 0; i < dstLen; i++ {
		srcPos := float64(i) * float64(srcRate) / float64(dstRate)
		srcIdx := int(srcPos)
		frac := srcPos - float64(srcIdx)
		if srcIdx >= srcLen-1 {
			s := binary.LittleEndian.Uint16(data[(srcLen-1)*2 : (srcLen-1)*2+2])
			binary.LittleEndian.PutUint16(dst[i*2:i*2+2], s)
		} else {
			s0 := int16(binary.LittleEndian.Uint16(data[srcIdx*2 : srcIdx*2+2]))
			s1 := int16(binary.LittleEndian.Uint16(data[(srcIdx+1)*2 : (srcIdx+1)*2+2]))
			sample := int16(float64(s0)*(1-frac) + float64(s1)*frac)
			binary.LittleEndian.PutUint16(dst[i*2:i*2+2], uint16(sample))
		}
	}
	return dst
}

// stereoToMono 将 16-bit 立体声 PCM (左右交错) 转为单声道
func stereoToMono(stereo []byte) []byte {
	if len(stereo) < 4 {
		return stereo
	}
	samples := len(stereo) / 4
	mono := make([]byte, samples*2)
	for i := 0; i < samples; i++ {
		l := int(int16(binary.LittleEndian.Uint16(stereo[i*4 : i*4+2])))
		r := int(int16(binary.LittleEndian.Uint16(stereo[i*4+2 : i*4+4])))
		m := int16((l + r) / 2)
		binary.LittleEndian.PutUint16(mono[i*2:i*2+2], uint16(m))
	}
	return mono
}

// decodeToPcm 尝试内置解码器将音频转为 PCM
func decodeToPcm(data []byte, targetSampleRate int) []byte {
	// 1. 优先 WAV 直接提取
	if pcm := wavToPcm(data, targetSampleRate); pcm != nil {
		mylog.Printf("解码: 从 WAV 直接提取 PCM")
		return pcm
	}

	fmt := detectAudioFormat(data)
	mylog.Printf("解码: 音频格式 %s", fmt)

	// 2. 尝试 MP3 解码
	if strings.HasPrefix(fmt, "MP3") {
		pcm, err := mp3ToPcm(data, targetSampleRate)
		if err != nil {
			mylog.Errorf("MP3 解码失败: %v", err)
			return nil
		}
		mylog.Printf("MP3 解码成功 (%d bytes PCM)", len(pcm))
		return pcm
	}

	mylog.Printf("解码: 格式 %s 无内置解码器，尝试 ffmpeg", fmt)
	return nil
}

// encode 将音频编码为Silk
func encode(record []byte, tempName string) (silkWav []byte) {
	// 0. 创建缓存目录
	err := createDirectoryIfNotExists(silkCachePath)
	if err != nil {
		fmt.Printf("创建语音缓存目录失败：%v\n", err)
	}

	sampleRate := config.GetRecordSampleRate() // 获取采样率
	bitRate := config.GetRecordBitRate()       // 获取比特率
	mylog.Printf("sampleRate%v", sampleRate)
	mylog.Printf("bitRate%v", bitRate)

	pcmPath := path.Join(silkCachePath, tempName+".pcm")

	// 1. 优先使用内置解码器（WAV/MP3/OGG/FLAC，无需 ffmpeg）
	pcmData := decodeToPcm(record, sampleRate)
	if pcmData != nil {
		mylog.Printf("使用内置解码器转换 PCM 成功")
		err = os.WriteFile(pcmPath, pcmData, os.ModePerm)
		if err != nil {
			mylog.Errorf("write pcm file error")
			return nil
		}
	} else {
		// 2. 回退 ffmpeg
		audioFmt := detectAudioFormat(record)
		mylog.Printf("回退: 尝试 ffmpeg 转换 %s", audioFmt)

		rawPath := path.Join(silkCachePath, tempName+".raw")
		err = os.WriteFile(rawPath, record, os.ModePerm)
		if err != nil {
			mylog.Errorf("write temp file error")
			return nil
		}
		defer os.Remove(rawPath)

		cmd := exec.Command("ffmpeg", "-i", rawPath, "-f", "s16le", "-ar", strconv.Itoa(sampleRate), "-ac", "1", pcmPath)
		if errors.Is(cmd.Err, exec.ErrDot) {
			cmd.Err = nil
		}
		if err = cmd.Run(); err != nil {
			mylog.Errorf("convert pcm file error: ffmpeg 未安装或输入格式(%s)不受支持", audioFmt)
			return nil
		}
	}
	defer os.Remove(pcmPath)

	silkPath := path.Join(silkCachePath, tempName+".silk")

	// 调用silk_codec转换为Silk

	// 获取silk_codec文件名
	codecFileName, err := getSilkCodecPath()
	if err != nil {
		mylog.Errorf("failed to get codec path: %v", err)
		return nil
	}

	// 从嵌入的文件系统中读取silk_codec二进制文件
	codecData, err := silkCodecs.ReadFile(codecFileName)
	if err != nil {
		mylog.Errorf("failed to read codec data: %v", err)
		return nil
	}

	// 根据操作系统确定临时文件的扩展名
	tempFilePattern := "silk_codec*"
	if runtime.GOOS == "windows" {
		tempFilePattern += ".exe"
	}

	// 创建临时文件
	tmpFile, err := os.CreateTemp("", tempFilePattern)
	if err != nil {
		mylog.Errorf("failed to create temp file: %v", err)
		return nil
	}
	defer os.Remove(tmpFile.Name()) // 清理临时文件

	// 写入二进制数据到临时文件
	if _, err := tmpFile.Write(codecData); err != nil {
		mylog.Errorf("failed to write codec data to temp file: %v", err)
		return nil
	}
	if err := tmpFile.Close(); err != nil {
		mylog.Errorf("failed to close temp file: %v", err)
		return nil
	}

	// 确保临时文件可执行
	if err := os.Chmod(tmpFile.Name(), 0700); err != nil {
		mylog.Errorf("failed to chmod temp file: %v", err)
		return nil
	}

	// 使用临时文件执行silk_codec
	cmd := exec.Command(tmpFile.Name(), "pts", "-i", pcmPath, "-o", silkPath, "-s", strconv.Itoa(sampleRate))
	if err := cmd.Run(); err != nil {
		mylog.Errorf("silk encode error: %v", err)
		return nil
	}

	// 读取Silk文件
	silkWav, err = os.ReadFile(silkPath)
	if err != nil {
		mylog.Errorf("read silk file error: %v", err)
		return nil
	}

	return silkWav
}

// IsAMRorSILK 判断给定文件是否为Amr或Silk格式
func IsAMRorSILK(b []byte) bool {
	return bytes.HasPrefix(b, []byte(HeaderAmr)) || bytes.HasPrefix(b, []byte(HeaderSilk))
}

// 扫描格式
func scan(r io.ReadSeeker) string {
	_, _ = r.Seek(0, io.SeekStart)
	defer r.Seek(0, io.SeekStart)
	in := make([]byte, limit)
	_, _ = r.Read(in)
	return http.DetectContentType(in)
}

// CheckAudio 判断给定流是否为合法音频
func CheckAudio(r io.ReadSeeker) (string, bool) {
	t := scan(r)
	// std mime type detection is not full supported for audio
	if strings.Contains(t, "text") || strings.Contains(t, "image") {
		return t, false
	}
	return t, true
}

// // resample 将silk重新编码为 24000 bit rate
// func resample(data []byte) []byte {
// 	pcm, err := silk.DecodeSilkBuffToPcm(data, 24000)
// 	if err != nil {
// 		panic(err)
// 	}
// 	data, err = silk.EncodePcmBuffToSilk(pcm, 24000, 24000, true)
// 	if err != nil {
// 		panic(err)
// 	}
// 	return data
// }

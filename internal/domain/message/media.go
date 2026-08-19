package message

// MediaKind 媒体来源类型。
type MediaKind uint8

const (
	// MediaLocalFile 本地文件路径。
	MediaLocalFile MediaKind = iota + 1
	// MediaRemoteURL 远程 URL。
	MediaRemoteURL
	// MediaBase64 base64 编码数据（Data 中为编码后的字符串字节）。
	MediaBase64
	// MediaBytes 原始字节。
	MediaBytes
)

// MediaSource 媒体来源的抽象描述，供 P5 媒体管线消费。
type MediaSource struct {
	Kind MediaKind

	Path string // MediaLocalFile
	URL  string // MediaRemoteURL

	Data []byte // MediaBase64 / MediaBytes
}

// MediaSourceFromFile 按 OneBot file 字段值推断媒体来源。
// 支持 base64://、http(s)://、file:// 与裸路径。
func MediaSourceFromFile(file string) MediaSource {
	switch {
	case hasPrefix(file, "base64://"):
		return MediaSource{Kind: MediaBase64, Data: []byte(file[len("base64://"):])}
	case hasPrefix(file, "http://"), hasPrefix(file, "https://"):
		return MediaSource{Kind: MediaRemoteURL, URL: file}
	case hasPrefix(file, "file://"):
		return MediaSource{Kind: MediaLocalFile, Path: file[len("file://"):]}
	default:
		return MediaSource{Kind: MediaLocalFile, Path: file}
	}
}

func hasPrefix(s, prefix string) bool {
	return len(s) >= len(prefix) && s[:len(prefix)] == prefix
}

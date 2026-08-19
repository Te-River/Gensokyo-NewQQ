package config

// RuntimeConfig 是程序运行时行为的视图，由 ConfigDTO 构建，对程序行为负责。
// 业务代码通过 Snapshot.Config() 获取，禁止直接修改。
type RuntimeConfig struct {
	Version   uint64
	QQ        QQConfig
	OneBot    OneBotConfig
	Transport TransportConfig
	IDMap     IDMapConfig
	Media     MediaConfig
}

// QQConfig QQ 机器人账号相关。
type QQConfig struct {
	AppID        uint64
	UIN          int64
	Token        string
	ClientSecret string
	ShardCount   int
	ShardID      int
	UseUin       bool
	SandBoxMode  bool
	TextIntent   []string
}

// OneBotConfig OneBot 上报/动作格式相关。
type OneBotConfig struct {
	ArrayValue      bool
	StringOb11      bool
	StringAction    bool
	NativeOb11      bool
	TwoWayEcho      bool
	NativeMD        bool
	LazyMessageId   bool
	DisableErrorChan bool
}

// TransportConfig 传输层（正向 HTTP API / 反向 WS / 正向 WS / Webhook / TLS）。
type TransportConfig struct {
	HTTPAddress    string
	HTTPTimeout    int
	WsAddress      []string
	EnableWsServer bool
	WsServerPath   string
	WsServerToken  string
	WebhookPath    string
	PostUrls       []string
	UseSelfCrt     bool
	Crt            string
	Key            string
}

// IDMapConfig 虚拟 ID 映射相关。
type IDMapConfig struct {
	Isolation    bool
	LegacyCompat bool
	Grpc         bool
	GrpcPort     int
}

// MediaConfig 媒体与图床相关（不承载云凭据，凭据由语义校验基于 DTO 检查）。
type MediaConfig struct {
	OssType          int
	ImageLimit       int
	ImageLimitB      int
	UrlPicTransfer   bool
	UploadPicV2Base64 bool
}

// clone 返回深拷贝：slice 字段重新分配底层数组，避免外部通过返回的
// RuntimeConfig 篡改快照内部状态。
func (r RuntimeConfig) clone() RuntimeConfig {
	r.QQ.TextIntent = append([]string(nil), r.QQ.TextIntent...)
	r.Transport.WsAddress = append([]string(nil), r.Transport.WsAddress...)
	r.Transport.PostUrls = append([]string(nil), r.Transport.PostUrls...)
	return r
}

// buildRuntime 从 DTO 构建 RuntimeConfig。
// slice 字段做防御性拷贝，保证 Snapshot 不可变。
func buildRuntime(dto ConfigDTO) RuntimeConfig {
	s := dto.Settings
	return RuntimeConfig{
		Version: uint64(dto.Version),
		QQ: QQConfig{
			AppID:        s.AppID,
			UIN:          s.Uin,
			Token:        s.Token,
			ClientSecret: s.ClientSecret,
			ShardCount:   s.ShardCount,
			ShardID:      s.ShardID,
			UseUin:       s.UseUin,
			SandBoxMode:  s.SandBoxMode,
			TextIntent:   append([]string(nil), s.TextIntent...),
		},
		OneBot: OneBotConfig{
			ArrayValue:       s.Array,
			StringOb11:       s.StringOb11,
			StringAction:     s.StringAction,
			NativeOb11:       s.NativeOb11,
			TwoWayEcho:       s.TwoWayEcho,
			NativeMD:         s.NativeMD,
			LazyMessageId:    s.LazyMessageId,
			DisableErrorChan: s.DisableErrorChan,
		},
		Transport: TransportConfig{
			HTTPAddress:    s.HttpAddress,
			HTTPTimeout:    s.HttpTimeOut,
			WsAddress:      append([]string(nil), s.WsAddress...),
			EnableWsServer: s.EnableWsServer,
			WsServerPath:   s.WsServerPath,
			WsServerToken:  s.WsServerToken,
			WebhookPath:    s.WebhookPath,
			PostUrls:       append([]string(nil), s.PostUrl...),
			UseSelfCrt:     s.UseSelfCrt,
			Crt:            s.Crt,
			Key:            s.Key,
		},
		IDMap: IDMapConfig{
			Isolation:    s.IdmapIsolation,
			LegacyCompat: s.IdmapLegacyCompat,
			Grpc:         s.LotusGrpc,
			GrpcPort:     s.LotusGrpcPort,
		},
		Media: MediaConfig{
			OssType:          s.OssType,
			ImageLimit:       s.ImageLimit,
			ImageLimitB:      s.ImageLimitB,
			UrlPicTransfer:   s.UrlPicTransfer,
			UploadPicV2Base64: s.UploadPicV2Base64,
		},
	}
}

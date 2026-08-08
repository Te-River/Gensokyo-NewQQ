package config

import (
	"fmt"
	"net"
	"net/url"
	"os"
	"strings"
)

// Validate 依次执行 schema 与语义校验。
func Validate(dto ConfigDTO) error {
	if err := ValidateSchema(dto); err != nil {
		return err
	}
	return ValidateSemantic(dto)
}

// ValidateSchema 校验静态约束：必填、枚举、范围、URL/地址格式、超时等。
func ValidateSchema(dto ConfigDTO) error {
	s := dto.Settings

	if s.AppID == 0 {
		return newValidationError("config.qq.app_id", "must not be empty")
	}

	if s.HttpAddress != "" && !validAddress(s.HttpAddress) {
		return newValidationError("config.transport.http_address",
			fmt.Sprintf("invalid address %q (want host:port or URL)", s.HttpAddress))
	}
	for i, addr := range s.WsAddress {
		if !strings.HasPrefix(addr, "ws://") && !strings.HasPrefix(addr, "wss://") {
			return newValidationError(fmt.Sprintf("config.transport.ws_address[%d]", i),
				fmt.Sprintf("must be ws:// or wss:// URL, got %q", addr))
		}
	}
	for i, u := range s.PostUrl {
		if !validHTTPURL(u) {
			return newValidationError(fmt.Sprintf("config.transport.post_url[%d]", i),
				fmt.Sprintf("invalid http(s) URL %q", u))
		}
	}
	if s.HttpTimeOut < 0 {
		return newValidationError("config.transport.http_timeout", "must be >= 0")
	}

	if s.LotusGrpcPort < 0 || s.LotusGrpcPort > 65535 {
		return newValidationError("config.idmap.grpc_port", "must be between 1 and 65535")
	}

	if s.OssType < 0 || s.OssType > 10 {
		return newValidationError("config.media.oss_type", fmt.Sprintf("must be between 0 and 10, got %d", s.OssType))
	}
	if s.ImageLimit < 0 || s.ImageLimitB < 0 {
		return newValidationError("config.media.image_limit", "must be >= 0")
	}

	return nil
}

// ValidateSemantic 校验 schema 正确但存在依赖/资源冲突的配置。
// 例如 TLS 开启但证书缺失、图床开启但凭据缺失、Lotus 开启但 endpoint 缺失。
func ValidateSemantic(dto ConfigDTO) error {
	s := dto.Settings

	// TLS 开启但证书/密钥不存在
	if s.UseSelfCrt {
		if s.Crt == "" || s.Key == "" {
			return newValidationError("config.transport.tls", "enabled but crt/key not set")
		}
		if _, err := os.Stat(s.Crt); err != nil {
			return newValidationError("config.transport.tls.crt", fmt.Sprintf("file not accessible: %v", err))
		}
		if _, err := os.Stat(s.Key); err != nil {
			return newValidationError("config.transport.tls.key", fmt.Sprintf("file not accessible: %v", err))
		}
	}

	// 图床 provider 开启但凭据缺失
	// 注：nature（oss_type=10）使用公开共享凭据，开箱即用，无需校验。
	switch s.OssType {
	case 4: // cos 自签
		if s.COS.SecretID == "" || s.COS.SecretKey == "" {
			return newValidationError("config.media.image_provider",
				"cos enabled but secret_id/secret_key missing")
		}
	}

	// Lotus 模式开启但 endpoint（server_dir:port）不存在
	if s.Lotus {
		if s.Server_dir == "" || s.Port == "" {
			return newValidationError("config.lotus.endpoint",
				"lotus enabled but server_dir/port not set")
		}
	}

	return nil
}

func validAddress(addr string) bool {
	if _, _, err := net.SplitHostPort(addr); err == nil {
		return true
	}
	u, err := url.Parse(addr)
	return err == nil && u.Scheme != "" && u.Host != ""
}

func validHTTPURL(raw string) bool {
	u, err := url.Parse(raw)
	if err != nil {
		return false
	}
	return (u.Scheme == "http" || u.Scheme == "https") && u.Host != ""
}

// Package securityaudit inspects deployment configuration for high-risk network defaults.
package securityaudit

import (
	"bufio"
	"bytes"
	"fmt"
	"net"
	"os"
	"strconv"
	"strings"
)

const maxAuditConfigBytes = 4 << 20

// Severity represents the operational impact of a finding.
type Severity string

const (
	SeverityWarning Severity = "warning"
	SeverityHigh    Severity = "high"
)

// Finding describes one insecure or ambiguous deployment setting.
type Finding struct {
	Severity Severity
	Code     string
	Message  string
}

// Report is the result of auditing one config.yml file.
type Report struct {
	Findings []Finding
}

func (r Report) HasHighRisk() bool {
	for _, finding := range r.Findings {
		if finding.Severity == SeverityHigh {
			return true
		}
	}
	return false
}

type securitySettings struct {
	Port              string
	ForceSSL          bool
	EnableWSServer    bool
	WSServerToken     string
	HTTPAddress       string
	HTTPAccessToken   string
	DisableWebUI      bool
	WebUIUsername     string
	WebUIPassword     string
	ChatGLMEnabled    bool
	UkakaEnabled      bool
	XingyeEnabled     bool
	NatureEnabled     bool
}

// AuditFile reads and audits a Gensokyo config file.
func AuditFile(path string) (Report, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return Report{}, err
	}
	return AuditYAML(data)
}

// AuditYAML audits config bytes without modifying them.
//
// The audit intentionally parses only the scalar keys it needs. This keeps the
// startup guard independent from the application's YAML loader and prevents a
// malformed optional section from silently disabling the audit.
func AuditYAML(data []byte) (Report, error) {
	settings, err := parseSecuritySettings(data)
	if err != nil {
		return Report{}, err
	}

	report := Report{}
	add := func(severity Severity, code, message string) {
		report.Findings = append(report.Findings, Finding{Severity: severity, Code: code, Message: message})
	}

	if settings.EnableWSServer && strings.TrimSpace(settings.WSServerToken) == "" {
		add(SeverityHigh, "ws-empty-token", "正向 WebSocket 监听在全部网络接口，但 ws_server_token 为空")
	}

	if address := strings.TrimSpace(settings.HTTPAddress); address != "" {
		if strings.TrimSpace(settings.HTTPAccessToken) == "" {
			if IsLoopbackAddress(address) {
				add(SeverityWarning, "http-api-loopback-empty-token", "HTTP API 仅监听本机但未配置 http_access_token")
			} else {
				add(SeverityHigh, "http-api-public-empty-token", "HTTP API 可能对外监听，但 http_access_token 为空")
			}
		} else {
			add(SeverityWarning, "http-api-query-token", "HTTP API 已配置令牌；调用方应只使用 Authorization: Bearer，避免把令牌放入 URL 查询参数")
		}
	}

	if !settings.DisableWebUI {
		username := strings.TrimSpace(settings.WebUIUsername)
		password := settings.WebUIPassword
		if username == "" || password == "" {
			add(SeverityHigh, "webui-empty-credentials", "WebUI 已启用，但用户名或密码为空")
		} else if isKnownDefaultCredential(username, password) {
			add(SeverityHigh, "webui-default-credentials", "WebUI 仍在使用模板默认凭据，请立即修改")
		} else if len([]rune(password)) < 12 {
			add(SeverityWarning, "webui-weak-password", "WebUI 密码少于 12 个字符")
		}
		if !settings.ForceSSL && strings.TrimSpace(settings.Port) != "443" {
			add(SeverityWarning, "webui-plaintext", "WebUI 主服务未启用 HTTPS；不要直接暴露到不可信网络")
		}
	}

	thirdPartyConfigured := settings.ChatGLMEnabled || settings.UkakaEnabled || settings.XingyeEnabled
	if thirdPartyConfigured && !ThirdPartyImageHostsOptedIn() {
		add(SeverityWarning, "third-party-image-hosts-gated", "配置中启用了第三方图床，但运行时显式授权未开启；图片不会上传到这些后端")
	}
	if settings.NatureEnabled {
		add(SeverityWarning, "nature-disabled", "Nature 图床配置仍为 enabled，但该后端已因公开凭据问题永久禁用")
	}

	return report, nil
}

func parseSecuritySettings(data []byte) (securitySettings, error) {
	if len(data) == 0 {
		return securitySettings{}, fmt.Errorf("security audit: config is empty")
	}
	if len(data) > maxAuditConfigBytes {
		return securitySettings{}, fmt.Errorf("security audit: config exceeds %d MiB", maxAuditConfigBytes>>20)
	}

	values, blocks, err := parseScalarYAML(data)
	if err != nil {
		return securitySettings{}, fmt.Errorf("security audit: %w", err)
	}
	if !blocks["settings"] {
		return securitySettings{}, fmt.Errorf("security audit: missing settings block")
	}

	get := func(path string) string { return values[path] }
	getBool := func(path string) bool {
		value, _ := strconv.ParseBool(strings.TrimSpace(get(path)))
		return value
	}

	return securitySettings{
		Port:            get("settings.port"),
		ForceSSL:        getBool("settings.force_ssl"),
		EnableWSServer:  getBool("settings.enable_ws_server"),
		WSServerToken:   get("settings.ws_server_token"),
		HTTPAddress:     get("settings.http_address"),
		HTTPAccessToken: get("settings.http_access_token"),
		DisableWebUI:    getBool("settings.disable_webui"),
		WebUIUsername:   get("settings.server_user_name"),
		WebUIPassword:   get("settings.server_user_password"),
		ChatGLMEnabled:  getBool("settings.image_hosting.chatglm.enabled"),
		UkakaEnabled:    getBool("settings.image_hosting.ukaka.enabled"),
		XingyeEnabled:   getBool("settings.image_hosting.xingye.enabled"),
		NatureEnabled:   getBool("settings.image_hosting.nature.enabled"),
	}, nil
}

type yamlStackEntry struct {
	indent int
	key    string
}

// parseScalarYAML is a conservative path-aware parser for block-style scalar
// configuration. It does not attempt to implement the full YAML specification.
func parseScalarYAML(data []byte) (map[string]string, map[string]bool, error) {
	values := make(map[string]string)
	blocks := make(map[string]bool)
	stack := make([]yamlStackEntry, 0, 8)

	scanner := bufio.NewScanner(bytes.NewReader(data))
	scanner.Buffer(make([]byte, 4096), maxAuditConfigBytes)
	lineNumber := 0
	for scanner.Scan() {
		lineNumber++
		line := strings.TrimSuffix(scanner.Text(), "\r")
		line = stripYAMLComment(line)
		if strings.TrimSpace(line) == "" {
			continue
		}

		indent := leadingIndent(line)
		trimmed := strings.TrimSpace(line)
		colon := strings.IndexByte(trimmed, ':')
		if colon <= 0 {
			continue
		}
		key := strings.TrimSpace(trimmed[:colon])
		if key == "" || strings.HasPrefix(key, "-") {
			continue
		}

		for len(stack) > 0 && stack[len(stack)-1].indent >= indent {
			stack = stack[:len(stack)-1]
		}
		pathParts := make([]string, 0, len(stack)+1)
		for _, entry := range stack {
			pathParts = append(pathParts, entry.key)
		}
		pathParts = append(pathParts, key)
		path := strings.Join(pathParts, ".")

		rawValue := strings.TrimSpace(trimmed[colon+1:])
		if rawValue == "" {
			blocks[path] = true
			stack = append(stack, yamlStackEntry{indent: indent, key: key})
			continue
		}

		value, err := decodeYAMLScalar(rawValue)
		if err != nil {
			return nil, nil, fmt.Errorf("line %d key %s: %w", lineNumber, path, err)
		}
		values[path] = value
	}
	if err := scanner.Err(); err != nil {
		return nil, nil, err
	}
	return values, blocks, nil
}

func stripYAMLComment(line string) string {
	var singleQuoted, doubleQuoted, escaped bool
	for index, char := range line {
		if escaped {
			escaped = false
			continue
		}
		if char == '\\' && doubleQuoted {
			escaped = true
			continue
		}
		switch char {
		case '\'':
			if !doubleQuoted {
				singleQuoted = !singleQuoted
			}
		case '"':
			if !singleQuoted {
				doubleQuoted = !doubleQuoted
			}
		case '#':
			if !singleQuoted && !doubleQuoted {
				return line[:index]
			}
		}
	}
	return line
}

func leadingIndent(line string) int {
	indent := 0
	for _, char := range line {
		switch char {
		case ' ':
			indent++
		case '\t':
			indent += 2
		default:
			return indent
		}
	}
	return indent
}

func decodeYAMLScalar(value string) (string, error) {
	value = strings.TrimSpace(value)
	if len(value) >= 2 && value[0] == '"' && value[len(value)-1] == '"' {
		decoded, err := strconv.Unquote(value)
		if err != nil {
			return "", err
		}
		return decoded, nil
	}
	if len(value) >= 2 && value[0] == '\'' && value[len(value)-1] == '\'' {
		return strings.ReplaceAll(value[1:len(value)-1], "''", "'"), nil
	}
	return strings.TrimSpace(value), nil
}

// StrictModeEnabled reports whether startup should fail on high-risk findings.
func StrictModeEnabled() bool {
	return parseBoolEnv("GENSOKYO_STRICT_SECURITY")
}

// ThirdPartyImageHostsOptedIn mirrors the image-host runtime opt-in.
func ThirdPartyImageHostsOptedIn() bool {
	return parseBoolEnv("GENSOKYO_ENABLE_THIRD_PARTY_IMAGE_HOSTS")
}

func parseBoolEnv(name string) bool {
	value := strings.TrimSpace(os.Getenv(name))
	if parsed, err := strconv.ParseBool(value); err == nil {
		return parsed
	}
	return strings.EqualFold(value, "yes") || strings.EqualFold(value, "on")
}

func isKnownDefaultCredential(username, password string) bool {
	username = strings.ToLower(strings.TrimSpace(username))
	return (username == "useradmin" && password == "admin") ||
		(username == "admin" && password == "admin")
}

// IsLoopbackAddress determines whether an HTTP listen address is constrained to localhost.
func IsLoopbackAddress(address string) bool {
	address = strings.TrimSpace(address)
	if address == "" {
		return false
	}

	host, _, err := net.SplitHostPort(address)
	if err != nil {
		// Accept a bare host only for audit purposes.
		host = address
	}
	host = strings.Trim(strings.TrimSpace(host), "[]")
	if strings.EqualFold(host, "localhost") {
		return true
	}
	ip := net.ParseIP(host)
	return ip != nil && ip.IsLoopback()
}

package securityaudit

import (
	"strings"
	"testing"
)

func TestIsLoopbackAddress(t *testing.T) {
	tests := map[string]bool{
		"127.0.0.1:5700":   true,
		"localhost:5700":   true,
		"[::1]:5700":       true,
		"0.0.0.0:5700":     false,
		":5700":            false,
		"[::]:5700":        false,
		"192.168.1.2:5700": false,
	}
	for address, want := range tests {
		if got := IsLoopbackAddress(address); got != want {
			t.Fatalf("IsLoopbackAddress(%q) = %v, want %v", address, got, want)
		}
	}
}

func TestAuditYAMLFindsHighRiskDefaults(t *testing.T) {
	t.Setenv("GENSOKYO_ENABLE_THIRD_PARTY_IMAGE_HOSTS", "")
	report, err := AuditYAML([]byte(`
version: 1
settings:
  port: "15630"
  force_ssl: false
  enable_ws_server: true
  ws_server_token: ""
  http_address: "0.0.0.0:5700"
  http_access_token: ""
  disable_webui: false
  server_user_name: "useradmin"
  server_user_password: "admin"
  image_hosting:
    chatglm:
      enabled: true
    ukaka:
      enabled: true
    xingye:
      enabled: true
    nature:
      enabled: true
`))
	if err != nil {
		t.Fatalf("AuditYAML failed: %v", err)
	}
	if !report.HasHighRisk() {
		t.Fatal("expected high-risk findings")
	}

	codes := make(map[string]bool)
	for _, finding := range report.Findings {
		codes[finding.Code] = true
	}
	for _, expected := range []string{
		"ws-empty-token",
		"http-api-public-empty-token",
		"webui-default-credentials",
		"webui-plaintext",
		"third-party-image-hosts-gated",
		"nature-disabled",
	} {
		if !codes[expected] {
			t.Fatalf("missing finding %q: %#v", expected, report.Findings)
		}
	}
}

func TestAuditYAMLAcceptsHardenedConfig(t *testing.T) {
	t.Setenv("GENSOKYO_ENABLE_THIRD_PARTY_IMAGE_HOSTS", "")
	report, err := AuditYAML([]byte(`
version: 1
settings:
  port: "443"
  force_ssl: true
  enable_ws_server: true
  ws_server_token: "a-long-random-websocket-token"
  http_address: "127.0.0.1:5700"
  http_access_token: "a-long-random-http-token"
  disable_webui: false
  server_user_name: "operator"
  server_user_password: "a-long-random-password"
  image_hosting:
    chatglm:
      enabled: false
    ukaka:
      enabled: false
    xingye:
      enabled: false
    nature:
      enabled: false
`))
	if err != nil {
		t.Fatalf("AuditYAML failed: %v", err)
	}
	if report.HasHighRisk() {
		t.Fatalf("unexpected high-risk finding: %#v", report.Findings)
	}
	for _, finding := range report.Findings {
		if !strings.Contains(finding.Code, "query-token") {
			t.Fatalf("unexpected finding: %#v", finding)
		}
	}
}

func TestParserPreservesQuotedHashAndNestedPaths(t *testing.T) {
	values, blocks, err := parseScalarYAML([]byte(`
settings:
  server_user_password: "long#password-value" # trailing comment
  image_hosting:
    chatglm:
      enabled: true
    ukaka:
      enabled: false
`))
	if err != nil {
		t.Fatalf("parseScalarYAML failed: %v", err)
	}
	if !blocks["settings.image_hosting"] {
		t.Fatal("image_hosting block not detected")
	}
	if got := values["settings.server_user_password"]; got != "long#password-value" {
		t.Fatalf("quoted hash value = %q", got)
	}
	if got := values["settings.image_hosting.chatglm.enabled"]; got != "true" {
		t.Fatalf("chatglm enabled = %q", got)
	}
	if got := values["settings.image_hosting.ukaka.enabled"]; got != "false" {
		t.Fatalf("ukaka enabled = %q", got)
	}
}

func TestAuditYAMLRejectsInvalidEnvelope(t *testing.T) {
	if _, err := AuditYAML(nil); err == nil {
		t.Fatal("empty config was accepted")
	}
	if _, err := AuditYAML([]byte("version: 1\nport: 15630\n")); err == nil {
		t.Fatal("config without settings block was accepted")
	}
	if _, err := AuditYAML(make([]byte, maxAuditConfigBytes+1)); err == nil {
		t.Fatal("oversized config was accepted")
	}
}

func TestStrictModeEnabled(t *testing.T) {
	t.Setenv("GENSOKYO_STRICT_SECURITY", "true")
	if !StrictModeEnabled() {
		t.Fatal("strict mode should be enabled")
	}
	t.Setenv("GENSOKYO_STRICT_SECURITY", "0")
	if StrictModeEnabled() {
		t.Fatal("strict mode should be disabled")
	}
}

package token

import (
	"strings"
	"testing"
)

func TestParseAccessTokenResponse(t *testing.T) {
	tests := []struct {
		name      string
		body      string
		wantToken string
		wantTTL   int64
		wantErr   string
	}{
		{
			name:      "扁平结构 int expires_in",
			body:      `{"access_token":"flat-token-abc","expires_in":7200}`,
			wantToken: "flat-token-abc",
			wantTTL:   7200,
		},
		{
			name:      "扁平结构 string expires_in",
			body:      `{"access_token":"flat-token-str","expires_in":"3600"}`,
			wantToken: "flat-token-str",
			wantTTL:   3600,
		},
		{
			name:      "data 信封结构",
			body:      `{"code":0,"message":"ok","data":{"access_token":"envelope-token","expires_in":1800}}`,
			wantToken: "envelope-token",
			wantTTL:   1800,
		},
		{
			name:      "缺失 expires_in 使用默认 7200",
			body:      `{"access_token":"no-ttl-token"}`,
			wantToken: "no-ttl-token",
			wantTTL:   7200,
		},
		{
			name:    "错误包络",
			body:    `{"code":10001,"message":"invalid clientSecret","data":null}`,
			wantErr: "invalid clientSecret",
		},
		{
			name:    "缺少 access_token",
			body:    `{"code":0,"message":"ok","data":{}}`,
			wantErr: "no access_token",
		},
		{
			name:    "非法 JSON",
			body:    `not-json`,
			wantErr: "parse access token response failed",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			info, err := parseAccessTokenResponse([]byte(tt.body))
			if tt.wantErr != "" {
				if err == nil {
					t.Fatalf("expected error containing %q, got nil", tt.wantErr)
				}
				if !strings.Contains(err.Error(), tt.wantErr) {
					t.Fatalf("expected error containing %q, got: %v", tt.wantErr, err)
				}
				return
			}
			if err != nil {
				t.Fatalf("unexpected error: %v", err)
			}
			if info.Token != tt.wantToken {
				t.Fatalf("token mismatch: got %q want %q", info.Token, tt.wantToken)
			}
			if info.ExpiresIn != tt.wantTTL {
				t.Fatalf("expires_in mismatch: got %d want %d", info.ExpiresIn, tt.wantTTL)
			}
		})
	}
}

func TestTruncateBody(t *testing.T) {
	long := strings.Repeat("a", 500)
	got := truncateBody([]byte(long))
	if len(got) != 203 { // 200 + "..."
		t.Fatalf("truncate length mismatch: got %d want %d", len(got), 203)
	}
	if got = truncateBody([]byte("short")); got != "short" {
		t.Fatalf("short body should be unchanged, got: %s", got)
	}
}

package client

import (
	"strings"
	"testing"
)

func TestRedactTokenInJSON(t *testing.T) {
	input := `{"op":2,"d":{"token":"QQBot yFAKLLDZY_secretsecretsecret","intents":1,"shard":[0,1]}}`
	got := redactTokenInJSON(input)
	if strings.Contains(got, "yFAKLLDZY_secretsecretsecret") {
		t.Fatalf("token should be redacted, got: %s", got)
	}
	if !strings.Contains(got, `"token":"QQBo****"`) {
		t.Fatalf("token prefix should be kept for debuggability, got: %s", got)
	}
}

func TestRedactTokenInJSONKeepsNonToken(t *testing.T) {
	input := `{"op":1,"d":{"intents":1107300352,"shard":[0,1],"properties":{}}}`
	got := redactTokenInJSON(input)
	if got != input {
		t.Fatalf("payload without token should be unchanged, got: %s", got)
	}
}

func TestRedactToken(t *testing.T) {
	if got := redactToken("abcd"); got != "****" {
		t.Fatalf("short token should be fully redacted, got: %s", got)
	}
	if got := redactToken("abcdefgh"); got != "abcd****" {
		t.Fatalf("token should keep first 4 chars, got: %s", got)
	}
}

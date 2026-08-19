package inbound

import (
	"testing"

	"github.com/hoshinonyaruko/gensokyo/internal/domain/message"
)

func TestIsSelfMention(t *testing.T) {
	if !IsSelfMention("SELF_OPENID", "SELF_OPENID", "12345") {
		t.Fatal("self openid not matched")
	}
	if !IsSelfMention("12345", "SELF_OPENID", "12345") {
		t.Fatal("appid string not matched")
	}
	if IsSelfMention("other", "SELF_OPENID") {
		t.Fatal("non-self matched")
	}
	if IsSelfMention("", "SELF_OPENID") {
		t.Fatal("empty matched")
	}
}

func TestNormalizeMentions(t *testing.T) {
	mentions := []message.MentionPart{
		{User: "SELF_OPENID"},
		{User: "12345"},
		{User: "other_user"},
	}
	got := NormalizeMentions("CANONICAL_SELF", mentions, "SELF_OPENID", "12345")
	if got[0].User != "CANONICAL_SELF" {
		t.Fatalf("self openid not normalized: %+v", got[0])
	}
	if got[1].User != "CANONICAL_SELF" {
		t.Fatalf("appid not normalized: %+v", got[1])
	}
	if got[2].User != "other_user" {
		t.Fatalf("non-self mutated: %+v", got[2])
	}
	if len(got) != 3 {
		t.Fatalf("len = %d", len(got))
	}
}

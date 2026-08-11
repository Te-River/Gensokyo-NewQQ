package config

import (
	"errors"
	"os"
	"path/filepath"
	"testing"
)

func readFixture(t *testing.T, name string) []byte {
	t.Helper()
	data, err := os.ReadFile(filepath.Join("testdata", "config", name))
	if err != nil {
		t.Fatalf("read fixture %s: %v", name, err)
	}
	return data
}

func TestParseValid(t *testing.T) {
	dto, err := Parse(readFixture(t, "v1-basic.yml"))
	if err != nil {
		t.Fatalf("Parse: %v", err)
	}
	if dto.Version != CurrentSchemaVersion {
		t.Fatalf("version = %d, want %d", dto.Version, CurrentSchemaVersion)
	}
	if dto.Settings.AppID != 12345 {
		t.Fatalf("app_id = %d, want 12345", dto.Settings.AppID)
	}
}

func TestParseMalformed(t *testing.T) {
	_, err := Parse(readFixture(t, "malformed.yml"))
	if err == nil {
		t.Fatal("Parse accepted malformed YAML")
	}
	var ce *Error
	if !errors.As(err, &ce) || ce.Kind != KindParse {
		t.Fatalf("err = %v, want KindParse", err)
	}
}

func TestParseUnknownFieldsTolerated(t *testing.T) {
	// 旧/未来配置可能含未知字段，必须容忍（向后兼容），不能拒绝加载
	dto, err := Parse(readFixture(t, "unknown-fields.yml"))
	if err != nil {
		t.Fatalf("Parse with unknown fields: %v", err)
	}
	if dto.Settings.AppID != 12345 {
		t.Fatalf("app_id = %d, want 12345", dto.Settings.AppID)
	}
}

func TestLoadFromFile(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "config.yml")
	if err := os.WriteFile(path, readFixture(t, "v1-basic.yml"), 0600); err != nil {
		t.Fatal(err)
	}
	dto, err := Load(path)
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	if dto.Settings.AppID != 12345 {
		t.Fatalf("app_id = %d, want 12345", dto.Settings.AppID)
	}
}

func TestLoadMissingFile(t *testing.T) {
	_, err := Load(filepath.Join(t.TempDir(), "nope.yml"))
	if err == nil {
		t.Fatal("Load accepted missing file")
	}
	var ce *Error
	if !errors.As(err, &ce) || ce.Kind != KindIO {
		t.Fatalf("err = %v, want KindIO", err)
	}
}

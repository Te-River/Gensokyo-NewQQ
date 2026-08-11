package config

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestAtomicWriteBackup(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "config.yml")
	if err := os.WriteFile(path, []byte("old-content"), 0600); err != nil {
		t.Fatal(err)
	}

	data := readFixture(t, "v1-basic.yml")
	if err := AtomicWrite(path, data); err != nil {
		t.Fatalf("AtomicWrite: %v", err)
	}

	got, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read written file: %v", err)
	}
	if string(got) != string(data) {
		t.Fatal("written content mismatch")
	}

	bak, err := os.ReadFile(path + ".bak")
	if err != nil {
		t.Fatalf("backup missing: %v", err)
	}
	if string(bak) != "old-content" {
		t.Fatalf("backup content = %q, want old-content", bak)
	}

	if _, err := os.Stat(path + ".tmp"); !os.IsNotExist(err) {
		t.Fatal("tmp file left behind after successful write")
	}
}

func TestAtomicWriteRefusesUnparsable(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "config.yml")
	original := "version: 1\nsettings:\n  app_id: 12345\n"
	if err := os.WriteFile(path, []byte(original), 0600); err != nil {
		t.Fatal(err)
	}

	bad := []byte("version: 1\nsettings: [broken")
	if err := AtomicWrite(path, bad); err == nil {
		t.Fatal("AtomicWrite accepted unparsable content")
	}

	got, _ := os.ReadFile(path)
	if string(got) != original {
		t.Fatalf("original config overwritten by bad write: %q", got)
	}
	if _, err := os.Stat(path + ".tmp"); !os.IsNotExist(err) {
		t.Fatal("tmp file left behind after refused write")
	}
}

func TestAtomicWriteNoOriginal(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "config.yml")

	data := readFixture(t, "v1-basic.yml")
	if err := AtomicWrite(path, data); err != nil {
		t.Fatalf("AtomicWrite: %v", err)
	}
	got, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read written file: %v", err)
	}
	if string(got) != string(data) {
		t.Fatal("content mismatch")
	}
	if !strings.Contains(string(got), "app_id: 12345") {
		t.Fatalf("written content missing app_id: %s", got)
	}
}

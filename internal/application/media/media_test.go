package media

import (
	"bytes"
	"image"
	"image/png"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/hoshinonyaruko/gensokyo/internal/domain/message"
)

func makePNG(t *testing.T, w, h int) []byte {
	t.Helper()
	img := image.NewRGBA(image.Rect(0, 0, w, h))
	var buf bytes.Buffer
	if err := png.Encode(&buf, img); err != nil {
		t.Fatal(err)
	}
	return buf.Bytes()
}

func TestPrepareBase64RejectsOversizeEncoded(t *testing.T) {
	policy := DefaultPolicy()
	policy.MaxEncodedBytes = 16
	_, err := prepareBase64(strings.Repeat("A", 64), policy)
	if err == nil || !strings.Contains(err.Error(), "base64 length") {
		t.Fatalf("err = %v", err)
	}
}

func TestPrepareBase64RejectsBadData(t *testing.T) {
	_, err := prepareBase64("!!!not-base64!!!", DefaultPolicy())
	if err == nil {
		t.Fatal("accepted invalid base64")
	}
}

func TestPrepareBase64RejectsOversizeDecoded(t *testing.T) {
	policy := DefaultPolicy()
	policy.MaxEncodedBytes = 1 << 20
	policy.MaxDecodedBytes = 8

	// "ABCD" = 4 字节，未超限
	if _, err := prepareBase64("QUJDRA==", policy); err != nil {
		t.Fatalf("4-byte base64 rejected: %v", err)
	}
	// "ABCDEFGHIJ" = 10 字节，超限
	if _, err := prepareBase64("QUJDREVGR0lK", policy); err == nil {
		t.Fatal("accepted 10-byte decoded content over limit 8")
	}
}

func TestPrepareBase64OK(t *testing.T) {
	p, err := prepareBase64("QUJD", DefaultPolicy())
	if err != nil {
		t.Fatal(err)
	}
	if string(p.Data) != "ABC" {
		t.Fatalf("data = %q", p.Data)
	}
}

func TestValidateImageRejectsOversizePixels(t *testing.T) {
	policy := DefaultPolicy()
	policy.MaxPixels = 1000
	data := makePNG(t, 100, 100) // 10000 pixels
	if _, _, err := ValidateImage(data, policy); err == nil {
		t.Fatal("accepted oversize pixels")
	}
}

func TestValidateImageOK(t *testing.T) {
	if _, _, err := ValidateImage(makePNG(t, 10, 10), DefaultPolicy()); err != nil {
		t.Fatalf("valid image rejected: %v", err)
	}
}

func TestPrepareLocalFileOutsideAllowedDir(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "a.png")
	os.WriteFile(path, []byte("x"), 0600)

	policy := DefaultPolicy()
	policy.AllowedDirs = []string{filepath.Join(t.TempDir())}
	if _, err := prepareLocalFile(path, policy); err == nil {
		t.Fatal("accepted file outside allowed dirs")
	}
}

func TestPrepareLocalFileNotRegular(t *testing.T) {
	dir := t.TempDir()
	if _, err := prepareLocalFile(dir, DefaultPolicy()); err == nil {
		t.Fatal("accepted directory as file")
	}
}

func TestPrepareLocalFileDisallowedExtension(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "a.exe")
	os.WriteFile(path, []byte("x"), 0600)

	policy := DefaultPolicy()
	policy.AllowedExtensions = []string{".png"}
	if _, err := prepareLocalFile(path, policy); err == nil {
		t.Fatal("accepted disallowed extension")
	}
}

func TestServicePrepareLocalFile(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "a.png")
	os.WriteFile(path, []byte("png-data"), 0600)

	s := NewService(NewFetcher(FetcherOptions{}))
	p, err := s.Prepare(t.Context(), message.MediaSource{Kind: message.MediaLocalFile, Path: path}, DefaultPolicy())
	if err != nil {
		t.Fatalf("Prepare: %v", err)
	}
	defer p.Close()
	if string(p.Data) != "png-data" {
		t.Fatalf("data = %q", p.Data)
	}
}

func TestPreparedCloseIdempotent(t *testing.T) {
	p := newPreparedFromTempFile(filepath.Join(t.TempDir(), "x"), "application/octet-stream", 0)
	if err := p.Close(); err != nil {
		t.Fatal(err)
	}
	if err := p.Close(); err != nil {
		t.Fatalf("second Close: %v", err)
	}
}

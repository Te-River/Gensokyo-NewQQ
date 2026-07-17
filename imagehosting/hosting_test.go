package imagehosting

import (
	"bytes"
	"image"
	"image/color"
	"image/png"
	"io"
	"net/http"
	"strings"
	"testing"
)

func makeTestPNG(t *testing.T) []byte {
	t.Helper()
	img := image.NewRGBA(image.Rect(0, 0, 2, 2))
	img.Set(0, 0, color.RGBA{R: 255, A: 255})

	var buffer bytes.Buffer
	if err := png.Encode(&buffer, img); err != nil {
		t.Fatalf("encode test png: %v", err)
	}
	return buffer.Bytes()
}

func TestValidateImageData(t *testing.T) {
	if err := validateImageData(makeTestPNG(t)); err != nil {
		t.Fatalf("valid PNG rejected: %v", err)
	}
	if err := validateImageData(nil); err == nil {
		t.Fatal("empty image data was accepted")
	}
	if err := validateImageData([]byte("not an image")); err == nil {
		t.Fatal("unknown image data was accepted")
	}
}

func TestValidateImageDataRejectsOversizedInput(t *testing.T) {
	data := bytes.Repeat([]byte{0}, maxImageBytes+1)
	if err := validateImageData(data); err == nil {
		t.Fatal("oversized image data was accepted")
	}
}

func TestDetectMIME(t *testing.T) {
	cases := []struct {
		name string
		data []byte
		want string
	}{
		{name: "png", data: []byte{0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A}, want: "image/png"},
		{name: "jpeg", data: []byte{0xFF, 0xD8, 0xFF}, want: "image/jpeg"},
		{name: "gif", data: []byte("GIF89a"), want: "image/gif"},
		{name: "webp", data: []byte("RIFF0000WEBP"), want: "image/webp"},
		{name: "unknown", data: []byte("unknown"), want: ""},
	}

	for _, test := range cases {
		t.Run(test.name, func(t *testing.T) {
			if got := detectMIME(test.data); got != test.want {
				t.Fatalf("detectMIME() = %q, want %q", got, test.want)
			}
		})
	}
}

func TestEnsureExtSanitizesFilename(t *testing.T) {
	data := makeTestPNG(t)
	cases := map[string]string{
		"../../secret.exe":     "secret.png",
		"..\\..\\windows.exe": "windows.png",
		"photo.JPG":             "photo.png",
		"\x00bad.jpg":           "bad.png",
		"":                      "image.png",
	}
	for input, want := range cases {
		if got := ensureExt(input, data); got != want {
			t.Fatalf("ensureExt(%q) = %q, want %q", input, got, want)
		}
	}
}

func TestThirdPartyImageHostsRequireExplicitOptIn(t *testing.T) {
	t.Setenv("GENSOKYO_ENABLE_THIRD_PARTY_IMAGE_HOSTS", "")
	if thirdPartyImageHostsAllowed() {
		t.Fatal("third-party image hosts enabled without opt-in")
	}

	t.Setenv("GENSOKYO_ENABLE_THIRD_PARTY_IMAGE_HOSTS", "true")
	if !thirdPartyImageHostsAllowed() {
		t.Fatal("third-party image hosts not enabled after explicit opt-in")
	}
}

func TestNatureBackendIsDisabled(t *testing.T) {
	if _, err := tryNature(makeTestPNG(t), "test.png"); err == nil {
		t.Fatal("Nature backend unexpectedly accepted an upload")
	}
}

func TestReadCloseRejectsOversizedResponse(t *testing.T) {
	resp := &http.Response{
		Body: io.NopCloser(strings.NewReader(strings.Repeat("x", maxResponseBodyBytes+1))),
	}
	if _, err := readClose(resp); err == nil {
		t.Fatal("oversized response body was accepted")
	}
}

func TestUploadBytesDoesNotUseNetworkWithoutConfiguredBackend(t *testing.T) {
	t.Setenv("GENSOKYO_ENABLE_THIRD_PARTY_IMAGE_HOSTS", "")
	_, err := UploadBytes(makeTestPNG(t), "test.png")
	if err == nil || !strings.Contains(err.Error(), "没有可用") {
		t.Fatalf("unexpected result: %v", err)
	}
}

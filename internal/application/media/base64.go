package media

import (
	"encoding/base64"
	"fmt"
)

// prepareBase64 在 decode 前先限制编码长度，防止几百 MB Base64 → decode → OOM。
func prepareBase64(encoded string, policy MediaPolicy) (*PreparedMedia, error) {
	if policy.MaxEncodedBytes > 0 && int64(len(encoded)) > policy.MaxEncodedBytes {
		return nil, fmt.Errorf("media: base64 length %d exceeds limit %d", len(encoded), policy.MaxEncodedBytes)
	}
	data, err := base64.StdEncoding.DecodeString(encoded)
	if err != nil {
		return nil, fmt.Errorf("media: base64 decode: %w", err)
	}
	if policy.MaxDecodedBytes > 0 && int64(len(data)) > policy.MaxDecodedBytes {
		return nil, fmt.Errorf("media: decoded size %d exceeds limit %d", len(data), policy.MaxDecodedBytes)
	}
	return newPrepared(data, sniffMIME(data), int64(len(data))), nil
}

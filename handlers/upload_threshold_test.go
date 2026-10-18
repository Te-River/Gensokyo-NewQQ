package handlers

import "testing"

// TestNeedsChunkedUploadBoundaries 断言分片上传判据只由官方 file_data 直传上限（5MB）决定，
// 与按类型的软限制无关（软限制仅用于平台降级提示）。
func TestNeedsChunkedUploadBoundaries(t *testing.T) {
	const (
		oneByte = int64(1)
		fiveMB  = int64(5 * 1024 * 1024)
		over5MB = int64(5*1024*1024 + 1)
		twoHMB  = int64(200 * 1024 * 1024)
	)

	tests := []struct {
		name     string
		size     int64
		fileType int
		want     bool
	}{
		{name: "1 字节 图片 直传", size: oneByte, fileType: 1, want: false},
		{name: "恰好 5MB 图片 直传（不超限）", size: fiveMB, fileType: 1, want: false},
		{name: "5MB+1 图片 分片", size: over5MB, fileType: 1, want: true},
		{name: "恰好 5MB 文件 直传（不超限）", size: fiveMB, fileType: 4, want: false},
		{name: "5MB+1 文件 分片（软限制 200MB 不再是直传依据）", size: over5MB, fileType: 4, want: true},
		{name: "5MB+1 未知类型 分片", size: over5MB, fileType: 99, want: true},
		{name: "200MB 文件 分片", size: twoHMB, fileType: 4, want: true},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := needsChunkedUpload(tt.size, tt.fileType); got != tt.want {
				t.Fatalf("needsChunkedUpload(%d, %d) = %v, want %v", tt.size, tt.fileType, got, tt.want)
			}
		})
	}

	// 上限常量必须与官方文档的 5M 一致
	if maxDirectFileDataBytes != 5*1024*1024 {
		t.Fatalf("maxDirectFileDataBytes = %d, want %d", maxDirectFileDataBytes, 5*1024*1024)
	}
}

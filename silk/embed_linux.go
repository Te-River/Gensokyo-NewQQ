//go:build linux && !android

package silk

import "embed"

//go:embed exec/linux-amd64/* exec/linux-arm64/*
var silkCodecs embed.FS

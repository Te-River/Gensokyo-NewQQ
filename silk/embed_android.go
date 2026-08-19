//go:build android

package silk

import "embed"

//go:embed exec/android-arm64/* exec/android-x86/* exec/android-x86_64/*
var silkCodecs embed.FS

//go:build windows

package silk

import "embed"

//go:embed exec/windows-amd64/* exec/windows-386/*
var silkCodecs embed.FS

//go:build darwin

package silk

import "embed"

//go:embed exec/darwin/*
var silkCodecs embed.FS

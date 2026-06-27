package buildinfo

import (
	"fmt"
	"sync"
	"time"
)

var (
	BuildType string
	BuildSpec string

	once     sync.Once
	resolved string
)

func Version() string {
	once.Do(func() {
		if BuildType != "" && BuildSpec != "" {
			resolved = BuildType + "-" + BuildSpec
			return
		}

		now := time.Now().UTC()
		resolved = fmt.Sprintf("dev-%d.%03d", now.Unix(), now.Nanosecond()/int(time.Millisecond))
	})
	return resolved
}

package imagehosting

import (
	"net/http"
	"time"
)

// 部分旧图床后端仍直接使用 http.DefaultClient。为避免网络故障导致请求永久挂起，
// 在不覆盖调用方已有超时设置的前提下提供进程级兜底超时。
func init() {
	if http.DefaultClient.Timeout == 0 {
		http.DefaultClient.Timeout = 15 * time.Second
	}
}

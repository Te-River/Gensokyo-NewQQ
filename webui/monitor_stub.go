//go:build small

package webui

import (
	"time"

	"github.com/gin-gonic/gin"
)

func handleSysInfo(c *gin.Context) {
	// 小型构建: 返回模拟数据，不依赖 gopsutil
	sysInfo := gin.H{
		"cpu_percent": 0.0,
		"memory": gin.H{
			"total":     0,
			"available": 0,
			"percent":   0.0,
		},
		"disk": gin.H{
			"total":   0,
			"free":    0,
			"percent": 0.0,
		},
		"boot_time": time.Now().Unix(),
		"process": gin.H{
			"pid":         0,
			"status":      "running",
			"memory_used": 0,
			"cpu_percent": 0.0,
			"start_time":  time.Now().Unix(),
		},
	}
	c.JSON(200, sysInfo)
}

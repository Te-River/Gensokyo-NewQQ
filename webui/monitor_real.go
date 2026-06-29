//go:build !small

package webui

import (
	"net/http"
	"os"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/shirou/gopsutil/cpu"
	"github.com/shirou/gopsutil/disk"
	"github.com/shirou/gopsutil/host"
	"github.com/shirou/gopsutil/mem"
	"github.com/shirou/gopsutil/process"
)

func handleSysInfo(c *gin.Context) {
	// 获取CPU使用率
	cpuPercent, _ := cpu.Percent(time.Second, false)

	// 获取内存信息
	vmStat, _ := mem.VirtualMemory()

	// 获取磁盘使用情况
	diskStat, _ := disk.Usage("/")

	// 获取系统启动时间
	bootTime, _ := host.BootTime()

	// 获取当前进程信息
	proc, _ := process.NewProcess(int32(os.Getpid()))
	procPercent, _ := proc.CPUPercent()
	memInfo, _ := proc.MemoryInfo()
	procStartTime, _ := proc.CreateTime()

	// 构造返回的JSON数据
	sysInfo := gin.H{
		"cpu_percent": cpuPercent[0], // CPU使用率
		"memory": gin.H{
			"total":     vmStat.Total,       // 总内存
			"available": vmStat.Available,   // 可用内存
			"percent":   vmStat.UsedPercent, // 内存使用率
		},
		"disk": gin.H{
			"total":   diskStat.Total,       // 磁盘总容量
			"free":    diskStat.Free,        // 磁盘剩余空间
			"percent": diskStat.UsedPercent, // 磁盘使用率
		},
		"boot_time": bootTime, // 系统启动时间
		"process": gin.H{
			"pid":         proc.Pid,      // 当前进程ID
			"status":      "running",     // 进程状态，这里假设为运行中
			"memory_used": memInfo.RSS,   // 进程使用的内存
			"cpu_percent": procPercent,   // 进程CPU使用率
			"start_time":  procStartTime, // 进程启动时间
		},
	}
	// 返回JSON数据
	c.JSON(http.StatusOK, sysInfo)
}

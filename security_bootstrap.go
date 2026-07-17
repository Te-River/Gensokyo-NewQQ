package main

import (
	"errors"
	"log"
	"os"

	"github.com/hoshinonyaruko/gensokyo/securityaudit"
)

func init() {
	path := os.Getenv("GENSOKYO_CONFIG_FILE")
	if path == "" {
		path = "config.yml"
	}

	report, err := securityaudit.AuditFile(path)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return
		}
		log.Printf("[security] 无法审计配置文件 %s: %v", path, err)
		return
	}

	for _, finding := range report.Findings {
		log.Printf("[security][%s][%s] %s", finding.Severity, finding.Code, finding.Message)
	}

	if securityaudit.StrictModeEnabled() && report.HasHighRisk() {
		log.Fatal("[security] 严格安全模式检测到高风险配置，已阻止服务启动")
	}
}

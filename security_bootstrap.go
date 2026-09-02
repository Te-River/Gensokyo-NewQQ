package main

import (
	"errors"
	"log"
	"os"
	"strings"

	"github.com/hoshinonyaruko/gensokyo/securityaudit"
)

func init() {
	configuredPath := strings.TrimSpace(os.Getenv("GENSOKYO_CONFIG_FILE"))
	path := configuredPath
	if path == "" {
		path = "config.yml"
	}
	strict := securityaudit.StrictModeEnabled()

	report, err := securityaudit.AuditFile(path)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) && configuredPath == "" {
			// 首次启动时 config.yml 尚未生成，交由现有初始化流程创建。
			return
		}
		if strict {
			log.Fatalf("[security] 严格安全模式无法审计配置文件 %s: %v", path, err)
		}
		log.Printf("[security] 无法审计配置文件 %s: %v", path, err)
		return
	}

	for _, finding := range report.Findings {
		log.Printf("[security][%s][%s] %s", finding.Severity, finding.Code, finding.Message)
	}

	if strict && report.HasHighRisk() {
		log.Fatal("[security] 严格安全模式检测到高风险配置，已阻止服务启动")
	}
}


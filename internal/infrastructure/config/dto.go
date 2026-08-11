package config

import "github.com/hoshinonyaruko/gensokyo/structs"

// CurrentSchemaVersion 当前配置文件 schema 版本。
const CurrentSchemaVersion = 1

// ConfigDTO 是磁盘文件（config.yml）的直接映射，只对磁盘格式负责。
// 业务代码禁止直接读取 DTO；请通过 Snapshot.Config() 获取 RuntimeConfig。
type ConfigDTO struct {
	Version  int              `yaml:"version"`
	Settings structs.Settings `yaml:"settings"`
}

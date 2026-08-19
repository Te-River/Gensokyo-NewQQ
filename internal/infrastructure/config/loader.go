package config

import (
	"os"

	"gopkg.in/yaml.v3"
)

// Load 读取并解析配置文件为 ConfigDTO（不做迁移/校验）。
func Load(path string) (*ConfigDTO, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, newIOError(err)
	}
	return Parse(data)
}

// Parse 解析配置内容为 ConfigDTO。
func Parse(data []byte) (*ConfigDTO, error) {
	var dto ConfigDTO
	if err := yaml.Unmarshal(data, &dto); err != nil {
		return nil, newParseError(err)
	}
	return &dto, nil
}

// ParseNode 解析配置内容为 yaml.Node 树（供迁移使用）。
func ParseNode(data []byte) (*yaml.Node, error) {
	var root yaml.Node
	if err := yaml.Unmarshal(data, &root); err != nil {
		return nil, newParseError(err)
	}
	return &root, nil
}

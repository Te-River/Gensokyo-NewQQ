package config

import (
	"fmt"

	"gopkg.in/yaml.v3"
)

// Migrator 将配置从版本 from 迁移到版本 to。
// 必须基于结构化的 yaml.Node 操作，禁止字符串 contains / 行号 / 缩进 / 手工 append YAML。
type Migrator interface {
	CanMigrate(from, to int) bool
	Migrate(node *yaml.Node, from, to int) error
}

// legacyMigrator 处理 v0（无 version 字段的旧配置）→ v1 的迁移。
type legacyMigrator struct{}

func (legacyMigrator) CanMigrate(from, to int) bool { return from < 1 && to == 1 }

// Migrate 在根 mapping 补上 version: 1（结构化的节点操作）。
func (legacyMigrator) Migrate(root *yaml.Node, from, to int) error {
	if root.Kind != yaml.DocumentNode || len(root.Content) == 0 {
		return fmt.Errorf("invalid document node")
	}
	mapping := root.Content[0]
	if mapping.Kind != yaml.MappingNode {
		return fmt.Errorf("config root must be a mapping")
	}
	if hasKey(mapping, "version") {
		return nil
	}
	mapping.Content = append([]*yaml.Node{
		{Kind: yaml.ScalarNode, Tag: "!!str", Value: "version"},
		{Kind: yaml.ScalarNode, Tag: "!!int", Value: fmt.Sprintf("%d", to)},
	}, mapping.Content...)
	return nil
}

func hasKey(mapping *yaml.Node, key string) bool {
	for i := 0; i+1 < len(mapping.Content); i += 2 {
		if mapping.Content[i].Value == key {
			return true
		}
	}
	return false
}

// migrators 已注册的迁移器（后续 v1→v2 在此追加）。
var migrators = []Migrator{legacyMigrator{}}

// Migrate 将节点树迁移到目标版本；已在目标版本时为空操作。
func Migrate(node *yaml.Node, to int) error {
	from := detectVersion(node)
	if from >= to {
		return nil
	}
	for _, m := range migrators {
		if m.CanMigrate(from, to) {
			if err := m.Migrate(node, from, to); err != nil {
				return newMigrationError(err)
			}
			return nil
		}
	}
	return newMigrationError(fmt.Errorf("no migrator from %d to %d", from, to))
}

// detectVersion 读取根节点 version；缺失视为 0（legacy）。
func detectVersion(root *yaml.Node) int {
	if root.Kind == yaml.DocumentNode && len(root.Content) > 0 {
		root = root.Content[0]
	}
	if root.Kind != yaml.MappingNode {
		return 0
	}
	for i := 0; i+1 < len(root.Content); i += 2 {
		if root.Content[i].Value == "version" {
			var v int
			_ = root.Content[i+1].Decode(&v)
			return v
		}
	}
	return 0
}

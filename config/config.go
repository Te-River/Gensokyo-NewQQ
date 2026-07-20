package config

import (
	"fmt"
	"log"
	"os"
	"path/filepath"
	"reflect"
	"regexp"
	"strings"
	"sync"
	"time"

	"github.com/hoshinonyaruko/gensokyo/structs"
	"github.com/hoshinonyaruko/gensokyo/sys"
	"github.com/hoshinonyaruko/gensokyo/template"
	"gopkg.in/yaml.v3"
)

var (
	instance *Config
	mu       sync.RWMutex
)

type Config struct {
	Version  int              `yaml:"version"`
	Settings structs.Settings `yaml:"settings"`
}

// CommentInfo 用于存储注释及其定位信息
type CommentBlock struct {
	Comments  []string // 一个或多个连续的注释
	TargetKey string   // 注释所指向的键（如果有）
	Offset    int      // 注释与目标键之间的行数
}

// 不支持配置热重载的配置项
var restartRequiredFields = []string{
	"WsAddress", "WsToken", "ReconnectTimes", "HeartBeatInterval", "LaunchReconnectTimes",
	"AppID", "Uin", "Token", "ClientSecret", "ShardCount", "ShardID", "UseUin",
	"TextIntent",
	"ServerDir", "Port", "BackupPort", "Lotus", "LotusPassword", "LotusWithoutIdmaps",
	"WsServerPath", "EnableWsServer", "WsServerToken",
	"IdentifyFile", "IdentifyAppids", "Crt", "Key",
	"DeveloperLog", "LogLevel", "SaveLogs",
	"DisableWebui", "Username", "Password",
	"Title", // 继续检查和增加
}

// LoadConfig 从文件中加载配置并初始化单例配置
func LoadConfig(path string, fastload bool) (*Config, error) {
	mu.Lock()
	defer mu.Unlock()

	configData, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}

	// 清理配置文件中因旧版 bug 产生的重复 settings:/version 行
	if cleaned := cleanupDuplicateSettings(configData); len(cleaned) != len(configData) {
		configData = cleaned
		_ = os.WriteFile(path, configData, 0600)
	}

	// 尝试解析配置数据
	conf := &Config{}
	if err = yaml.Unmarshal(configData, conf); err != nil {
		return nil, err
	}

	if !fastload {
		// 确保本地配置文件的完整性,添加新的字段
		if err = ensureConfigComplete(path); err != nil {
			return nil, err
		}
	} else {
		if isValidConfig(conf) {
			// 用现有的instance比对即将覆盖赋值的conf,用[]string返回配置发生了变化的配置项
			changedFields := compareConfigChanges("Settings", instance.Settings, conf.Settings)
			// 根据changedFields进行进一步的操作，在不支持热重载的字段实现自动重启
			if len(changedFields) > 0 {
				log.Printf("配置已变更的字段：%v", changedFields)
				checkForRestart(changedFields) // 检查变更字段是否需要重启
			}
		} //conf为空时不对比
	}

	// 更新单例实例，即使它已经存在 更新前检查是否有效,vscode对文件的更新行为会触发2次文件变动
	// 第一次会让configData为空,迅速的第二次才是正常有值的configData
	if isValidConfig(conf) {
		instance = conf
	}

	return instance, nil
}

func isValidConfig(conf *Config) bool {
	// 确认config不为空且必要字段已设置
	return conf != nil && conf.Version != 0
}

// 去除Settings前缀
func stripSettingsPrefix(fieldName string) string {
	return strings.TrimPrefix(fieldName, "Settings.")
}

// compareConfigChanges 检查并返回发生变化的配置字段，处理嵌套结构体
func compareConfigChanges(prefix string, oldConfig interface{}, newConfig interface{}) []string {
	var changedFields []string

	oldVal := reflect.ValueOf(oldConfig)
	newVal := reflect.ValueOf(newConfig)

	// 解引用指针
	if oldVal.Kind() == reflect.Ptr {
		oldVal = oldVal.Elem()
	}
	if newVal.Kind() == reflect.Ptr {
		newVal = newVal.Elem()
	}

	// 遍历所有字段
	for i := 0; i < oldVal.NumField(); i++ {
		oldField := oldVal.Field(i)
		newField := newVal.Field(i)
		fieldType := oldVal.Type().Field(i)
		fieldName := fieldType.Name

		fullFieldName := fieldName
		if prefix != "" {
			fullFieldName = fmt.Sprintf("%s.%s", prefix, fieldName)
		}

		// 对于结构体字段递归比较
		if oldField.Kind() == reflect.Struct || newField.Kind() == reflect.Struct {
			subChanges := compareConfigChanges(fullFieldName, oldField.Interface(), newField.Interface())
			changedFields = append(changedFields, subChanges...)
		} else {
			if !reflect.DeepEqual(oldField.Interface(), newField.Interface()) {
				// 去除Settings前缀后添加到变更字段列表
				changedField := stripSettingsPrefix(fullFieldName)
				changedFields = append(changedFields, changedField)
			}
		}
	}

	return changedFields
}

// 检查是否需要重启
func checkForRestart(changedFields []string) {
	for _, field := range changedFields {
		for _, restartField := range restartRequiredFields {
			if field == restartField {
				fmt.Println("Configuration change requires restart:", field)
				sys.RestartApplication() // 调用重启函数
				return
			}
		}
	}
}

func CreateAndWriteConfigTemp() error {
	// 读取config.yml
	configFile, err := os.ReadFile("config.yml")
	if err != nil {
		return err
	}

	// 获取当前日期
	currentDate := time.Now().Format("2006-1-2")
	// 重命名原始config.yml文件
	err = os.Rename("config.yml", "config"+currentDate+".yml")
	if err != nil {
		return err
	}

	var config Config
	err = yaml.Unmarshal(configFile, &config)
	if err != nil {
		return err
	}

	// 创建config_temp.yml文件
	tempFile, err := os.Create("config.yml")
	if err != nil {
		return err
	}
	defer tempFile.Close()

	// 使用yaml.Encoder写入，以保留注释
	encoder := yaml.NewEncoder(tempFile)
	encoder.SetIndent(2) // 设置缩进
	err = encoder.Encode(config)
	if err != nil {
		return err
	}

	// 处理注释并重命名文件
	err = addCommentsToConfigTemp(template.ConfigTemplate, "config.yml")
	if err != nil {
		return err
	}

	return nil
}

func parseTemplate(template string) ([]CommentBlock, map[string]string) {
	var blocks []CommentBlock
	lines := strings.Split(template, "\n")

	var currentBlock CommentBlock
	var lastKey string

	directComments := make(map[string]string)

	for _, line := range lines {
		trimmed := strings.TrimSpace(line)
		if strings.HasPrefix(trimmed, "#") {
			currentBlock.Comments = append(currentBlock.Comments, trimmed) // 收集注释行
		} else {
			if containsKey(trimmed) {
				key := strings.SplitN(trimmed, ":", 2)[0]
				trimmedKey := strings.TrimSpace(key)

				if len(currentBlock.Comments) > 0 {
					currentBlock.TargetKey = lastKey // 关联到上一个找到的键
					blocks = append(blocks, currentBlock)
					currentBlock = CommentBlock{} // 重置为新的注释块
				}

				// 如果当前行包含注释，则单独处理
				if parts := strings.SplitN(trimmed, "#", 2); len(parts) > 1 {
					directComments[trimmedKey] = "#" + parts[1]
				}
				lastKey = trimmedKey // 更新最后一个键
			} else if len(currentBlock.Comments) > 0 {
				// 如果当前行不是注释行且存在挂起的注释，但并没有新的键出现，将其作为独立的注释块
				blocks = append(blocks, currentBlock)
				currentBlock = CommentBlock{} // 重置为新的注释块
			}
		}
	}

	// 处理文件末尾的挂起注释块
	if len(currentBlock.Comments) > 0 {
		blocks = append(blocks, currentBlock)
	}

	return blocks, directComments
}

func addCommentsToConfigTemp(template, tempFilePath string) error {
	commentBlocks, directComments := parseTemplate(template)

	// 读取并分割新生成的配置文件内容
	content, err := os.ReadFile(tempFilePath)
	if err != nil {
		return err
	}
	lines := strings.Split(string(content), "\n")

	// 处理并插入注释
	for _, block := range commentBlocks {
		// 根据注释块的目标键，找到插入位置并插入注释
		for i, line := range lines {
			if containsKey(line) {
				key := strings.SplitN(line, ":", 2)[0]
				if strings.TrimSpace(key) == block.TargetKey {
					// 计算基本插入点：在目标键之后
					insertionPoint := i + block.Offset + 1

					// 向下移动插入点直到找到键行或到达文件末尾
					for insertionPoint < len(lines) && !containsKey(lines[insertionPoint]) {
						insertionPoint++
					}

					// 在计算出的插入点插入注释
					if insertionPoint >= len(lines) {
						lines = append(lines, block.Comments...) // 如果到达文件末尾，直接追加注释
					} else {
						// 插入注释到计算出的位置
						lines = append(lines[:insertionPoint], append(block.Comments, lines[insertionPoint:]...)...)
					}
					break
				}
			}
		}
	}

	// 处理直接跟在键后面的注释
	for i, line := range lines {
		if containsKey(line) {
			key := strings.SplitN(line, ":", 2)[0]
			trimmedKey := strings.TrimSpace(key)
			if comment, exists := directComments[trimmedKey]; exists {
				// 如果这个键有直接的注释
				lines[i] = line + " " + comment
			}
		}
	}

	// 重新组合lines为一个字符串，准备写回文件
	updatedContent := strings.Join(lines, "\n")

	// 写回更新后的内容到原配置文件
	err = os.WriteFile(tempFilePath, []byte(updatedContent), 0600)
	if err != nil {
		return err
	}

	return nil
}

// containsKey 检查给定的字符串行是否可能包含YAML键。
// 它尝试排除注释行和冒号用于其他目的的行（例如，在URLs中）。
func containsKey(line string) bool {
	// 去除行首和行尾的空格
	trimmedLine := strings.TrimSpace(line)

	// 如果行是注释，直接返回false
	if strings.HasPrefix(trimmedLine, "#") {
		return false
	}

	// 检查是否存在冒号，如果不存在，则直接返回false
	colonIndex := strings.Index(trimmedLine, ":")
	return colonIndex != -1
}

// 确保配置完整性
func ensureConfigComplete(path string) error {
	// 读取配置文件到缓冲区
	configData, err := os.ReadFile(path)
	if err != nil {
		return err
	}

	// 将现有的配置解析到结构体中
	currentConfig := &Config{}
	err = yaml.Unmarshal(configData, currentConfig)
	if err != nil {
		return err
	}

	// 解析默认配置模板到结构体中
	defaultConfig := &Config{}
	err = yaml.Unmarshal([]byte(template.ConfigTemplate), defaultConfig)
	if err != nil {
		return err
	}

	// 使用反射找出结构体中缺失的设置
	missingSettingsByReflection, err := getMissingSettingsByReflection(currentConfig, defaultConfig)
	if err != nil {
		return err
	}

	// 使用文本比对找出缺失的设置
	missingSettingsByText, err := getMissingSettingsByText(template.ConfigTemplate, string(configData))
	if err != nil {
		return err
	}

	// 合并缺失的设置
	allMissingSettings := mergeMissingSettings(missingSettingsByReflection, missingSettingsByText)

	// 如果存在缺失的设置，处理缺失的配置行
	if len(allMissingSettings) > 0 {
		fmt.Println("缺失的设置:", allMissingSettings)
		missingConfigLines, err := extractMissingConfigLines(allMissingSettings, template.ConfigTemplate)
		if err != nil {
			return err
		}

		// 将缺失的配置追加到现有配置文件
		written, err := appendToConfigFile(path, missingConfigLines)
		if err != nil {
			return err
		}

		if written > 0 {
			fmt.Println("检测到配置文件缺少项。已经更新配置文件，正在重启程序以应用新的配置。")
			sys.RestartApplication()
		}
	}

	return nil
}

// mergeMissingSettings 合并由反射和文本比对找到的缺失设置
func mergeMissingSettings(reflectionSettings, textSettings map[string]string) map[string]string {
	for k, v := range textSettings {
		reflectionSettings[k] = v
	}
	return reflectionSettings
}

// getMissingSettingsByReflection 使用反射来对比结构体并找出缺失的设置
func getMissingSettingsByReflection(currentConfig, defaultConfig *Config) (map[string]string, error) {
	missingSettings := make(map[string]string)
	currentVal := reflect.ValueOf(currentConfig).Elem()
	defaultVal := reflect.ValueOf(defaultConfig).Elem()

	for i := 0; i < currentVal.NumField(); i++ {
		field := currentVal.Type().Field(i)
		yamlTag := field.Tag.Get("yaml")
		if yamlTag == "" {
			continue // 跳过没有yaml标签的字段
		}
		yamlKeyName := strings.SplitN(yamlTag, ",", 2)[0]
		// 跳过结构体类型的字段（如 Settings），无法通过追加一行来修复
		if currentVal.Field(i).Kind() == reflect.Struct {
			continue
		}
		if isZeroOfUnderlyingType(currentVal.Field(i).Interface()) && !isZeroOfUnderlyingType(defaultVal.Field(i).Interface()) {
			missingSettings[yamlKeyName] = "missing"
		}
	}

	return missingSettings, nil
}

// getMissingSettingsByText compares settings in two strings line by line, looking for missing keys.
func getMissingSettingsByText(templateContent, currentConfigContent string) (map[string]string, error) {
	templateKeys := extractKeysFromString(templateContent)
	currentKeys := extractKeysFromString(currentConfigContent)

	// 构建模板中每个 key 的父 key 映射（用于判断嵌套关系）
	parentMap := buildParentKeyMap(templateContent)

	missingSettings := make(map[string]string)
	for key := range templateKeys {
		if _, found := currentKeys[key]; !found {
			missingSettings[key] = "missing"
		}
	}
	// 第一轮过滤：如果父 key 已在配置中存在 → 跳过（属于已存在的块）
	// 但父 key 为 "settings" 时不过滤（顶级配置项，单独缺失需要追加）
	for key := range missingSettings {
		parent, hasParent := parentMap[key]
		if !hasParent || parent == "settings" {
			continue
		}
		if _, parentExists := currentKeys[parent]; parentExists {
			delete(missingSettings, key)
		}
	}

	// 第二轮：如果某个缺失 key 的祖先 key 不在配置中，补上祖先 key
	// 例如子 key 缺失但父 `image_hosting` 也不在配置中 → 把 `image_hosting` 加入缺失
	for key := range missingSettings {
		// 沿着父链向上追溯
		cur := key
		for {
			p, ok := parentMap[cur]
			if !ok {
				break // 已到顶层 key
			}
			if _, exists := currentKeys[p]; !exists {
				// 祖先不存在配置中，加入缺失
				missingSettings[p] = "missing"
			}
			cur = p
		}
	}

	// 第三轮：如果父 key 现在也在缺失列表中（刚补上的祖先）→ 跳过子 key
	for key := range missingSettings {
		parent, hasParent := parentMap[key]
		if !hasParent {
			continue
		}
		if _, parentMissing := missingSettings[parent]; parentMissing {
			delete(missingSettings, key)
		}
	}

	// 第四轮：只保留 settings 下的直接缺失项，避免子 key 被单独追加造成结构混乱
	topLevelMissing := make(map[string]string)
	for key := range missingSettings {
		if parentMap[key] == "settings" {
			topLevelMissing[key] = "missing"
		}
	}

	return topLevelMissing, nil
}

// buildParentKeyMap 解析模板，为每个 key 找到其父 key
// 例如 image_hosting.cos.enabled 的父 key 是 cos，cos 的父 key 是 image_hosting
func buildParentKeyMap(templateContent string) map[string]string {
	parentMap := make(map[string]string)
	lines := strings.Split(templateContent, "\n")

	// 用栈跟踪缩进层级: 每个元素是 (缩进长度, key名)
	type indentKey struct {
		indent int
		key    string
	}
	var stack []indentKey

	for _, line := range lines {
		trimmed := strings.TrimSpace(line)
		if trimmed == "" || !strings.Contains(trimmed, ":") {
			continue
		}
		// 跳过注释行
		if strings.HasPrefix(trimmed, "#") {
			continue
		}
		key := strings.TrimSpace(strings.Split(trimmed, ":")[0])
		indent := len(line) - len(strings.TrimLeft(line, " \t"))

		// 出栈：移除所有缩进 >= 当前行的
		for len(stack) > 0 && stack[len(stack)-1].indent >= indent {
			stack = stack[:len(stack)-1]
		}

		// 记录父 key
		if len(stack) > 0 {
			parentMap[key] = stack[len(stack)-1].key
		}

		// 入栈
		stack = append(stack, indentKey{indent, key})
	}

	return parentMap
}

// extractKeysFromString reads a string and extracts the keys (text before the colon).
func extractKeysFromString(content string) map[string]bool {
	keys := make(map[string]bool)
	lines := strings.Split(content, "\n")
	for _, line := range lines {
		trimmed := strings.TrimSpace(line)
		// 跳过空行和注释行
		if trimmed == "" || strings.HasPrefix(trimmed, "#") {
			continue
		}
		if strings.Contains(line, ":") {
			key := strings.TrimSpace(strings.Split(line, ":")[0])
			keys[key] = true
		}
	}
	return keys
}

func extractMissingConfigLines(missingSettings map[string]string, configTemplate string) ([]string, error) {
	var missingConfigLines []string

	lines := strings.Split(configTemplate, "\n")
	for yamlKey := range missingSettings {
		found := false
		// Create a regex to match the line with optional spaces around the colon
		regexPattern := fmt.Sprintf(`^(\s*)%s\s*:\s*`, regexp.QuoteMeta(yamlKey))
		regex, err := regexp.Compile(regexPattern)
		if err != nil {
			return nil, fmt.Errorf("invalid regex pattern: %s", err)
		}

		for i, line := range lines {
			matches := regex.FindStringSubmatch(line)
			if matches == nil {
				continue
			}
			// 提取整个块：从当前行到下一个同等或更浅缩进的行
			indent := len(matches[1])
			missingConfigLines = append(missingConfigLines, line)
			for j := i + 1; j < len(lines); j++ {
				nextLine := lines[j]
				if strings.TrimSpace(nextLine) == "" {
					missingConfigLines = append(missingConfigLines, nextLine)
					continue
				}
				nextIndent := len(nextLine) - len(strings.TrimLeft(nextLine, " \t"))
				if nextIndent <= indent {
					break
				}
				missingConfigLines = append(missingConfigLines, nextLine)
			}
			found = true
			break
		}
		if !found {
			return nil, fmt.Errorf("missing configuration for key: %s", yamlKey)
		}
	}

	return missingConfigLines, nil
}

func appendToConfigFile(path string, lines []string) (int, error) {
	// 先读取现有内容
	existingBytes, err := os.ReadFile(path)
	if err != nil {
		return 0, err
	}
	existingContent := string(existingBytes)

	file, err := os.OpenFile(path, os.O_WRONLY|os.O_APPEND, 0644)
	if err != nil {
		fmt.Println("打开文件错误:", err)
		return 0, err
	}
	defer file.Close()

	// 按配置块写入，对每个顶层 key 做存在性检查，避免同名子 key 被误跳过
	written := 0
	i := 0
	for i < len(lines) {
		line := lines[i]
		trimmed := strings.TrimSpace(line)
		if trimmed == "" || !strings.Contains(trimmed, ":") {
			if trimmed != "" {
				if _, err := file.WriteString("\n" + line); err != nil {
					return written, err
				}
				written++
			}
			i++
			continue
		}

		indent := len(line) - len(strings.TrimLeft(line, " \t"))
		key := strings.TrimSpace(strings.SplitN(trimmed, ":", 2)[0])

		// 收集当前块（从当前行到下一个同等或更浅缩进的行）
		var blockLines []string
		blockLines = append(blockLines, line)
		j := i + 1
		for j < len(lines) {
			nextLine := lines[j]
			if strings.TrimSpace(nextLine) == "" {
				blockLines = append(blockLines, nextLine)
				j++
				continue
			}
			nextIndent := len(nextLine) - len(strings.TrimLeft(nextLine, " \t"))
			if nextIndent <= indent {
				break
			}
			blockLines = append(blockLines, nextLine)
			j++
		}

		// 如果顶层 key 已存在，跳过整个块
		if keyExistsInConfig(existingContent, key) {
			i = j
			continue
		}

		// 写入整个块
		for _, blockLine := range blockLines {
			if _, err := file.WriteString("\n" + blockLine); err != nil {
				fmt.Println("写入配置错误:", err)
				return written, err
			}
			written++
		}
		i = j
	}

	if written > 0 {
		fmt.Println("配置已更新，写入到文件:", path)
	}

	return written, nil
}

// keyExistsInConfig 检查配置中是否已存在某个顶层 key（按行首缩进判断为同层级）
func keyExistsInConfig(content, key string) bool {
	lines := strings.Split(content, "\n")
	for _, line := range lines {
		trimmed := strings.TrimSpace(line)
		if strings.HasPrefix(trimmed, "#") || trimmed == "" {
			continue
		}
		if !strings.Contains(trimmed, ":") {
			continue
		}
		// 只检查顶层或 settings 下的同层级 key（缩进为 2 个空格）
		indent := len(line) - len(strings.TrimLeft(line, " \t"))
		if indent != 2 {
			continue
		}
		lineKey := strings.TrimSpace(strings.SplitN(trimmed, ":", 2)[0])
		if lineKey == key {
			return true
		}
	}
	return false
}

// cleanupDuplicateSettings 清理配置文件中因旧版 bug 产生的异常。
// 1. 删除重复的 settings: 行（截断到第一个 settings: 块末尾）
// 2. 如果文件开头缺少 version:/settings: 顶层 key，自动补全
func cleanupDuplicateSettings(data []byte) []byte {
	content := string(data)
	// 统一换行符为 \n，移除 \r
	content = strings.ReplaceAll(content, "\r\n", "\n")
	content = strings.ReplaceAll(content, "\r", "")
	lines := strings.Split(content, "\n")

	// 第一步：查找并删除重复的 settings: 行
	settingsCount := 0
	cutIndex := -1
	for i, line := range lines {
		trimmed := strings.TrimSpace(line)
		if trimmed == "settings:" {
			settingsCount++
			if settingsCount == 2 {
				cutIndex = i
				break
			}
		}
	}
	if cutIndex > 0 {
		lines = lines[:cutIndex]
		content = strings.Join(lines, "\n")
		fmt.Printf("[config] 检测到重复的 settings: 行，已截断\n")
	}

	// 第二步：检查文件开头是否以 version:/settings: 开头
	firstNonEmpty := -1
	hasVersion := false
	hasSettings := false
	for i, line := range lines {
		trimmed := strings.TrimSpace(line)
		if trimmed == "" || strings.HasPrefix(trimmed, "#") {
			continue
		}
		if firstNonEmpty < 0 {
			firstNonEmpty = i
		}
		if trimmed == "version:" || strings.HasPrefix(trimmed, "version:") {
			hasVersion = true
		}
		if trimmed == "settings:" {
			hasSettings = true
		}
	}

	// 如果第一个非空非注释行不是 version: 或 settings:，说明顶层 key 丢失
	if firstNonEmpty >= 0 {
		trimmed := strings.TrimSpace(lines[firstNonEmpty])
		if trimmed != "settings:" && !strings.HasPrefix(trimmed, "version:") {
			// 文件结构损坏，重建：version + settings + 原内容缩进
			var rebuilt []string
			if !hasVersion {
				rebuilt = append(rebuilt, "version: 1")
			}
			if !hasSettings {
				rebuilt = append(rebuilt, "settings:")
			}
			for _, line := range lines {
				trimmed := strings.TrimSpace(line)
				if trimmed == "" || trimmed == "settings:" || strings.HasPrefix(trimmed, "version:") {
					continue
				}
				if strings.HasPrefix(trimmed, "#") {
					rebuilt = append(rebuilt, line) // 注释原样保留
				} else {
					rebuilt = append(rebuilt, "  "+trimmed) // 内容缩进两层
				}
			}
			content = strings.Join(rebuilt, "\n") + "\n"
			fmt.Printf("[config] 检测到配置文件结构损坏，已重建\n")
		}
	}

	return []byte(content)
}

func isZeroOfUnderlyingType(x interface{}) bool {
	return reflect.DeepEqual(x, reflect.Zero(reflect.TypeOf(x)).Interface())
}

// UpdateConfig 将配置写入文件
func UpdateConfig(conf *Config, path string) error {
	data, err := yaml.Marshal(conf)
	if err != nil {
		return err
	}
	return os.WriteFile(path, data, 0600)
}

// WriteYAMLToFile 将YAML格式的字符串写入到指定的文件路径
func WriteYAMLToFile(yamlContent string) error {
	// 获取当前执行的可执行文件的路径
	exePath, err := os.Executable()
	if err != nil {
		log.Println("Error getting executable path:", err)
		return err
	}

	// 获取可执行文件所在的目录
	exeDir := filepath.Dir(exePath)

	// 构建config.yml的完整路径
	configPath := filepath.Join(exeDir, "config.yml")

	// 写入文件
	os.WriteFile(configPath, []byte(yamlContent), 0600)

	sys.RestartApplication()
	return nil
}

// DeleteConfig 删除配置文件并创建一个新的配置文件模板
func DeleteConfig() error {
	// 获取当前执行的可执行文件的路径
	exePath, err := os.Executable()
	if err != nil {
		fmt.Println("Error getting executable path:", err)
		return err
	}

	// 获取可执行文件所在的目录
	exeDir := filepath.Dir(exePath)

	// 构建config.yml的完整路径
	configPath := filepath.Join(exeDir, "config.yml")

	// 删除配置文件
	if err := os.Remove(configPath); err != nil {
		fmt.Println("Error removing config file:", err)
		return err
	}

	// 获取内网IP地址
	ip, err := sys.GetLocalIP()
	if err != nil {
		fmt.Println("Error retrieving the local IP address:", err)
		return err
	}

	// 将 <YOUR_SERVER_DIR> 替换成实际的内网IP地址
	configData := strings.Replace(template.ConfigTemplate, "<YOUR_SERVER_DIR>", ip, -1)

	// 创建一个新的配置文件模板 写到配置
	if err := os.WriteFile(configPath, []byte(configData), 0600); err != nil {
		fmt.Println("Error writing config.yml:", err)
		return err
	}

	sys.RestartApplication()

	return nil
}

// oss_type 枚举值（仅控制图片上传路径）
const (
	OssTypeLocal     = 0
	OssTypeTencent   = 1
	OssTypeBaidu     = 2
	OssTypeAliyun    = 3
	OssTypeCOS       = 4
	OssTypeBilibili  = 5
	OssTypeQQChannel = 6
	OssTypeChatGLM   = 7
	OssTypeUkaka     = 8
	OssTypeXingye    = 9
	OssTypeNature    = 10
)
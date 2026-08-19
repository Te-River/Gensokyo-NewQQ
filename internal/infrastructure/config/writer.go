package config

import (
	"io"
	"os"
	"path/filepath"

	"gopkg.in/yaml.v3"
)

// AtomicWrite 以 临时文件 + fsync + 前置校验 + 备份 + rename 方式安全写入配置。
// 写失败时原配置保持可用；覆盖前会保留一份 config.yml.bak。
func AtomicWrite(path string, data []byte) error {
	// 写入前校验内容可解析，拒绝用坏配置覆盖有效配置
	if _, err := Parse(data); err != nil {
		return newValidationError("config", "refusing to write unparsable config: "+err.Error())
	}

	dir := filepath.Dir(path)
	tmp := filepath.Join(dir, filepath.Base(path)+".tmp")

	f, err := os.OpenFile(tmp, os.O_WRONLY|os.O_CREATE|os.O_TRUNC, 0600)
	if err != nil {
		return newIOError(err)
	}
	if _, err := f.Write(data); err != nil {
		f.Close()
		os.Remove(tmp)
		return newIOError(err)
	}
	if err := f.Sync(); err != nil {
		f.Close()
		os.Remove(tmp)
		return newIOError(err)
	}
	if err := f.Close(); err != nil {
		os.Remove(tmp)
		return newIOError(err)
	}

	// 备份现有配置（拷贝而非 rename，保证 rename 失败时原文件仍可用）
	if _, err := os.Stat(path); err == nil {
		if err := copyFile(path, path+".bak"); err != nil {
			os.Remove(tmp)
			return newIOError(err)
		}
	}

	// 原子替换
	if err := os.Rename(tmp, path); err != nil {
		os.Remove(tmp)
		return newIOError(err)
	}
	return nil
}

// WriteYAML 序列化 DTO 并原子写入。
func WriteYAML(path string, dto ConfigDTO) error {
	data, err := yaml.Marshal(dto)
	if err != nil {
		return newParseError(err)
	}
	return AtomicWrite(path, data)
}

func readFile(path string) ([]byte, error) { return os.ReadFile(path) }

func copyFile(src, dst string) error {
	in, err := os.Open(src)
	if err != nil {
		return err
	}
	defer in.Close()
	out, err := os.OpenFile(dst, os.O_WRONLY|os.O_CREATE|os.O_TRUNC, 0600)
	if err != nil {
		return err
	}
	if _, err := io.Copy(out, in); err != nil {
		out.Close()
		return err
	}
	return out.Close()
}

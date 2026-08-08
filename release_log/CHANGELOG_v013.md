# Changelog — Release013

> 自 Release012 以来的所有变更。

---

## 🔒 安全：移除源码内置云凭据（S0）

**背景：** `imagehosting/nature.go` 中曾硬编码一组腾讯 COS 的 SecretId/SecretKey（base64 编码），
用于 "Nature" 免费图床（oss_type=10）直传。内置真实云凭据属于高危安全缺陷，必须移除。

**变更：**

- `imagehosting/nature.go` 删除 base64 硬编码凭据及 `mustB64` 函数，改为从配置读取。
- `structs.Settings.Nature` 类型由空结构 `ImageHostingSimple` 改为 `ImageHostingNature`
  （含 `secret_id` / `secret_key` / `region` / `bucket` / `domain` 字段）。
- `config.GetImageHostingNature()` 返回类型同步更新。
- 凭据缺失时 **fail closed**（返回错误），不再回退到任何内置凭据。
- 默认域名不再指向旧存储桶的 CDN，改为与 `cos.go` 一致：留空时使用 COS 默认域名。
- `template/config_template.go`、`readme.md`、`imagehosting/README.md` 同步更新配置示例。

**⚠️ 破坏性变更：** `oss_type=10`（Nature）不再开箱即用，必须自行填写 COS 凭据。

**安全注意：** 请到腾讯云控制台 revoke 旧凭据（被内置在历史版本源码中的那组），
并确认旧凭据已无法继续使用。删除源码中的 Secret 不等于修复，**旧凭据失效**才是真正的修复点。

---

## 🧪 验证

| 命令 | 结果 |
|------|------|
| `go build ./...` | ✅ 通过 |
| `go vet ./...` | ✅ 通过 |
| `go test ./imagehosting/ ./config/ ./structs/ ./template/` | ✅ 通过 |
| `go test ./...` | ✅ 通过 |

---

## ✅ 提交记录

```
<commit hash>
```

# Changelog — Release006 (since Release005)

> 自 `Release005` 以来的所有变更。提交范围: `Release005..69c358d`

---

## 🚀 新增功能

### 消息撤回 (OneBot V11 语义)
- `delete_group_msg` / `delete_msg` 实现，符合 OneBot V11 风格
- `[CQ:remove]` 支持按指定消息 ID 撤回，msg_id 为必填参数
- 撤回用户最近一条消息的出站 CQ 码

### 构建信息 & 命令前缀
- 新增 `buildinfo` 包，嵌入构建元数据（git commit / 时间戳）
- `status`/`broadcast` 命令前缀可配置（`status_prefix`, `broadcast_prefix`）
- 支持 `--version` 查看版本信息
- 可禁用 status/broadcast 命令

### 消息队列 & 限流器
- 新增 `messagequeue` 包，基于 `golang.org/x/time/rate` 实现限流
- 批量消息发送时自动排队

### WebUI 仪表盘
- 完整的系统监控面板（CPU、内存、磁盘、进程）
- 基于 ApexCharts 的资源趋势线型图
- Monaco Editor 配置编辑器（前端 dist 约 4.3MB）

---

## 🔧 构建系统改进

### 参数重构
| 旧参数 | 新参数 | 说明 |
|--------|--------|------|
| `-Small` | `-NoWebUI` | 编译不带 WebUI 的精简版 |
| — | `-All` | 全平台**双版本**编译（完整版 + noWebUI） |
| — | `-LinuxOnly` | 仅 Linux 全平台双版本 |

- 默认编译完整版（含 WebUI 监控数据）
- `-NoWebUI` 输出文件名带 `-noWebui` 后缀
- 兼容 `--all` `--nowebui` 写法

### 输出命名
- 移除旧 `-small` 后缀
- 统一格式: `gensokyo-{OS}-{Arch}[-noWebui].exe`

### 依赖镜像
```
GOPROXY = 阿里云 → goproxy.cn → 清华 → direct
```
多镜像链，自动 fallback

### UPX 压缩
- 自动检测 UPX，压缩级别可配（默认 -7）
- `-NoUPX` 跳过压缩

---

## 🐛 Bug 修复

### Config 修复 (PR #7)
- **config 缺失设置导致无限重启循环** — `appendToConfigFile` 重写，插入到 `settings:` 块内而非追加
- **`cleanupDuplicateSettings`** — yaml 库 Unmarshal→Marshal 重建，修复截断方向
- **降级机制** — 修复失败时生成默认模板 + 备份旧文件

### Idmap 修复 (PR #6 合并后)
- `type=5` 写新库优先，迁移期双写旧库
- HTTP API 统一为原始接口名
- 逆向映射缺失时正向扫描兜底
- 虚拟 ID=0 自动重新分配
- 内存 username 缓存
- 普通文本 `[CQ:at]` 不再转换

### WebUI 修复
- **内存显示 0 Bytes** — 后端增加 `used` 字段
- **磁盘/内存百分比** — 保留 2 位小数
- **ApexCharts 飞入动画** — 禁用 `animateGradually`，保留平滑过渡
- **初始渲染** — 预填充种子数据点，避免首次渲染从底部飞入

### 其他
- `ProcessCQRemoveOutbound` 失败不保留下文
- `appendToConfigFile` 追加前去重
- PR #6 误改 Errorf 改回 Infof
- `.gitignore` 添加 `gensokyo-*` 覆盖多平台构建产物

---

## ⚡ 性能优化

- **二进制体积**: Monaco Editor 移除并恢复，-small 标签系统
- **日志轮转**: 默认保留 7 天/最大 24MB，`log_keep_files` 默认 12
- **简化错误日志**: 委托给 `LogToFile`，移除旧 rotation/JSON 行为

---

## 📝 文档

- 扩展 CQ 码汇总更新
- API 介绍文档补充 `delete_msg` / `delete_group_msg`
- idmap 文档标注实际运行状态
- 新增功能文档全面更新

---

## 🔄 提交记录 (摘要)

```
7351c8b  buildinfo, command prefixes, logging revamp
9a6c7d3  Refactor build scripts, improve log rotation
162e91f  delete_group_msg (OneBot V11 semantic)
ab60893  type=5 新库优先, HTTP API 统一接口名
277f449  idmap 正向扫描兜底 & 虚拟ID=0重分配
798a4f3  ProcessCQRemoveOutbound 修复
929bed5  config 无限重启修复
5b273fd  二进制体积优化 + 消息队列
b5224b0  cleanupDuplicateSettings yaml 重写
141bca7  -small 默认编译, 恢复 Monaco Editor
9be0789  去掉 -small 输出后缀
1206581  formatBytes 尾随零修复
9d26bd7  -Small → -NoWebUI, -All 双版本
b26ee03  GOPROXY 多镜像链
6babb0d  兼容 --all/--nowebui
f2f75f0  ApexCharts 动画 & 磁盘百分比修复
7f5b0a6  内存 used 字段修复
c6c2cae  保留动态动画
69c358d  最终状态
```

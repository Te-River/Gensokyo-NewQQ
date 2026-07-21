# Changelog — Release009

> 自 Release008 (`6b38e78`) 以来的所有变更。

---

## 🐛 Bug 修复

### HTTP API 端口绑定失败不再终止进程

在 Windows 系统上，端口 5700 可能被 Hyper-V 或其他系统服务保留，导致 `http_address` 配置的 HTTP API 服务器启动时出现：

```
http apilisten: listen tcp 127.0.0.1:5700: bind: An attempt was made to access a socket in a way forbidden by its access permissions.
```

此前程序会调用 `log.Fatalf` 直接终止整个进程，即使主 Gin 服务器（配置端口）和 WebSocket 连接已经正常运行。

**修复**：将 `log.Fatalf` 改为 `mylog.Printf`，仅记录错误日志但不终止进程。HTTP API 服务器是可选附加功能，其绑定失败不影响主程序运行。

---

## 提交历史

```
6b38e78 fix: http apilisten 绑定失败时不终止进程
```
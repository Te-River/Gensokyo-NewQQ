# Changelog — Release016

> 自 Release015 以来的所有变更。

---

## ✨ 新功能 / 变更

### 生产主链接入 `internal/` + `adapter/` 分层架构（`arch_mode` 切换，2026-10-07）

生产主链接入 `internal/`（`application/`、`domain/`、`infrastructure/`）+ `adapter/`（`qq/`、`onebot/`、`media/`、`state/`、`identity/`）分层架构，与 legacy 两路同时装配，按 `arch_mode` 逐请求切换，legacy 保留回退：

- **配置**：新增 `arch_mode`（`legacy|shadow|new`，默认 `new`）；逐请求读取、热重载即时生效，空值/非法值回退 `new`；`legacy` 一键回退，行为零变化
- **六个接缝**：出站（`handlers/outbound_bridge.go`）、入站（`Processor/inbound_bridge.go`）、action（`callapi/dispatch_bridge.go`）、state（`handlers/state_bridge.go`）、config（`internal/infrastructure/config/bootstrap.go`）、media（`handlers/media_bridge.go`）；新链未注入/失败时自动落回 legacy
- **装配**：`main.go` 经 `configbootstrap.Bootstrap("config.yml")` 加载配置快照并注入新旧两路；bootstrap 失败时降级 legacy 语义（不 panic）

---

## 🐛 修复

### 私聊 markdown 段内嵌 keyboard 按钮权限 `permission.type=0` 原样发出（点击提示"无权限操作"）

私聊 `[CQ:markdown]` 段内嵌 keyboard（markdown 段 `data.data` 同时含 `markdown` 与 `keyboard`，qbind 等下游插件的真实形态）路径漏调 `ResolvePlaceholderUserIDs`，QQ 官方 C2C 单聊不支持的按钮 `permission.type=0`（指定用户可操作）原样发给 QQ，用户点击按钮被服务端拒绝并提示"无权限操作"：

- **修改**：`HandleSendPrivateMsg` 的 markdown 分支在 `ResolveKeyboardImages` 前补调 `idmap.ResolveOriginalID(UserID)` + `ResolvePlaceholderUserIDs(kb, userOpenID)`，与同文件独立 `[CQ:keyboard]` 路径及 `generatePrivateMessage`（定义于 `send_group_msg.go`、私聊循环路径使用）行为对齐
- **影响**：内嵌键盘与独立 `[CQ:keyboard]` 行为一致——`specify_user_ids` 非空且 `permission.type=0` 时转为 `type=2`（所有人），`specify_user_ids` 中 `__USER_ID__` 占位符替换为实际用户 OpenID；`specify_user_ids` 列表保留不清空
- **测试**：新增回归测试 `TestSendPrivateMsgEmbeddedKeyboardPermissionType`（`handlers/send_msg_md_gate_test.go`），构造 markdown 段内嵌 keyboard（`permission.type=0` + `specify_user_ids`），断言出站 `MsgType=2`、`Permission.Type=2`（修复前为 0）、`specify_user_ids` 保留非空

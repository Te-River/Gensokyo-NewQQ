# Changelog — Release010.001 (Guild-Version)

> Guild-Version 分支自基线 `dbdd193` 以来的变更。
> 本分支保留频道（QQ Guild）功能；后续迭代在本文档基础上递增：`CHANGELOG_v010_002.md`、`CHANGELOG_v010_003.md` ...

---

## 🚀 新增功能

### get_group_info 接入官方群信息 API

**文件：** `botgo/dto/group.go`（新建）、`botgo/openapi/iface.go`、`botgo/openapi/v1/group.go`（新建）、`botgo/openapi/v2/group.go`（新建）、`botgo/openapi/v2/resource.go`、`handlers/get_group_info.go`

按官方接口 `GET /v2/groups/{group_openid}/info`（获取群基本信息）接入真实群信息：

- botgo SDK 新增 `dto.GroupInfo` 结构体（`group_openid`/`group_name`/`group_finger_memo`/`group_class_text`/`group_tags`/`group_member_num`）
- `MessageAPI` 新增 `GroupInfo` 方法，v1 桩实现（不支持）、v2 完整实现，URI 常量 `groupInfoURI`
- `handlers/get_group_info.go` 的 default 分支（真实群聊）由写死的假数据"测试群"改为调用 `apiv2.GroupInfo`，返回真实群名、群简介、成员数
- 虚拟群 ID 自动反查真实 OpenID（32 位原生 OpenID 直接使用），反查失败 / API 调用失败时回传错误响应（`sendGroupInfoError`）
- **频道路径保持不变**：`guild` / `guild_private` 分支仍走 `ConvertGuildToGroupInfo`，频道功能无回归

### 频道创建事件上报（GUILD_CREATE）

**文件：** `Processor/ProcessGuildCreate.go`（新建）、`main.go`

机器人被加入频道时触发 `GUILD_CREATE` 事件，新增上报：

- `Processor/ProcessGuildCreate.go` 构造 OneBot notice 事件（`notice_type=guild_create`），包含频道虚拟 `group_id`、真实频道 ID、频道名称、成员数、成员上限、简介、操作人虚拟 ID
- `main.go` 的 `GuildEventHandler` 按 `event.Type == dto.EventGuildCreate` 分发到 `ProcessGuildCreate`
- 与既有 `ProcessGuildMember`（频道成员变动）上报模式一致，走 `BroadcastMessageToAll` 广播

---

## 🐛 Bug 修复

### get_friend_list 不再过滤虚拟数字 ID

**文件：** `handlers/get_friend_list.go`

移植主分支修复：原 `!isNumeric(user.UserID)` 过滤把 C2C 私聊用户（虚拟数字 ID）全部滤掉，接口一直返回空列表。移除 `isNumeric` 过滤（顺带删除 `regexp` 导入），仅过滤空 `UserID`，保留全部私聊用户。

### access_token 定时刷新提前 45s 并支持失败快速重试

**文件：** `botgo/token/authtoken.go`

移植主分支修复：由等满整个 TTL（7200s）才刷新改为提前 45s 刷新（旧 token 在该窗口内仍有效），刷新失败后 30s 快速重试，消除最长 2 小时空窗期内 WS 鉴权 4004 掉线问题；新增刷新计划 / 刷新成功日志（token 沿用脱敏）。

### QQ 网关鉴权失败无限重连与 access_token 获取健壮性

**文件：** `botgo/token/authtoken.go`、`botgo/token/token.go`、`botgo/token/authtoken_test.go`（新建）、`botgo/dto/websocket.go`、`botgo/sessions/local/local.go`、`botgo/sessions/multi/multi.go`、`botgo/websocket/client/client.go`、`botgo/websocket/client/client_test.go`（新建）

移植主分支修复：

- access_token 响应解析兼容扁平与 `data` 信封结构，校验 HTTP 状态码与错误包络（`code`/`message`），获取失败不再静默写入空 token（初次重试 3 次后返回明确错误）
- `InitToken` 失败向上传播，启动阶段即提示无法获取 AccessToken，避免空 token 进入 ws 鉴权
- Identify/Resume 日志 token 字段脱敏（`redactTokenInJSON`）；连续 3 次鉴权失败（InvalidSession d=false / close 4004）终止重连并提示检查配置
- local/multi session 管理器对不可鉴权错误（含 4914/4915）不再放回队列无限重连

### 鉴权失败达到上限后自动整轮重试

**文件：** `botgo/dto/websocket.go`、`botgo/sessions/manager/manager.go`、`botgo/sessions/local/local.go`、`botgo/sessions/multi/multi.go`

移植主分支修复：`dto.Session` 新增 `AuthFailCount`/`AuthRetryCount` 字段，sessions/manager 新增 `MaxAuthRetryCount=5` 常量；未达上限时清零 `AuthFailCount` 并将 session 放回队列自动从头整轮重试，覆盖停电重启后陈旧 token 的瞬时鉴权失败；满 5 轮仍未成功才真正停止并输出提示。

### Windows 下 Markdown/Keyboard 本地图片 file:/// 路径解析错误

**文件：** `handlers/message_parser.go`

移植主分支修复：`ResolveMarkdownImages` 与 `ResolveKeyboardImages` 三处统一改用 `trimFilePrefix()`（Windows 剥 `file:///`、Unix 剥 `file://`），修复 Windows 上 `file:///C:/...` 残留前导斜杠导致本地图片读取失败、无法上传 CDN 的问题。

---

## 📦 文件变更清单

| 文件 | 变更 |
|------|------|
| `botgo/dto/group.go` | 新建，`GroupInfo` 群基本信息结构体（对应官方接口 GET /v2/groups/{group_openid}/info） |
| `botgo/openapi/iface.go` | `MessageAPI` 新增 `GroupInfo` 方法 |
| `botgo/openapi/v1/group.go` | 新建，`GroupInfo` v1 桩实现（不支持） |
| `botgo/openapi/v2/group.go` | 新建，`GroupInfo` v2 实现 |
| `botgo/openapi/v2/resource.go` | 新增 `groupInfoURI` 常量 |
| `handlers/get_group_info.go` | default 分支接入 `apiv2.GroupInfo` 真实群信息，新增 `sendGroupInfoError`；频道路径保留 |
| `Processor/ProcessGuildCreate.go` | 新建，`GUILD_CREATE` 频道创建事件 notice 上报 |
| `main.go` | `GuildEventHandler` 按事件类型分发 `GUILD_CREATE` |
| `handlers/get_friend_list.go` | 移除 `isNumeric` 过滤，保留全部私聊用户（含虚拟数字 ID） |
| `handlers/message_parser.go` | `ResolveMarkdownImages`/`ResolveKeyboardImages` 三处改用 `trimFilePrefix()` |
| `botgo/token/authtoken.go` | access_token 提前 45s 刷新 + 失败 30s 快速重试 + 信封结构兼容 |
| `botgo/token/token.go` | `InitToken` 失败向上传播 |
| `botgo/token/authtoken_test.go` | 新建，access_token 解析与日志脱敏单元测试 |
| `botgo/dto/websocket.go` | `Session` 新增 `AuthFailCount`/`AuthRetryCount` 字段 |
| `botgo/sessions/manager/manager.go` | 新增 `MaxAuthRetryCount=5` 常量 |
| `botgo/sessions/local/local.go` | 鉴权失败未达上限时自动整轮重试 |
| `botgo/sessions/multi/multi.go` | 鉴权失败未达上限时自动整轮重试 |
| `botgo/websocket/client/client.go` | 连续鉴权失败计数 + token 日志脱敏 |
| `botgo/websocket/client/client_test.go` | 新建，鉴权失败计数单元测试 |
| `release_log/CHANGELOG_v010_001.md` | 新建，本文档 |

---

## 📝 文档

- **新建** `release_log/CHANGELOG_v010_001.md` — 本迭代变更记录（Guild-Version 分支专用，后续迭代为 `CHANGELOG_v010_002.md` 依次递增）

---

## ✅ 提交记录

```
1d1cb56  feat: 移植主分支必要修复并新增群信息 API 与频道创建事件上报
```

# Changelog — Release013

> 自 Release012 以来的所有变更。

---

## 🐛 Bug 修复

### QQ WebSocket 鉴权失败无限重连 + access_token 获取健壮性修复

修复连接 QQ 官方网关时 `InvalidSession(d=false)` + `close 4004 Authentication fail` 无限循环的问题：

- `botgo/token/authtoken.go`：access_token 响应解析兼容扁平结构（`{"access_token":...,"expires_in":7200}`）与 api-v2 `data` 信封结构，校验 HTTP 状态码与错误包络（`code`/`message`），缺失 `expires_in` 时默认 7200s；获取失败不再静默写入空 token，初次获取失败重试 3 次后向上返回明确错误（含响应体便于排查）
- `botgo/token/token.go`：`InitToken` 失败时向调用方返回错误，启动阶段即提示"无法获取AccessToken"并终止，而不是带着空 token 进入 4004 无限重连
- `botgo/websocket/client/client.go`：Identify/Resume 日志对 token 字段脱敏（补全 GSK-006 遗漏路径）；连续 3 次鉴权失败（InvalidSession `d=false` / close 4004）后转为不可鉴权错误并终止重连，提示检查 `appid/client_secret/token` 配置与机器人类型；鉴权成功（Ready）后重置失败计数
- `botgo/sessions/local/local.go` / `botgo/sessions/multi/multi.go`：不可鉴权错误（含 4914/4915 封禁下架）不再把 session 放回队列无限重连，输出明确的配置排查提示后停止该分片

### SSM 补发链入队路径补齐共享关联标识日志

- `echo.PushGlobalStack` 内部新增 `[SSM][cid] 已入队 group=<群> 队列长度=<n>` 日志，入队事件不再依赖调用方样板日志，任何入队路径（含未来新增调用点）均可通过共享关联标识观测
- `echo.NextSSMCorrelationID` 对短 groupID 做安全截断（长度不足 8 时取全部），避免切片越界 panic
- 入队（`PushGlobalStack`）与补发（`SendStackMessages`）两侧日志沿用同一 `CorrelationID`，跨边界关联方式不变，发送与补发行为零改动

---

## ✨ 新功能 / 变更

### set_group 系列 CQ 码统一为 `[CQ:set_group,action=...]`

将原 4 个出站动作 CQ 码合并为 1 个统一 CQ 码，统一参数解析与 ID 反查，CQ 码路径与 OneBot API handler 共享同一底层实现：

- **新 CQ 码格式**：
  - `[CQ:set_group,action=ban,group_id=,user_id=,duration=]`：成员禁言/解禁
  - `[CQ:set_group,action=whole_ban,group_id=,enable=]`：全员禁言开关
  - `[CQ:set_group,action=add_request,group_id=,user_id=,flag=,approve=,reason=,add_to_member_blacklist=]`：入群申请审批
  - `[CQ:set_group,action=strategy_execute/strategy_delete,strategy_id=]`：审批策略执行/删除
- **破坏性变更**：旧 CQ 码 `set_group_ban` / `set_group_whole_ban` / `set_group_add_request` / `strategy` 已移除，插件需迁移（迁移对照见 [`docs/cq码/扩展CQ码/扩展cq码-cq-set_group.md`](../docs/cq码/扩展CQ码/扩展cq码-cq-set_group.md)）
- **新增 `cqParseParams`**：顺序无关的 `key=value` 参数解析器（handlers/cqcode.go），替换 4 个旧函数各自重复的解析循环
- **新增 `handlers/set_group_helpers.go`** 共享底层：`resolveGroupOpenID` / `resolveMemberOpenID` / `applyRestrictChatSetting` / `approveJoinRequest`，CQ 码动作与 `set_group_ban` / `set_group_whole_ban` / `set_group_add_request` handler 共用
- **消息段路径补齐**：`handlers/message_parser.go` 的 `[]interface{}` 与 map（TRSS）两条路径新增 `case "set_group"`，消息段自动还原为 CQ 码字符串后走统一分发；顺带补齐 `member` 在 TRSS map 路径缺失的 case
- **OneBot API 兼容**：`/set_group_ban`、`/get_group_ban`、`/set_group_whole_ban`、`/get_group_whole_ban`、`/set_group_add_request` 注册名与参数不变，仅内部实现切换为共享底层
- 文档：新增 `扩展cq码-cq-set_group.md`（含参数详解/数据流/迁移指南/FAQ），删除 4 个旧 CQ 码文档，同步更新 `readme.md`、`CQ码汇总.md`、`api介绍.md`、`本版新增功能.md`

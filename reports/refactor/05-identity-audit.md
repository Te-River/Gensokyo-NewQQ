# 身份与 ID 语义审计

## 当前路径

- `idmap/map_service.go` 已存在带 `openid:`、`rUIN-`、`vuin` 语义的存储/检索辅助。
- `message_parser.go:133-165` 处理 self-at 和入站 ID 映射；`ConvertToSegmentedMessage`、`ResolveMarkdownAtMentions`、`ProcessCQMemberOutbound` 分别处理不同消息形态。
- `send_private_msg.go`、`send_private_msg_sse.go`、`send_private_msg_wakeup.go`、`send_group_msg.go` 多处直接使用 `len(id)==32` 或 `len(id)!=32` 推断 OpenID、虚拟 ID 或群私聊类型。
- `callapi.ActionMessage` 的 `GroupID`、`UserID`、`Message`、`Messages` 仍为 `interface{}`，类型安全依赖运行时断言。

## 已确认问题

1. **身份类型没有编译期区分**：同一个字符串可能代表 OpenID、虚拟 UIN、群 ID、私聊目标或兼容 ID。
2. **长度启发式重复存在**：`send_private_msg.go:86,111,146` 等路径根据 32 字符判断身份，不是显式解析器。
3. **映射 fallback 分散**：存在 `RetrieveRowByIDv2`、`RetrieveRowByIDv2Pro` 等不同回退路径，维护者需要同时理解多个存储版本。
4. **特殊值存在**：私聊映射路径使用字符串魔法值作为类型/所有者语义，增加误用风险。

## 风险等级

- `P1 / 结构性风险`：长度判断在 ID 格式扩展、第三方 ID 长度变化或错误输入时可能把目标识别为错误类型。
- `P1 / 兼容风险`：多种入站消息形态分别做 self-at、群成员和 Markdown mention 解析，语义可能不一致。
- 运行时是否已触发错误映射：**UNVERIFIED**，本轮未做真实 QQ/OneBot 端到端矩阵测试。

## 目标模型

定义 `Identity` 枚举/联合类型，例如 `OpenID`、`VirtualUserID`、`VirtualGroupID`、`UIN`、`AppID`，并提供显式 `ParseIdentity`、`ResolveIdentity`、`ResolveMentionTarget`。handler 不再自行检查长度；兼容输入在边界适配层转换。

## 迁移顺序

先增加纯函数解析器和表驱动测试，再把 send private/group 接入；随后统一入站 mention 与出站 mention；最后收敛旧 idmap API。没有完成兼容测试前不得删除长度路径。

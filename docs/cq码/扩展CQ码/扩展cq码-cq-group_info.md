# [CQ:group_info]

> 🆕 **新增扩展码**（2026-09，随 cqparse 统一解析架构发布）

## 用途

`[CQ:group_info]` 用于在出站消息文本中展开当前群（或指定群）的信息：群名、群简报（公告）、成员数，或三者合一的 JSON。适合做群名片欢迎语、状态播报等场景。

范围：`q群 (Group Chat)` / `C2C (私聊)`（私聊需显式 `group_id`）

> ⚠️ **解析模式要求**：本码需 `cq_parse_mode: new`；`legacy`/`shadow` 模式下该码不被识别，会原文发出。

## 语法

```text
[CQ:group_info,field=name]
[CQ:group_info,field=memo]
[CQ:group_info,field=member_count]
[CQ:group_info,field=all]
[CQ:group_info,field=name,group_id=12345]
[CQ:group_info,field=name,fallback=暂无群名]
```

| 参数 | 必填 | 说明 |
|------|------|------|
| `field` | 是 | 展开字段：`name`（群名）/ `memo`（群简报）/ `member_count`（成员数）/ `all`（三字段 JSON） |
| `group_id` | 否 | 目标群：省略时回退当前会话群；支持虚拟数字 ID 与 32 位原生 OpenID 双格式 |
| `fallback` | 否 | 取数失败时的替换文本，默认空串；值中的逗号请转义为 `&#44;` |

## 替换行为

- CQ 码**从正文中移除**，替换为对应字段值（是内容展开码，不是媒体码）。
- 同一条消息内多个 `[CQ:group_info]` 指向同一目标群时，**合并为一次**群信息 API 调用（30 QPM 限频保护），多码共享同一次取数结果。
- 消息段数组格式（`{"type":"group_info","data":{"field":"name"}}`）同样支持，替换值按段位置展开。

### field 取值示例

| field | 示例输出 |
|-------|----------|
| `name` | `幻想乡` |
| `memo` | `欢迎加入` |
| `member_count` | `42` |
| `all` | `{"member_count":"42","memo":"欢迎加入","name":"幻想乡"}` |

## 失败分级

| 场景 | 行为 |
|------|------|
| `field` 缺失或非法 | CQ 码原样保留在文本中，记录警告日志（诚实暴露无效用法） |
| 显式 `group_id` 反查失败 | 替换为 `fallback`（默认空串），记录日志 |
| 群信息 API 返回错误 / 网络失败 | 替换为 `fallback`（默认空串），记录日志（含错误信息、field、目标群） |
| 私聊消息省略 `group_id` | 替换为 `fallback`（默认空串），记录日志 |

## 使用示例

NoneBot 插件发送：

```python
await bot.send_group_msg(
    group_id=event.group_id,
    message="[CQ:group_info,field=name] 当前共有 [CQ:group_info,field=member_count] 位成员"
)
# 实际发出："幻想乡 当前共有 42 位成员"
```

指定群（跨群播报）：

```text
[CQ:group_info,field=name,group_id=12345] 的公告：[CQ:group_info,field=memo,group_id=12345]
```

## 常见问题

**Q: 为什么取数失败默认替换为空串而不是保留码？**
A: 保留码会把 `[CQ:group_info,...]` 原文发到群里造成泄漏；空串 + 日志是更诚实的行为。需要占位提示时请显式配置 `fallback`。

**Q: 频繁使用会触发官方限频吗？**
A: 同消息同群多码只取数一次；不同消息多次使用受官方群信息接口限频（30 QPM）约束，请避免在循环群发场景高频使用。

# botgo Fork Inventory

> 本地 fork 依赖：`go.mod` 中 `replace github.com/tencent-connect/botgo => ./botgo`

## 基本信息

| 项 | 值 |
|----|----|
| upstream | https://github.com/tencent-connect/botgo |
| 本地 module | `github.com/tencent-connect/botgo`（路径 `./botgo`） |
| 用途 | QQ 开放平台机器人 SDK（OpenAPI + WebSocket gateway） |
| 引用方式 | go.mod `replace`（本地目录，非发布版本） |

## 为什么需要 fork

官方 SDK 未暴露部分自定义事件类型（群消息、C2C、好友等），本仓库需要这些事件才能
实现 OneBot 协议的入站映射。

## 本地 patch 内容（已知）

- 自定义事件类型：群消息（`GROUP_AT_MESSAGE_CREATE` / `GROUP_MESSAGE_CREATE`）、
  C2C、好友添加/删除等官方 SDK 未暴露的事件。
- `dto/` 扩展：`User` 等结构补充 OpenID 相关字段。

> ⚠️ 完整本地 patch 清单需维护者补齐（相对 upstream 的 diff）。

## 未来同步方式

- 定期对比 upstream，仅将官方新增的稳定改动合并回本地 fork（`git fetch upstream` + cherry-pick）。
- 若官方 SDK 最终暴露所需事件，应评估移除 fork、回到 upstream 依赖。

## 参考

- `botgo/README.md`
- `botgo/DEVELOP.md`

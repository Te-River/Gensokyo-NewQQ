# 文件规模与复杂度报告

## 总量

- Go 文件：226 个。
- Go 源码：85,477 行，3,353,031 字节。
- 根模块包：27 个；模块条目：134 个。

## 最大文件

| 文件 | 行数 | 备注 |
|---|---:|---|
| `go-silk/sdk/skype_silk_sdk_32.go` | 21,712 | 生成/绑定 SDK，优先隔离，不作为首个业务拆分目标 |
| `go-silk/sdk/skype_silk_sdk_64.go` | 17,267 | 生成/绑定 SDK，优先隔离 |
| `handlers/send_group_msg.go` | 3,100 | 业务核心热点 |
| `config/config.go` | 3,058 | 全局配置、读写、补全、重载混合 |
| `handlers/message_parser.go` | 2,860 | 多种消息输入形态及媒体解析混合 |
| `proto/idmap.pb.go` | 2,525 | 生成代码 |
| `idmap/service.go` | 2,466 | 身份映射、HTTP、存储与清理混合 |
| `Processor/Processor.go` | 1,323 | 事件分发、广播、HTTP/WS 发送混合 |
| `main.go` | 1,112 | 启动、配置、服务器、生命周期混合 |
| `idmap/new_service.go` | 1,089 | 身份映射服务热点 |
| `idmap/map_service.go` | 915 | 身份映射兼容路径 |
| `handlers/send_private_msg.go` | 866 | 私聊发送与媒体/重试混合 |
| `httpapi/httpapi.go` | 851 | HTTP API 路由与请求处理 |

## AST 复杂度热点

以下为 Go AST 统计的函数范围、行数和分支复杂度，复杂度用于排序，不是运行时性能指标。

| 函数 | 范围 | 行数 | 复杂度 |
|---|---|---:|---:|
| `handlers.HandleSendGroupMsg` | `send_group_msg.go:77-975` | 899 | 227 |
| `handlers.parseMessageContent` | `message_parser.go:676-1414` | 739 | 181 |
| `handlers.generatePrivateMessage` | `send_group_msg.go:1758-2496` | 739 | 121 |
| `handlers.generateGroupMessage` | `send_group_msg.go:1019-1755` | 737 | 123 |
| `handlers.HandleSendPrivateMsg` | `send_private_msg.go:50-659` | 610 | 147 |
| `main` | `main.go:55-664` | 610 | 92 |
| `Processor.ProcessInlineSearch` | `ProcessInlineSearch.go:31-639` | 609 | 57 |
| `handlers.HandleSendGroupMsgRaw` | `send_group_msg_raw.go:24-495` | 472 | 110 |
| `Processor.ProcessGroupMessage` | `ProcessGroupMessage.go:23-388` | 366 | 53 |
| `Processor.ProcessGroupNormalMessage` | `ProcessGroupNormalMessage.go:25-348` | 324 | 60 |

## 拆分优先级

1. 先把 `foundItems`、媒体解析、消息生成、QQ 发送和重试拆成可测试边界。
2. 再处理 `config/config.go` 的配置快照与持久化边界。
3. 生成代码与第三方绑定文件只做目录/依赖隔离，不重写。

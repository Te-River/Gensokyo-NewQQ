# 依赖与模块边界图

## 当前主链路

```text
main
 ├─ config / structs / template / sys
 ├─ botgo
 ├─ Processor
 ├─ server / httpapi / webui
 └─ wsclient

QQ botgo event
 → Processor
 → message_parser / idmap / echo
 → OneBot JSON
 → callapi.Client / wsclient / HTTP reverse post

OneBot action
 → httpapi or WebSocket
 → callapi.ActionMessage
 → init-registered handler
 → parseMessageContent
 → foundItems
 → generateGroupMessage or generatePrivateMessage
 → images / imagehosting / oss / silk / messagequeue / idmap / echo
 → QQ API
```

## 主要包关系

| 上游 | 直接耦合的下游/基础设施 | 观察 |
|---|---|---|
| `main` | `config`, `botgo`, `Processor`, `server`, `httpapi`, `webui`, `wsclient` | 启动编排与基础设施组装集中在一个入口 |
| `Processor` | `handlers`, `callapi`, `config`, `echo`, `idmap`, `images`, `wsclient`, `botgo` | 事件处理、广播、协议转换和外发混合 |
| `handlers` | `callapi`, `config`, `echo`, `idmap`, `imagehosting`, `images`, `messagequeue`, `silk`, `botgo` | 业务用例同时依赖协议、全局配置、HTTP、媒体和 QQ SDK |
| `config` | `structs`, `template`, `sys`, YAML/fsnotify | 全局 singleton、配置补全、持久化、重载混合 |
| `idmap` | bbolt、gRPC、HTTP、`config` | 本地身份存储和外部查询混合 |
| `echo` | `config`, DTO、`sync.Map` | echo 与消息序号/生命周期清理混合 |
| `server`/`httpapi` | `config`, `callapi`, `handlers`, gin/HTTP | 传输协议和应用动作路由未完全分层 |
| `wsclient` | gorilla WebSocket、`config`, `callapi` | 连接状态、重连、发送队列混合 |
| `images`/`imagehosting`/`oss`/`silk` | HTTP、文件、配置、外部供应商 | 媒体内容的读取、变换和上传边界分散 |

## 依赖规则建议

目标结构中 `domain` 不依赖 botgo、gin、gorilla、bbolt、全局 config 或 HTTP；`adapters` 实现端口，`application` 编排用例，`infrastructure` 提供持久化/网络/媒体实现。迁移期间允许旧 handler 作为兼容入口，但新增代码不得继续扩大跨层依赖。

## 结构性风险

- `callapi.ActionMessage` 的 ID、消息和参数使用 `interface{}`，解析正确性依靠运行时类型断言。
- handler 通过 `init()` 注册，隐式依赖导入副作用，难以从包图上看出完整注册关系。
- `foundItems` 是跨 parser、sender、media、retry 的隐式协议，不受编译器约束。

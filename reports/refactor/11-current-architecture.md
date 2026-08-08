# 当前架构说明

## 入口与生命周期

`main.go` 负责加载配置、启动 watcher、初始化 botgo/logger/WebUI/API client、创建服务器并启动后台任务。服务器、WebSocket、反向 HTTP、QQ SDK 和全局配置均在入口附近组装。

## 入站 QQ 事件

```text
botgo event
 → Processor 的事件处理函数
 → mention / idmap / echo 处理
 → ConvertToSegmentedMessage 或字符串化 OneBot 消息
 → 广播到 reverse WebSocket、forward WebSocket、HTTP POST
```

`Processor` 同时持有 WebSocket client 列表和发送/广播逻辑；广播包含异步 fire-and-forget 路径与等待路径。

## 出站 OneBot action

```text
HTTP API 或 WebSocket
 → callapi.ActionMessage
 → callapi handler registry
 → HandleSendGroupMsg / HandleSendPrivateMsg / raw / wakeup
 → parseMessageContent
 → foundItems
 → generateGroupMessage / generatePrivateMessage
 → media、idmap、echo、queue、retry
 → botgo QQ API
```

`callapi` 通过 `init()` 注册 action handler；`ActionMessage` 的多项参数为 `interface{}`。这使传输层、协议层和业务 handler 之间缺少显式契约。

## 当前边界判断

- 真实 domain 层尚未形成；核心规则散落在 Processor、handlers、idmap、echo 和 config。
- parser 不只是语法解析，还会参与 URL/媒体/身份处理。
- handler 同时承担用例编排、请求参数校验、媒体转换、QQ API 调用、错误格式化和重试。
- config 是全局可变 singleton，业务层可直接读取。
- 生成代码和第三方 SDK 位于同一工作树，依赖图噪声较大。

## 主要架构热点

1. `handlers/send_group_msg.go`：群/私消息生成、媒体和重试。
2. `handlers/message_parser.go`：多输入形态与隐式 `foundItems` 协议。
3. `config/config.go`：配置 singleton、补全、写入、watcher 配合。
4. `Processor`：事件分发、OneBot 转换和广播生命周期。
5. `idmap`/`echo`：跨请求身份与消息关联状态。

<p align="center">
  <a href="https://www.github.com/hoshinonyaruko/gensokyo">
    <img src="images/head.gif" width="200" height="200" alt="gensokyo">
  </a>
</p>

<div align="center">

# gensokyo

_✨ 基于 [OneBot](https://github.com/howmanybots/onebot/blob/master/README.md) QQ官方机器人Api Golang 原生实现 ✨_  

</div>

<p align="center">
  <a href="https://raw.githubusercontent.com/hoshinonyaruko/gensokyo/main/LICENSE">
    <img src="https://img.shields.io/github/license/hoshinonyaruko/gensokyo" alt="license">
  </a>
  <a href="https://github.com/hoshinonyaruko/gensokyo/releases">
    <img src="https://img.shields.io/github/v/release/hoshinonyaruko/gensokyo?color=blueviolet&include_prereleases" alt="release">
  </a>
  <a href="https://github.com/howmanybots/onebot/blob/master/README.md">
    <img src="https://img.shields.io/badge/OneBot-v11-blue?style=flat&logo=data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAEAAAABABAMAAABYR2ztAAAAIVBMVEUAAAAAAAADAwMHBwceHh4UFBQNDQ0ZGRkoKCgvLy8iIiLWSdWYAAAAAXRSTlMAQObYZgAAAQVJREFUSMftlM0RgjAQhV+0ATYK6i1Xb+iMd0qgBEqgBEuwBOxU2QDKsjvojQPvkJ/ZL5sXkgWrFirK4MibYUdE3OR2nEpuKz1/q8CdNxNQgthZCXYVLjyoDQftaKuniHHWRnPh2GCUetR2/9HsMAXyUT4/3UHwtQT2AggSCGKeSAsFnxBIOuAggdh3AKTL7pDuCyABcMb0aQP7aM4AnAbc/wHwA5D2wDHTTe56gIIOUA/4YYV2e1sg713PXdZJAuncdZMAGkAukU9OAn40O849+0ornPwT93rphWF0mgAbauUrEOthlX8Zu7P5A6kZyKCJy75hhw1Mgr9RAUvX7A3csGqZegEdniCx30c3agAAAABJRU5ErkJggg==" alt="gensokyo">
  </a>
  <a href="https://github.com/hoshinonyaruko/gensokyo/actions">
    <img src="images/badge.svg" alt="action">
  </a>
  <a href="https://goreportcard.com/report/github.com/hoshinonyaruko/gensokyo">
  <img src="https://goreportcard.com/badge/github.com/hoshinonyaruko/gensokyo" alt="GoReportCard">
  </a>
</p>

<p align="center">
  <a href="/docs/更多文档.md">文档</a>
  ·
  <a href="https://github.com/hoshinonyaruko/gensokyo/releases">下载</a>
  ·
  <a href="/docs/开始使用.md">开始使用</a>
  ·
  <a href="https://github.com/hoshinonyaruko/gensokyo/blob/master/CONTRIBUTING.md">参与贡献</a>
</p>
<p align="center">
  <a href="https://gensokyo.bot">项目主页:gensokyo.bot</a>
</p>

## 介绍

Gensokyo 是一款兼容 [OneBot V11](https://github.com/botuniverse/onebot-11) 标准的 QQ 机器人服务端，将 QQ 官方 API 和 WebSocket 事件和 HTTP 接口转换为 OneBot V11 协议。

**支持连接的客户端框架：** Koishi、NoneBot2、Trss、Zerobot、MiraiCQ、Hoshino、Tata、派蒙、炸毛、早苗、Yobot、Mirai(Overflow) 等所有支持 OneBot V11 适配器的项目。

实现插件开发和用户开发者无需重新开发，**复用过往生态的插件和使用体验**。

原版交流群：**196173384**

## 快速开始

1. 前往 [Releases](https://github.com/hoshinonyaruko/gensokyo/releases) 下载对应系统的二进制文件
2. 参考[开始使用](/docs/开始使用.md) 创建机器人并配置
3. 启动 gensokyo，连接你的 OneBot V11 客户端

## 功能亮点

-  兼容 OneBot V11 的 HTTP API、反向 HTTP POST、正向 WebSocket、反向 WebSocket
-  群非 @ 消息支持（GroupMessageEventHandler）
-  q群 (Group Chat) 消息自动剔除 @机器人字符
-  非自身 @ 可配置转换为已有 idmap 的 `[CQ:at,qq=虚拟ID]` 格式
-  按钮权限中虚拟数字 ID 自动转化为 QQ 官方 OpenID
-  扩展 API：`get_avatar`（获取头像直链）
-  event_id 存储，支持被动消息
-  消息事件新增 `to_me` 字段，标识是否 @ 了机器人
-  多 WS 地址连接
-  q頻 (QQ Guild) 虚拟成 q群 事件、私信虚拟成 q頻 事件
-  WebUI 管理界面
-  指令黑白名单、URL 自动转换
-  可自定义图片压缩/图床服务
-  支持文字、图片、语音、视频、Markdown 等多种消息类型
-  主动信息失败自动转被动
-  完善的重连机制

## 文档

- [开始使用](/docs/开始使用.md) — 注册机器人、配置、启动
- [本版新增功能](/docs/本版新增功能.md) — 本版新增功能
- [API 介绍](/docs/api/api介绍.md) — 支持的 API 列表与扩展
- [扩展 API](/docs/api/扩展API文档.md) — 扩展 API 文档索引
- [Markdown 消息](/docs/文档-markdown消息.md) — Markdown 卡片消息说明
- [扩展 CQ 码](/docs/cq码/扩展CQ码汇总.md) — 本 Fork 新增 CQ 码
- [更多文档](/docs/更多文档.md) — 完整文档索引

## CQ 码与 API 支持

<details>
<summary>已实现 CQ 码</summary>

> 新增了如下 **CQ码**</br>
> -------
> - **[CQ:avatar]** 头像获取
> - **[CQ:markdown]** Markdown 卡片消息
> - **[CQ:member]** 群成员变动
> - **[CQ:active,type=...,sub_type=...]** active 标记

#### 符合 OneBot 标准的 CQ 码

| CQ 码 | 功能 |
| ------------ | --------------------------- |
| [CQ:face]    | [QQ 表情]                   |
| [CQ:record]  | [语音]                      |
| [CQ:video]   | [短视频]                    |
| [CQ:at]      | [@某人]                     |
| [CQ:share]   | [链接分享]                  |
| [CQ:music]   | [音乐分享] [音乐自定义分享] |
| [CQ:reply]   | [回复]                      |
| [CQ:forward] | [合并转发]                  |
| [CQ:node]    | [合并转发节点]              |
| [CQ:xml]     | [XML 消息]                  |
| [CQ:json]    | [JSON 消息]                 |

todo,正在施工...

#### 拓展 CQ 码及与 OneBot 标准有略微差异的 CQ 码

| 拓展 CQ 码 | 功能 |
| -------------- | --------------------------------- |
| [CQ:image]     | [图片]                            |
| [CQ:poke]      | [戳一戳]                          |
| [CQ:node]      | [合并转发消息节点]                |
| [CQ:markdown]  | [markdown卡片收发] |
| [CQ:avatar]    | [头像获取] |
| [CQ:member]    | [q群成员变动] |
| [CQ:active]    | [active 标记] |
| [CQ:tts]       | [文本转语音]                      |


</details>

<details>
<summary>已实现 API</summary>

> 新增了如下 **API**</br>
> -------
> - **get_avatar**（获取头像直链）
> - **get_robot_share_link**（获取分享链接）
> - **send_private_msg_wakeup**（C2C 召回消息）
> - **send_private_msg_sse**（SSE 私聊）
> - **put_interaction**（处理按钮回调）
> - **get_group_ban**（查询 q群 禁言）
> - **get_group_whole_ban**（查询 q群 全员禁言）
> - **send_to_group**（send_group_msg 别名）


#### 符合 OneBot 标准的 API

| API                      | 功能                   |
| ------------------------ | ---------------------- |
| /send_private_msg | [发送私聊 (C2C) 消息] |
| /send_group_msg | [发送 q群 (Group Chat) 消息] |
| /send_guild_channel_msg | [发送 q頻 (QQ Guild) 消息] |
| /send_msg | [发送消息] |
| /delete_msg              | [撤回信息]             |
| /delete_group_msg        | [撤回QQ群用户或Bot消息] |
| /set_group_kick          | [群 (Group Chat) 踢人] |
| /set_group_ban | [群单人禁言] |
| /set_group_whole_ban | [群全员禁言] |
| /set_group_admin         | [群设置管理员] |
| /set_group_card          | [设置群名片] |
| /set_group_name          | [设置群名称] |
| /set_group_leave         | [退出群] |
| /set_group_special_title | [设置群专属头衔] |
| /set_friend_add_request  | [处理加好友请求]       |
| /set_group_add_request   | [处理加群请求/邀请] |
| /get_login_info | [获取登录号信息] |
| /get_stranger_info | [获取陌生人信息] |
| /get_friend_list | [获取好友列表] |
| /get_group_info | [获取群聊/频道信息] |
| /get_group_list | [获取群列表] |
| /get_group_member_info | [获取群成员信息] |
| /get_group_member_list | [获取群成员列表] |
| /get_group_honor_info    | [获取群荣誉信息] |
| /can_send_image | [检查是否可以发送图片] |
| /can_send_record | [检查是否可以发送语音] |
| /get_version_info | [获取版本信息] |
| /set_restart | [重启 Gensokyo] |
| /.handle_quick_operation | [对事件执行快速操作]   |


#### 拓展 API 及与 OneBot 标准有略微差异的 API

| 拓展 API                    | 功能                   |
| --------------------------- | ---------------------- |
| /set_group_portrait         | [设置群头像]           |
| /get_image                  | [获取图片信息]         |
| /get_msg                    | [获取消息]             |
| /get_forward_msg            | [获取合并转发内容]     |
| /send_group_forward_msg | [发送合并转发] |
| /.get_word_slices           | [获取中文分词]         |
| /.ocr_image                 | [图片 OCR]             |
| /get_group_system_msg       | [获取群系统消息]       |
| /get_group_file_system_info | [获取群文件系统信息]   |
| /get_group_root_files       | [获取群根目录文件列表] |
| /get_group_files_by_folder  | [获取群子目录文件列表] |
| /get_group_file_url         | [获取群文件资源链接]   |
| /get_status | [获取状态] |


</details>

<details>
<summary>已实现 Event</summary>

> 新增了如下 **Event**</br>
> -------
> - **friend_decrease**（好友删除）
> - **friend_increase**（好友新增）
> - **group_reject**（群推送关闭）
> - **group_receive**（群推送开启）
> - **interaction**（按钮回调）
> - **group_increase**（群成员新增，标准 OneBot V11 notice）
> - **group_decrease**（q群成员移除，标准 OneBot V11 notice）

#### 符合 OneBot 标准的 Event（部分 Event 比 OneBot 标准多上报几个字段，不影响使用）

| 事件类型 | Event            |
| -------- | ---------------- |
| 消息事件 | [私聊信息] |
| 消息事件 | [群消息] |
| 通知事件 | [群文件上传]   |
| 通知事件 | [群管理员变动] |
| 通知事件 | [群成员减少]   |
| 通知事件 | [群成员增加]   |
| 通知事件 | [群禁言]       |
| 通知事件 | [好友添加]       |
| 通知事件 | [好友删除]       |
| 通知事件 | [群消息撤回]   |
| 通知事件 | [好友消息撤回]   |
| 通知事件 | [群内戳一戳]   |
| 通知事件 | [群红包运气王] |
| 通知事件 | [群成员荣誉变更] |
| 通知事件 | [群消息推送关闭] |
| 通知事件 | [群消息推送开启] |
| 请求事件 | [加好友请求]     |
| 请求事件 | [加群请求/邀请] |


#### 拓展 Event

| 事件类型 | 拓展 Event       |
| -------- | ---------------- |
| 通知事件 | [好友戳一戳]     |
| 通知事件 | [群内戳一戳]   |
| 通知事件 | [群成员名片更新] |
| 通知事件 | [接收到离线文件] |
| 通知事件 | [按钮交互回调] |
| 通知事件 | [私聊 (C2C) 消息推送关闭] |
| 通知事件 | [私聊 (C2C) 消息推送开启] |

</details>


<details>
<summary>已实现 Intent</summary>

#### 允许向后端推送的事件类型
> 新增了如下 **Intent**</br>
> -------
> - **GroupMessageEventHandler**（非@群消息）事件
> - **GroupAddRobotEventHandler**（群机器人新增）
> - **GroupDelRobotEventHandler**（群机器人删除）
> - **GroupMsgRejectHandler**（群推送关闭）
> - **GroupMsgReceiveHandler**（群推送开启）
> - **GroupMemberAddEventHandler**（群成员新增）
> - **GroupMemberRemoveEventHandler**（群成员移除）

| 事件名称                   | 代表含义                         |
| --------------------------- | ------------------------------- |
| ATmessageEventHandler      | [频道@ 消息]                       |
| DirectMessageHandler       | [频道私信 dms]                     |
| ReadyHandler               | [连接成功]                         |
| ErrorNotifyHandler         | [连接关闭]                         |
| GuildEventHandler          | [频道事件]                         |
| MemberEventHandler         | [频道成员新增]                     |
| ChannelEventHandler        | [频道子频道事件]                   |
| CreateMessageHandler       | [频道非@ 消息]                    |
| InteractionHandler         | [频道卡片按钮 data 回调事件]       |
| GroupATMessageEventHandler | [群聊@ 消息]                       |
| GroupMessageEventHandler   | [群聊普通消息]                     |
| C2CMessageEventHandler     | [私聊 (C2C)]                       |
| ThreadEventHandler         | [频道发帖事件]                     |
| FriendAddEventHandler      | [被添加好友]                       |
| FriendDelEventHandler      | [被删除好友]                       |
| GroupAddRobotEventHandler  | [群聊机器人新增]                   |
| GroupDelRobotEventHandler  | [群聊机器人删除]                   |
| GroupMsgRejectHandler      | [群聊请求关闭推送]                 |
| GroupMsgReceiveHandler     | [群聊请求开启推送]                 |
| GroupMemberAddEventHandler | [群聊成员新增]                     |
| GroupMemberRemoveEventHandler | [群聊成员移除]                  |
| C2CMsgRejectHandler        | [用户拒绝私聊 (C2C) 消息推送]      |
| C2CMsgReceiveHandler       | [用户同意私聊 (C2C) 消息推送]      |


</details>

## 完整配置示例

首次运行会自动生成 `config.yml`，以下为完整配置项及说明（敏感值已脱敏）：

```yaml
version: 1
settings:
  #── 反向 WebSocket ──────────────────────────────────
  ws_address: ["ws://127.0.0.1:8080/onebot/v11/ws"]   # 后端 OneBot 适配器地址
  ws_token: ["your_ws_token_here"]                    # 对应 ws_address 的 token
  reconnect_times: 100                                 # 断线重连次数
  heart_beat_interval: 5                               # 心跳间隔（秒）
  launch_reconnect_times: 1                            # 启动时重连次数

  #── 基础设置 ────────────────────────────────────────
  app_id: 123456789                                    # QQ 开放平台应用 ID
  uin: 0                                               # 机器人 QQ 号
  use_uin: false                                       # 使用 QQ 号作为 bot ID
  token: "your_app_token"                              # 应用令牌
  client_secret: "your_client_secret"                  # 客户端密钥
  idmap_isolation: false                               # 多 Bot 共用数据库时加 UIN 前缀隔离
  idmap_legacy_compat: false                           # 同时写入旧格式 key 兼容官方版
  shard_count: 1                                       # 分片数量
  shard_id: 0                                          # 当前分片 ID
  shard_num: 1                                         # 限频时可调大，尝试多分片

  #── 事件订阅 ────────────────────────────────────────
  text_intent:
    - "ATMessageEventHandler"                          # q頻 @ 消息
    - "DirectMessageHandler"                           # q頻 私信
    - "ReadyHandler"                                   # 连接成功
    - "ErrorNotifyHandler"                             # 连接关闭
    - "GroupATMessageEventHandler"                     # q群 @ 消息
    - "GroupMessageEventHandler"                       # q群 普通消息
    - "GroupMemberAddEventHandler"                     # q群 成员新增
    - "GroupMemberRemoveEventHandler"                  # q群 成员移除
    - "C2CMessageEventHandler"                         # 私聊 (C2C)
  discover_unknown_events: false                       # 探测未文档化事件
  suppress_disallowed_intents: false                   # 仅屏蔽已在 text_intent 启用的高风险 intent bit, 不自动注册 handler

  #── 消息转换 ────────────────────────────────────────
  global_channel_to_group: true                        # q頻 事件转 q群 事件
  global_private_to_channel: false                     # 私聊转 q頻 事件
  global_forum_to_channel: false                       # 帖子转 q頻 事件
  hash_id: true                                        # 使用 hash 生成虚拟 ID
  op_userid_type: "vuin"                               # 下游 user_id/group_id 来源: vuin/raw/ruin
  convert_other_at: false                              # 非机器人自身的 <@OpenID> 仅在已有 idmap 映射且开启时转换
  msgid_ttl_seconds: 3600                              # message_id 映射保留时间
  array: false                                         # 使用 segment 数组格式上报

  #── Gensokyo 互联 ───────────────────────────────────
  server_dir: "your_server_ip_or_domain"               # Lotus 地址
  port: "15630"                                        # HTTP 服务端口
  lotus: false                                         # 启用 Lotus 模式
  lotus_grpc: false                                    # 使用 gRPC 进行 Lotus 连接

  #── WebSocket ──────────────────────────────────────
  enable_ws_server: true                               # 启用正向 WebSocket
  ws_server_path: "ws"                                 # 正向 WS 路径
  ws_server_token: ""                                  # 正向 WS token

  #── SSL 与域名校验 ─────────────────────────────────
  identify_file: true                                  # 自动生成域名校验文件
  crt: ""                                              # SSL 证书路径
  key: ""                                              # SSL 密钥路径
  force_ssl: false                                     # 强制启用 SSL

  #── HTTP API ───────────────────────────────────────
  http_address: ""                                     # HTTP API 监听地址
  http_access_token: ""                                # HTTP API token
  post_url: [""]                                       # 反向 HTTP POST 地址
  post_secret: [""]                                    # 反向 HTTP POST 密钥

  #── 日志 ────────────────────────────────────────────
  developer_log: false                                 # 开启开发者日志
  log_level: 1                                         # 0=debug 1=info 2=warn 3=error
  save_logs: false                                     # 保存本地日志; 与启动参数 run --local-logger=enable 等效
  log_max_age_days: 7                                  # 本地日志最大保留天数
  log_max_size_mb: 24                                  # 单个日志文件大小上限(MB)
  log_keep_files: 12                                   # 本地旧日志文件最大保留个数

  #── WebUI ───────────────────────────────────────────
  disable_webui: false                                 # 禁用 Web 管理面板
  server_user_name: "admin"                            # 面板用户名
  server_user_password: "admin"                        # 面板密码

  #── 指令控制 ───────────────────────────────────────
  remove_prefix: false                                 # 忽略指令前 /
  remove_at: false                                     # 忽略指令前 @
  white_prefix_mode: false                             # 指令白名单模式
  black_prefix_mode: false                             # 指令黑名单模式
  bind_prefix: "/bind"                                 # 绑定指令
  status_prefix: "/gskstatus"                          # 运行状态查询; 设为 ""、/disabled 或 /disabled... 可禁用
  broadcast_prefix: "/gskbroadcast"                    # 广播指令; 设为 ""、/disabled 或 /disabled... 可禁用

  #── Markdown 消息 ──────────────────────────────────
  twoway_echo: false                                   # 启用双向 echo
  native_md: false                                     # 启用原生 Markdown
  custom_template_id: ""                               # 图文转 MD 模板 ID
  keyboard_id: ""                                      # 图文转 MD 按钮 ID

  #── 消息发送 ──────────────────────────────────────
  memory_msgid: false                                  # 使用内存存储 msg_id
  lazy_message_id: false                               # 惰性 message_id
  send_delay: 300                                      # 发送间隔（毫秒）
  threads_ret_msg: false                               # 异步发送回执
  no_ret_msg: false                                    # 禁用回执（提升性能）

  #── 云存储 ─────────────────────────────────────────
  oss_type: 0                                          # 0=本机 1=腾讯COS 2=百度BOS 3=阿里OSS
  # 腾讯云 COS 配置（oss_type=1 时需填写）
  t_COS_BUCKETNAME: ""
  t_COS_REGION: ""
  t_COS_SECRETID: ""
  t_COS_SECRETKEY: ""
  # 百度云 BOS 配置（oss_type=2 时需填写）
  b_BOS_BUCKETNAME: ""
  b_BCE_AK: ""
  b_BCE_SK: ""
  # 阿里云 OSS 配置（oss_type=3 时需填写）
  a_OSS_EndPoint: ""
  a_OSS_BucketName: ""
  a_OSS_AccessKeyId: ""
  a_OSS_AccessKeySecret: ""
```

> 完整文档请参阅 [docs/开始使用.md](./docs/开始使用.md) 和 [docs/idmap.md](./docs/idmap.md)

## 关于 ISSUE

以下 ISSUE 会被直接关闭

- 提交 BUG 不使用 Template
- 询问已知问题
- 提问找不到重点
- 重复提问

> 请注意：开发者并没有义务回复您的问题。您应该具备基本的提问技巧。 
> 有关如何提问，请阅读[《提问的智慧》](https://github.com/ryanhanwu/How-To-Ask-Questions-The-Smart-Way/blob/main/README-zh_CN.md)

## 性能

10mb内存占用 端口错开可多开 稳定运行无报错

## 特别鸣谢

- [`mnixry/nonebot-plugin-gocqhttp`](https://github.com/mnixry/nonebot-plugin-gocqhttp/): 本项目采用了mnixry编写的前端并实现了与它对应的基于qq官方api的后端api.
- 特别鸣谢[`dk 盾`](https://www.dkdun.cn/),友情赞助服务器资源

## 引用

- [`tencent-connect/botgo`](https://github.com/tencent-connect/botgo): 本项目引用了此项目并做了一些改动

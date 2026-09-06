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
 <a href="https://www.star-history.com/Te-River/Gensokyo-NewQQ">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/badge?repo=Te-River/Gensokyo-NewQQ&theme=dark" />
    <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/badge?repo=Te-River/Gensokyo-NewQQ" />
    <img alt="Star History Rank" src="https://api.star-history.com/badge?repo=Te-River/Gensokyo-NewQQ" />
  </picture>
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
  <a href="https://gsk.mizuki.top">项目主页:gsk.mizuki.top</a>
  ·
  <a href="https://help.mizuki.top">帮助文档</a>
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
-  WebUI 管理界面
-  指令黑白名单、URL 自动转换
-  可自定义图片压缩/图床/OSS 服务（oss_type 统一选择，支持 11 种后端）
-  `[CQ:file]` 文件上传（支持本地路径/HTTP/base64 三种方式）
-  `send_private_msg_wakeup` C2C 互动召回消息
-  `[CQ:active]` 主动消息标记，强制走主动推送通道
-  `[CQ:wakeup,userid=xxx]` C2C 互动召回消息标记（在 send_private_msg 中指定目标用户）
-  `[CQ:card]` 群聊图文卡片消息（msg_type=8）
-  `[CQ:input_notify]` 单聊输入状态通知（msg_type=6）
-  `[CQ:stream]` 单聊流式消息（打字机效果）
-  QQ API 错误码中文提示（控制台直接输出错误码含义和排查建议）
-  idmap 迁移阻塞式 + 强制解绑工具（`getid type=18` 按 OpenID 清理重复映射）
-  支持文字、图片、语音、视频、Markdown 等多种消息类型
-  主动信息失败自动转被动
-  完善的重连机制
-  分层架构基础设施（`internal/` 类型化消息/身份/出站/入站/媒体/队列/配置管线，双轨并存）

## 文档

- [开始使用](/docs/开始使用.md) — 注册机器人、配置、启动
- [本版新增功能](/docs/本版新增功能.md) — 本版新增功能
- [API 介绍](/docs/api/api介绍.md) — 支持的 API 列表与扩展
- [扩展 API](/docs/api/扩展API文档.md) — 扩展 API 文档索引
- [Markdown 消息](/docs/文档-markdown消息.md) — Markdown 卡片消息说明
- [扩展 CQ 码](/docs/cq码/扩展CQ码汇总.md) — 本 Fork 新增 CQ 码
- [标准 CQ 码差异](/docs/cq码/标准CQ码/) — 与标准 OneBot 有差异的 CQ 码
- [图床/OSS 后端](/imagehosting/README.md) — oss_type 统一选择，11 种后端
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
> - **[CQ:wakeup,userid=...]** C2C 互动召回消息标记
> - **[CQ:file,file=...,file_name=...]** 文件上传
> - **[CQ:card,title=...,desc=...,pic=...,url=...]** 群聊图文卡片消息
> - **[CQ:input_notify,type=...,second=...]** 单聊输入状态通知
> - **[CQ:stream,type:start,qq:...]** 单聊流式消息（start→mid→finish）
> - **[CQ:group_info,field=...,group_id=...,fallback=...]** 群信息内容展开码（群名/群简报/成员数/三字段 JSON；需 `cq_parse_mode: new`，legacy/shadow 下该码不被识别会原文发出）

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
| [CQ:wakeup]    | [C2C 互动召回消息标记] |
| [CQ:tts]       | [文本转语音]                      |
| [CQ:file]      | [文件上传]                        |
| [CQ:card]      | [群聊图文卡片消息]                  |
| [CQ:input_notify] | [单聊输入状态通知]               |
| [CQ:stream]      | [单聊流式消息]                    |
| [CQ:set_group]  | [群管理动作统一码(禁言/全员禁言/入群审批/策略/踢人/黑名单, 出站动作)] |


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
> - **send_group_msg_raw**（发送原始消息）
> - **get_group_member_blacklist**（获取群黑名单列表）
> - **set_group_member_blacklist**（群黑名单增删，≤20）
> - **get_custom_menu**（获取 C2C 自定义菜单）
> - **set_custom_menu**（设置 C2C 自定义菜单）
> - **get_panel_list**（获取指令面板列表）
> - **create_panel**（创建指令面板）
> - **get_panel**（获取指令面板详情）
> - **set_panel**（更新指令面板）
> - **delete_panel**（删除指令面板）
> - **set_panel_target**（增删面板关联对象）


#### 符合 OneBot 标准的 API

| API                      | 功能                   |
| ------------------------ | ---------------------- |
| /send_private_msg | [发送私聊 (C2C) 消息] |
| /send_group_msg | [发送 q群 (Group Chat) 消息] |
| /send_msg | [发送消息] |
| /delete_msg              | [撤回信息]             |
| /delete_group_msg        | [撤回QQ群用户或Bot消息] |
| /set_group_kick          | [群批量踢人(≤20,真实API,可同步拉黑)] |
| /set_group_ban | [群单人禁言(禁言/解禁)] |
| /set_group_whole_ban | [群全员禁言(开/关)] |
| /set_group_admin         | [群设置管理员] |
| /set_group_card          | [设置群名片] |
| /set_group_name          | [设置群名称] |
| /set_group_leave         | [退出群] |
| /set_group_special_title | [设置群专属头衔] |
| /set_friend_add_request  | [处理加好友请求]       |
| /set_group_add_request   | [处理加群请求(真实审批)] |
| /get_login_info | [获取登录号信息] |
| /get_stranger_info | [获取陌生人信息] |
| /get_friend_list | [获取好友列表] |
| /get_group_info | [获取群聊信息(真实)] |
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
| /get_group_join_request_list | [获取入群申请列表] |
| /get_group_bot_state | [获取机器人群内状态] |
| /join_approval_strategy_create | [创建入群自动审批策略] |
| /join_approval_strategy_list | [查询入群自动审批策略列表] |
| /join_approval_strategy_update | [修改入群自动审批策略] |
| /join_approval_strategy_execute | [执行入群自动审批策略] |
| /join_approval_strategy_whitelist | [修改策略白名单号码] |
| /join_approval_strategy_delete | [删除入群自动审批策略] |
| /get_group_member_blacklist | [获取群黑名单列表] |
| /set_group_member_blacklist | [群黑名单增删(≤20)] |
| /get_custom_menu | [获取 C2C 自定义菜单] |
| /set_custom_menu | [设置 C2C 自定义菜单] |
| /get_panel_list | [获取指令面板列表] |
| /create_panel | [创建指令面板] |
| /get_panel | [获取指令面板详情] |
| /set_panel | [更新指令面板] |
| /delete_panel | [删除指令面板] |
| /set_panel_target | [增删面板关联对象] |


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
> - **GroupJoinRequestEventHandler**（入群申请）

| 事件名称                   | 代表含义                         |
| --------------------------- | ------------------------------- |
| GroupATMessageEventHandler | [群聊@ 消息]                       |
| GroupMessageEventHandler   | [群聊普通消息]                     |
| C2CMessageEventHandler     | [私聊 (C2C)]                       |
| FriendAddEventHandler      | [被添加好友]                       |
| FriendDelEventHandler      | [被删除好友]                       |
| GroupAddRobotEventHandler  | [群聊机器人新增]                   |
| GroupDelRobotEventHandler  | [群聊机器人删除]                   |
| GroupMsgRejectHandler      | [群聊请求关闭推送]                 |
| GroupMsgReceiveHandler     | [群聊请求开启推送]                 |
| GroupMemberAddEventHandler | [群聊成员新增]                     |
| GroupMemberRemoveEventHandler | [群聊成员移除]                  |
| GroupJoinRequestEventHandler | [入群申请]                       |
| C2CMsgRejectHandler        | [用户拒绝私聊 (C2C) 消息推送]      |
| C2CMsgReceiveHandler       | [用户同意私聊 (C2C) 消息推送]      |


</details>

## 完整配置示例

> 全部配置项以首次运行自动生成的 `config.yml` 为准，以下为常见用法的完整示例：

```yaml
version: 1
settings:
  #── 基础设置 ────────────────────────────────────────
  app_id: 123456789                    # QQ 开放平台应用 ID
  token: "your_app_token"              # 应用令牌
  client_secret: "your_client_secret"  # 客户端密钥
  uin: 0                               # 机器人 QQ 号（用于 use_uin）
  use_uin: false                       # 使用 QQ 号作为 bot ID

  #── 连接方式（至少启用一种）──────────────────────────
  ws_address: ["ws://127.0.0.1:8080/onebot/v11/ws"]  # 反向 WS
  enable_ws_server: true                               # 正向 WS
  ws_server_token: "12345"                             # 正向 WS token
  http_address: "0.0.0.0:5700"                         # HTTP API
  http_access_token: ""                                # HTTP token
  post_url: [""]                                       # 反向 HTTP POST

  #── 事件订阅 ────────────────────────────────────────
    # - "ReadyHandler"                               # 连接成功
    # - "ErrorNotifyHandler"                         # 连接关闭
    # - "InteractionHandler"                         # 按钮回调事件
    - "GroupATMessageEventHandler"                   # 群 @ 消息
    - "GroupMessageEventHandler"                     # 群普通消息（需开放平台申请）
    - "C2CMessageEventHandler"                       # 私聊（需开放平台申请）
    # - "FriendAddEventHandler"                      # 用户添加机器人
    # - "FriendDelEventHandler"                      # 用户删除机器人
    # - "C2CMsgRejectHandler"                        # 用户拒绝 C2C 推送
    # - "C2CMsgReceiveHandler"                       # 用户开启 C2C 推送
    # - "GroupAddRobotEventHandler"                  # 群聊机器人新增
    # - "GroupDelRobotEventHandler"                  # 群聊机器人删除
    # - "GroupMemberAddEventHandler"                 # 群成员新增
    # - "GroupMemberRemoveEventHandler"              # 群成员移除
    # - "GroupJoinRequestEventHandler"               # 入群申请事件（需群管理员身份）

  #── 消息转换 ────────────────────────────────────────
  hash_id: true                       # 使用 hash 生成虚拟 ID
  op_userid_type: "vuin"             # user_id 来源
  array: false                        # segment 数组格式上报
  cq_parse_mode: legacy               # CQ 码解析器：legacy(默认，旧正则管道)/shadow(新旧并行解析，差异仅日志上报，行为仍走 legacy)/new(统一解析器 cqparse，修复 C1 贪婪 JSON/C3 批量 user_ids/私聊动作码拦截等，支持 [CQ:group_info]；建议先 shadow 观察日志无 diff 再切 new，可随时改回 legacy 回滚)

  #── Gensokyo 互联 ──────────────────────────────────
  server_dir: "your_server_ip"        # 图床/互联地址
  port: "15630"                       # HTTP 服务端口
  lotus: false                        # Lotus 互联模式

  #── WebUI ──────────────────────────────────────────
  disable_webui: false
  server_user_name: "admin"
  server_user_password: "admin"

  #── Markdown 消息 ──────────────────────────────────
  twoway_echo: false                  # 双向 echo
  custom_template_id: ""              # 图文转 MD 模板 ID
  keyboard_id: ""                     # 图文转 MD 按钮 ID
  force_verify_image_resource: false  # Markdown 图片转存结果校验（转存失败返回 40034004 且不发送；仅需确保图片必然可显示时开启）

  #── 消息发送 ──────────────────────────────────────
  lazy_message_id: false              # 惰性 message_id（主动推送用）
  send_delay: 300                     # 发送间隔（毫秒）
  no_ret_msg: false                   # 禁用回执（提升性能）

  #── 指令控制 ──────────────────────────────────────
  remove_prefix: false                # 忽略指令前 /
  remove_at: false                    # 忽略指令前 @
  bind_prefix: "/bind"
  status_prefix: "/gskstatus"

  #── 云存储 / 图床 ──────────────────────────────────
  # oss_type 仅控制图片上传路径；语音上传不受此选项影响（仍走本机或 1~3 云OSS）
  # 0=本机上传 1=腾讯云COS(旧t_COS_*) 2=百度云BOS 3=阿里云OSS 4=腾讯云COS自签(cos.*)
  # 5=Bilibili 6=QQ频道 7=ChatGLM 8=Ukaka 9=星野 10=Nature
  oss_type: 0                         # 请根据需求选择一个，同时只能启用一个

  #── 腾讯云配置 ──────────────────────────────────────
  t_COS_BUCKETNAME : ""               # 存储桶名称
  t_COS_SECRETID : ""                 # 用户的 SecretId
  t_COS_SECRETKEY : ""                # 用户的 SecretKey
  t_audit : false                     # 是否审核内容
  cos:                                # 腾讯云COS自签（oss_type=4）
    secret_id: ""                   # 腾讯云 API SecretId
    secret_key: ""                  # 腾讯云 API SecretKey
    region: "ap-guangzhou"          # 存储桶所在地域
    bucket: ""                      # 存储桶名称
    domain: ""                      # 自定义域名（留空使用COS默认域名）

  #── 百度云配置 ──────────────────────────────────────
  b_BOS_BUCKETNAME : ""               # 存储桶域名
  b_BCE_AK : ""                       # BCE AK
  b_BCE_SK : ""                       # BCE SK

  #── 阿里云配置 ──────────────────────────────────────
  a_OSS_EndPoint : ""                 # EndPoint
  a_OSS_BucketName : ""               # 存储桶名称
  a_OSS_AccessKeyId : ""              # AccessKeyId
  a_OSS_AccessKeySecret : ""          # AccessKeySecret
  a_audit : false                     # 是否审核图片
  # 图床凭证（oss_type=5~10）
  bilibili:                           # B站图床（oss_type=5）
    csrf_token: ""                  # B站 bili_jct
    sessdata: ""                    # B站 SESSDATA
    bucket: "openplatform"
  qq_channel:                         # QQ频道图床（oss_type=6）
    channel_id: ""                  # 上传图片的子频道 ID
    token: ""                       # Authorization, 如 "QQBot xxx.yyy"
  chatglm: {}                         # 智谱免费图床（oss_type=7）
  ukaka: {}                           # Ukaka 免费图床（oss_type=8）
  xingye: {}                          # 星野免费图床（oss_type=9）
  nature: {}                          # Nature 免费图床（oss_type=10）
  ```

> 详细配置指南请参阅 [docs/开始使用.md](./docs/开始使用.md) 和 [docs/idmap.md](./docs/idmap.md)

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
当'cq_parse_mode: new'时，性能相比原先提升7~20倍

## 特别鸣谢

- 感谢[`mnixry/nonebot-plugin-gocqhttp`](https://github.com/mnixry/nonebot-plugin-gocqhttp/): 本项目采用了mnixry编写的前端并实现了与它对应的基于qq官方api的后端api.
- 感谢[`ElainaCore/ElainaBot_v2`](https://github.com/ElainaCore/ElainaBot_v2)为本项目的图床方面提供了相关思路
- 感谢[`HX-Wrdzgzs/GensokyoNewQQWeb`](https://github.com/HX-Wrdzgzs/GensokyoNewQQWeb)为本项目搭建了 WebUI ~~(虽然没什么人看就是了)~~
- 感谢[`Hoshinonyaruko/Gensokyo`](https://github.com/Hoshinonyaruko/Gensokyo/)

## 引用

- [`QQ 机器人官方文档`](https://bot.q.qq.com/wiki/)：本项目许多处理均参考该文档
- [`tencent-connect/botgo`](https://github.com/tencent-connect/botgo): 本项目引用了此项目并做了一些改动
- [`ElainaCore/ElainaBot_v2`](https://github.com/ElainaCore/ElainaBot_v2)：本项目的图床服务部分基于其相关源代码修改
- [`Te-River/Gensokyo-NewQQ-Test-plugins`](https://github.com/Te-River/Gensokyo-NewQQ-Test-plugins): 基于Nonebot2的测试插件

## ⭐ Star History

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=Te-River/Gensokyo-NewQQ&type=date&theme=dark&legend=top-left" />
  <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=Te-River/Gensokyo-NewQQ&type=date&legend=top-left" />
  <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=Te-River/Gensokyo-NewQQ&type=date&legend=top-left" />
</picture>

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

交流群：**196173384**

## 快速开始

1. 前往 [Releases](https://github.com/hoshinonyaruko/gensokyo/releases) 下载对应系统的二进制文件
2. 参考[开始使用](/docs/开始使用.md) 创建机器人并配置
3. 启动 gensokyo，连接你的 OneBot V11 客户端

## 功能亮点

-  兼容 OneBot V11 的 HTTP API、反向 HTTP POST、正向 WebSocket、反向 WebSocket
-  群聊非@消息支持（GroupMessageEventHandler）
-  群聊消息自动剔除 @机器人字符
-  非自身 @ 自动转为 `[CQ:at,qq=虚拟ID]` 格式
-  按钮权限中虚拟数字 ID 自动转化为 QQ 官方 OpenID
-  扩展 API：`get_avatar`（获取头像直链）
-  全场景 event_id 存储，支持被动消息
-  消息事件新增 `to_me` 字段，标识是否 @ 了机器人
-  多 WS 地址连接
-  频道虚拟成群事件、私信虚拟成频道事件
-  WebUI 管理界面
-  指令黑白名单、URL 自动转换
-  可自定义图片压缩/图床服务
-  支持文字、图片、语音、视频、Markdown 等多种消息类型
-  主动信息失败自动转被动
-  完善的重连机制

## 文档

- [开始使用](/docs/开始使用.md) — 注册机器人、配置、启动
- [API 介绍](/docs/api/api介绍.md) — 支持的 API 列表与扩展
- [额外 API - get_avatar](/docs/扩展api/扩展api-get_avatar.md) — 获取用户头像直链
- [Markdown 消息](/docs/文档-markdown消息.md) — Markdown 卡片消息说明
- [更多文档](/docs/更多文档.md) — 完整文档索引

## CQ 码与 API 支持

### 已实现 CQ 码

<details>
<summary>展开查看</summary>

> 新增了如下 **CQ码**</br>
> -------
> - **[CQ:avatar]** 头像获取

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
| [CQ:tts]       | [文本转语音]                      |


</details>

<details>
<summary>已实现 API</summary>

> 新增了如下 **API**</br>
> -------
> - **get_avatar**（头像直链）
> - **get_robot_share_link**（分享链接）
> - **send_private_msg_wakeup**（被动唤醒）
> - **put_interaction**（按钮回调）


#### 符合 OneBot 标准的 API

| API                      | 功能                   |
| ------------------------ | ---------------------- |
| /send_private_msg | [发送私聊消息] |
| /send_group_msg | [发送群消息] |
| /send_guild_channel_msg | [发送频道消息] |
| /send_msg | [发送消息] |
| /delete_msg              | [撤回信息]             |
| /set_group_kick          | [群组踢人]             |
| /set_group_ban | [群组单人禁言] |
| /set_group_whole_ban | [群组全员禁言] |
| /set_group_admin         | [群组设置管理员]       |
| /set_group_card          | [设置群名片（群备注）] |
| /set_group_name          | [设置群名]             |
| /set_group_leave         | [退出群组]             |
| /set_group_special_title | [设置群组专属头衔]     |
| /set_friend_add_request  | [处理加好友请求]       |
| /set_group_add_request   | [处理加群请求/邀请]    |
| /get_login_info | [获取登录号信息] |
| /get_stranger_info | [获取陌生人信息] |
| /get_friend_list | [获取好友列表] |
| /get_group_info | [获取群/频道信息] |
| /get_group_list | [获取群列表] |
| /get_group_member_info | [获取群成员信息] |
| /get_group_member_list | [获取群成员列表] |
| /get_group_honor_info    | [获取群荣誉信息]       |
| /can_send_image | [检查是否可以发送图片] |
| /can_send_record | [检查是否可以发送语音] |
| /get_version_info | [获取版本信息] |
| /set_restart | [重启 gensokyo] |
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
> - **group_member_add**（群成员新增）
> - **group_member_remove**（群成员移除）

#### 符合 OneBot 标准的 Event（部分 Event 比 OneBot 标准多上报几个字段，不影响使用）

| 事件类型 | Event            |
| -------- | ---------------- |
| 消息事件 | [私聊信息] |
| 消息事件 | [群消息] |
| 通知事件 | [群文件上传]     |
| 通知事件 | [群管理员变动]   |
| 通知事件 | [群成员减少]     |
| 通知事件 | [群成员增加]     |
| 通知事件 | [群禁言]         |
| 通知事件 | [好友添加]       |
| 通知事件 | [好友删除]       |
| 通知事件 | [群消息撤回]     |
| 通知事件 | [好友消息撤回]   |
| 通知事件 | [群内戳一戳]     |
| 通知事件 | [群红包运气王]   |
| 通知事件 | [群成员荣誉变更] |
| 通知事件 | [群消息推送关闭] |
| 通知事件 | [群消息推送开启] |
| 请求事件 | [加好友请求]     |
| 请求事件 | [加群请求/邀请]  |


#### 拓展 Event

| 事件类型 | 拓展 Event       |
| -------- | ---------------- |
| 通知事件 | [好友戳一戳]     |
| 通知事件 | [群内戳一戳]     |
| 通知事件 | [群成员名片更新] |
| 通知事件 | [接收到离线文件] |
| 通知事件 | [按钮交互回调] |
| 通知事件 | [C2C 消息推送关闭] |
| 通知事件 | [C2C 消息推送开启] |

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
| ATmessageEventHandler      | [频道at消息]                       |
| DirectMessageHandler       | [私域频道私信(dms)]                |
| ReadyHandler               | [连接成功]                         |
| ErrorNotifyHandler         | [连接关闭]                         |
| GuildEventHandler          | [频道事件]                         |
| MemberEventHandler         | [频道成员新增]                     |
| ChannelEventHandler        | [频道事件]                         |
| CreateMessageHandler       | [频道不at消息]                     |
| InteractionHandler         | [频道卡片按钮data回调事件] |
| GroupATMessageEventHandler | [群at消息]                         |
| GroupMessageEventHandler   | [群普通消息]                       |
| C2CMessageEventHandler     | [群私聊]                           |
| ThreadEventHandler         | [频道发帖事件]                     |
| FriendAddEventHandler      | [被添加好友]                       |
| FriendDelEventHandler      | [被删除好友]                       |
| GroupAddRobotEventHandler  | [群机器人新增]                     |
| GroupDelRobotEventHandler  | [群机器人删除]                     |
| GroupMsgRejectHandler      | [群请求关闭推送]                   |
| GroupMsgReceiveHandler     | [群请求开启推送]                   |
| GroupMemberAddEventHandler | [群成员新增]                       |
| GroupMemberRemoveEventHandler | [群成员移除]                    |
| C2CMsgRejectHandler        | [用户拒绝(C2C)消息推送]             |
| C2CMsgReceiveHandler       | [用户同意(C2C)消息推送]             |


</details>

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

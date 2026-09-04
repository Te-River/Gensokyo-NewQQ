# Changelog — Release015

> 自 Release014 以来的所有变更。

---

## Added

- 为 QQ CDN 富媒体上传与统一图床上传添加重试机制：上传失败后最多重试 2 次（线性退避 1s/2s）；CDN 仅对超时类错误重试，图床对所有错误重试。
- 群成员管理（QQ 官方 v2 接口）：
  - `get_group_member_list` 升级为真实成员列表（游标分页全量拉取，单页≤30、安全上限 100 页；`join_time`/`role`/`nickname` 为真实数据）；官方接口不可用时自动回退本地 idmap-pro 缓存路径，中途页失败返回已收集数据。
  - `get_group_member_info` 升级为真实单成员信息；接口失败时回退中性值兜底。
  - 新增 `set_group_kick`：批量移出群成员（单批≤20，`user_id`/`user_ids` 二选一可合并去重，`add_blacklist` 移出同时拉黑）。
  - 新增 `get_group_member_blacklist` / `set_group_member_blacklist`：群黑名单列表查询与增删。
- C2C 自定义菜单与指令面板：新增 `get_custom_menu` / `set_custom_menu` / `get_panel_list` / `create_panel` / `get_panel` / `set_panel` / `delete_panel` / `set_panel_target` 共 8 个扩展 action（菜单仅 C2C 全局生效；面板支持 c2c/group/channel/dm；`create_panel`/`set_panel_target` 关联对象列表任一反查失败整体报错）。
- `[CQ:set_group]` 新增 3 个子动作：`action=kick`（批量踢人，`user_ids` 逗号分隔、≤20 截断警告、`add_blacklist` 同步拉黑）、`action=blacklist_add` / `action=blacklist_del`（黑名单增删）；详见 [扩展 CQ:set_group](../docs/cq码/扩展CQ码/扩展cq码-cq-set_group.md)。
- 全局域名统一：`api.sgroup.qq.com` → `api.bot.qq.com`（botgo v1/v2 与图床直连地址）。官方"统一请求地址"仅记载 `api.bot.qq.com`，sandbox 域名官方文档已不再单列；`sandbox_mode` 配置项保留，行为收敛到同一域名。
- Markdown 消息新增配置项 `force_verify_image_resource`（默认 false）：开启后官方校验图片转存结果，转存失败返回错误（40034004）且消息不发送；注入点收敛在 `parseMDData()`，群聊与单聊 Markdown 共用。
- **行为变化（假数据诚实化）**：
  - 发送失败/官方审核中场景的回执 `message_id` 由固定假值 `123` 改为 `0`（无真实 message_id 不再伪造；`retcode 0` 语义保持不变）。
  - `set_group_card` 由假成功改为明确失败回执（`retcode 100`，QQ 官方 API 未提供设置群名片接口）。
  - `get_group_list` 的 `group_create_time` 由当前时间戳改为 `0`（官方无群创建时间字段），并逐群拉取真实群名/群简介/成员数（受 `get_g_list_delay` 毫秒节流防超 QPM，群量大时响应变慢属预期）。
  - `get_status` 的包收发/断线/丢包统计由假数据改为 `0`（官方无统计接口且 wsclient 无导出连接计数），`online`/`good` 保持 true（进程存活即在线）；消息收发统计仍为 botstats 真实数据。
  - `mark_msg_as_read` 清除假字面量，保持空实现回执（QQ 官方无已读上报接口）。
  - `get_group_member_list` 弱字段（`level`/`area`/`title`）由固定 `"0"` 改为空字符串（真实接口无对应字段，不再伪造；下游按数字解析 `level` 需注意，本地缓存回退路径仍为 `"0"`）。


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
- 登录排障提示（UX）：登录失败时按官方错误码输出针对性指引（`11245`=固定 Token 已禁用，提示使用 appid+client_secret 动态鉴权；`11243`=Token 错误，提示核对 appid/client_secret）；启动时输出当前 API 域名日志，便于确认新旧构建。
- Markdown 消息新增配置项 `force_verify_image_resource`（默认 false）：开启后官方校验图片转存结果，转存失败返回错误（40034004）且消息不发送；注入点收敛在 `parseMDData()`，群聊与单聊 Markdown 共用。
- **行为变化（假数据诚实化）**：
  - 发送失败/官方审核中场景的回执 `message_id` 由固定假值 `123` 改为 `0`（无真实 message_id 不再伪造；`retcode 0` 语义保持不变）。
  - `set_group_card` 由假成功改为明确失败回执（`retcode 100`，QQ 官方 API 未提供设置群名片接口）。
  - `get_group_list` 的 `group_create_time` 由当前时间戳改为 `0`（官方无群创建时间字段），并逐群拉取真实群名/群简介/成员数（受 `get_g_list_delay` 毫秒节流防超 QPM，群量大时响应变慢属预期）。
  - `get_status` 的包收发/断线/丢包统计由假数据改为 `0`（官方无统计接口且 wsclient 无导出连接计数），`online`/`good` 保持 true（进程存活即在线）；消息收发统计仍为 botstats 真实数据。
  - `mark_msg_as_read` 清除假字面量，保持空实现回执（QQ 官方无已读上报接口）。
  - `get_group_member_list` 弱字段（`level`/`area`/`title`）由固定 `"0"` 改为空字符串（真实接口无对应字段，不再伪造；下游按数字解析 `level` 需注意，本地缓存回退路径仍为 `"0"`）。
- **CQ 码统一解析架构（cqparse）与新扩展码**：
  - 新增 `handlers/cqparse` 统一解析包：字符串 CQ 码 / 消息段数组 / TRSS map 三输入归一为 Token 流，转义编解码唯一实现，替换值单遍拼接永不回炉重扫；22 个既有码全部迁入，新增码只需注册一次（根治"修一个漏一片"）。
  - 新增扩展码 `[CQ:group_info,field=name/memo/member_count/all,group_id=...,fallback=...]`：正文展开群名/群简报/成员数或三字段 JSON；同消息同群多码合并一次取数（30 QPM 保护）；失败分级（参数错保留原文，取数失败替换 fallback 默认空串）；详见 [扩展 CQ:group_info](../docs/cq码/扩展CQ码/扩展cq码-cq-group_info.md)。
  - 新增配置项 `cq_parse_mode: legacy|shadow|new`（默认 `legacy`，合并后零行为变化）：`shadow` 并行跑新旧解析、差异仅日志上报（`[cqparse-shadow]`）行为仍走 legacy；`new` 全走统一解析器；可随时改回 `legacy` 回滚。**建议先 shadow 观察一版无意外 diff 再切 new**。
  - **`cq_parse_mode: new` 下的行为修复**（legacy 模式维持旧行为）：
    - 修复贪婪 JSON 正则跨码吞噬：`[CQ:markdown,...]普通文本[CQ:keyboard,...]` 相邻时正文与 keyboard 不再丢失。
    - 修复字符串路径批量 `user_ids=1,2,3` 只取第一个的问题，与消息段/TRSS 路径等价。
    - 修复 `[CQ:remove]`/`[CQ:set_group]` 失败路径把 CQ 码原文留在正文发出的问题（统一"无论成败移除+日志"）。
    - 修复动作码在私聊/转发路径原样泄漏为聊天文本的问题（现拦截：移除+日志，不执行不发送）。
    - 修复媒体码附加参数污染 URL（`file=xx,subType=0,url=...`）、`xxx.image` 文件名整码泄漏、`file` 值含逗号被截断、未知前缀泄漏等问题（统一入 `unknown_*` 或按参数正确解析）。
    - 修复 `[CQ:cardboard]` 被 `[CQ:card]` 前缀误吞、`[CQ:stream,type=start,qq=1]` 等号语法不识别、`[CQ:reply,id=字母数字]` 不解析、消息段数字 qq/group_id 产 `[CQ:at,qq=]` 空参等问题。
    - `[CQ:avatar]` 多码不再共用第一个 URL；产物直接入 `url_images`，不再以 `[CQ:image,...]` 形式回写正文；反查失败时丢弃该码并记录日志（不再产出破损 URL）。
  - **入站修复（独立于解析模式，立即生效）**：
    - `array=true` 全量群消息（GROUP_MESSAGE_CREATE）现在与字符串路径一致剥离 @bot（此前 array 模式会向下游上报"有人@了机器人"，可能触发自我应答）。
    - `string_ob11=true` 时 C2C 私聊消息与群消息对齐：`message_id`/`user_id`/`sender.user_id` 上报真实 string（此前私聊无 StringOb11 分支，ID 体系割裂）。
    - `array=true` 时附件按 ContentType 过滤，视频/语音附件不再误标为 image 段。
    - 平台事件伪装消息（c2c_switch / 群消息拒收）的 `message_id` 由固定假值 `123` 改为保留值 `-1`（杜绝与真实虚拟 msg_id 撞号导致 reply/撤回串扰；该消息不可 reply）。
    - 修复 user_id 含正则元字符时出站 @ 处理触发 `regexp.MustCompile` panic 可致进程崩溃的问题（改为精确字符串匹配）。
    - 入站热路径正则改为包级预编译（每条群消息节省 ≥2 次正则编译）。


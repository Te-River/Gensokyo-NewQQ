# 扩展 API 文档

| API | 说明 | 详细文档 |
|-----|------|:--------:|
| `get_avatar` | 获取用户头像直链 | [查看](./扩展api/扩展api-get_avatar.md) |
| `get_robot_share_link` | 获取机器人分享链接 | [查看](./扩展api/扩展api-get_robot_share_link.md) |
| `send_private_msg_wakeup` | 发送 C2C 召回消息 | [查看](./扩展api/扩展api-send_private_msg_wakeup.md) |
| `put_interaction` | 处理按钮交互回调 | [查看](./扩展api/扩展api-put_interaction.md) |
| `delete_group_msg` | 撤回 QQ 群用户或 Bot 自身消息 | [查看](./扩展api/扩展api-delete_group_msg.md) |
| `get_group_join_request_list` | 获取入群申请列表 | [查看](./扩展api/扩展api-get_group_join_request_list.md) |
| `get_group_bot_state` | 获取机器人群内状态 | [查看](./扩展api/扩展api-get_group_bot_state.md) |
| `join_approval_strategy_create` / `_list` / `_update` / `_execute` / `_whitelist` / `_delete` | 入群自动审批策略管理 | [查看](./扩展api/扩展api-join_approval_strategy.md) |
| `set_group_kick` | 批量移出群成员（≤20，可同步拉黑） | — |
| `get_group_member_blacklist` / `set_group_member_blacklist` | 群黑名单列表查询 / 增删（≤20） | — |
| `get_custom_menu` / `set_custom_menu` | C2C 自定义菜单读取 / 设置（覆盖式） | — |
| `get_panel_list` / `create_panel` / `get_panel` / `set_panel` / `delete_panel` / `set_panel_target` | 指令面板管理：列表 / 创建 / 详情 / 更新元素与备注 / 删除 / 增删关联对象 | — |

> 上述群成员管理 / 菜单 / 面板 action 基于 QQ 官方 v2 接口（机器人需群管理员等相应权限）：
> - ID 均为虚拟数字 ID（32 位原生 OpenID 亦可直接传入）；`get_panel` 返回的 `user_openids`/`group_openids` 与 `get_group_member_blacklist` 返回的 `user_id` 均已转换为虚拟 ID，可直接回传操作。
> - 失败时统一返回 `retcode 100` + 错误消息（权限不足 / 接口未开放 / 参数无效等），不做假成功。
> - `[CQ:set_group,action=kick/blacklist_add/blacklist_del]` 为对应的 CQ 码形式，行为一致（详见 [CQ:set_group](../cq码/扩展CQ码/扩展cq码-cq-set_group.md)）。

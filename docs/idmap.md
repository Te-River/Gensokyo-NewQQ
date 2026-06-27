# idmap 数据库

## 目标

新的 idmap 设计把「外部身份」和「下游看到的 ID」分开：

- QQ 平台的 OpenID、可反查到的 rUIN，以及其他外部平台 ID 都是外部身份。
- Gensokyo 内部维护稳定的 vUIN，默认下游 OneBot 字段仍使用 vUIN。
- 多个 OpenID 可以绑定到同一个 vUIN，多个 rUIN 也可以绑定到同一个 vUIN。
- 反查不再返回某个 vUIN 的全部身份，只按当前消息上下文或最近一次上下文选择本次要使用的身份。

这解决了旧版 `idmap.db` 的核心缺陷：旧库是 `OpenID -> 数字 ID` 与 `row-数字 ID -> OpenID` 的单值双向表，无法表达「同一个真实用户在多个 AppID / 多个平台下的多个身份」。

## 数据库文件

| 文件 | 桶 | 用途 |
|------|-----|------|
| `openid-map.db` | `meta` | schema version、迁移标记、vUIN 计数器 |
| `openid-map.db` | `identity_to_vuin` | 外部身份 -> vUIN |
| `openid-map.db` | `vuin_to_identity` | vUIN -> 外部身份集合 |
| `openid-map.db` | `last_seen` | vUIN 在不同场景下最近一次使用的外部身份 |
| `openid-map.db` | `config` | 运行时配置 |
| `openid-map.db` | `UserInfo` | 用户信息缓存 |
| `msgid-map.db` | `msg_to_virtual` | 真实 message_id -> 虚拟 message_id |
| `msgid-map.db` | `virtual_to_msg` | 虚拟 message_id -> 真实 message_id |
| `msgid-map.db` | `expires_at` | message_id 过期时间 |
| `idmap.db`（旧） | `ids`/`config`/`UserInfo`/`cache` | 旧版单库，只作为一次性迁移源 |

> `idmap_pro` 在新结构中不再有意义：MultiMap 身份图已经原生支持「多个真实身份绑定到同一个 vUIN」。配置项会被移除，旧的 Pro API 仅作为兼容 wrapper 保留。

## 身份键格式

所有写入数据库的身份都必须带类型，不能只靠长度猜测：

| 类型 | 规范化键 |
|------|----------|
| QQ OpenID | `openid:QQ:<appid>:<openid>` |
| QQ rUIN | `ruin:QQ:<index>:<qq>` |
| OAuth/GitHub rUIN | `ruin:OAuth_Github:<index>:<id>` |
| 旧版或未分类原始值 | `raw:<value>` |

QQ OpenID 的身份键只包含 `QQ`、`AppID` 和 OpenID 本体，不再按群聊、私聊或群成员事件拆分。QQ 平台下同一个 QQ 号在同一个 AppID 内应复用同一个 OpenID，因此群聊、C2C、好友事件、群成员事件都会落到同一个 vUIN。频道相关 ID 暂不纳入本轮重构，继续沿用现有逻辑。

对外传递 rUIN 时使用：

```text
rUIN-<PLATFORM_NAME>-<ID_CNT>-<ID>
```

- `PLATFORM_NAME` 示例：`QQ`、`OAuth_Github`
- `ID_CNT` 从 `0` 开始递增
- `ID` 对下游输出时使用 Base32；接收客户端输入时允许 raw 或 Base32
- `rUIN-QQ-0-2870338968` 与 `rUIN-QQ-0-GI4DOMBTGM4DSNRY` 等价

## 下游 ID 来源

默认配置继续向下游发送内部 vUIN：

```yaml
op_userid_type: vuin
```

可选值：

| 值 | 含义 |
|----|------|
| `vuin` | 默认值，`user_id`/`group_id` 使用内部 vUIN |
| `raw` | 发送当前消息的原始平台 ID |
| `ruin` | 优先发送当前上下文中的 rUIN；没有 rUIN 时回退 vUIN |

接收客户端操作时：

- vUIN 和 OpenID 都是默认行为，不需要额外前缀。
- rUIN 使用 `rUIN-<PLATFORM>-<index>-<id>`。
- 当客户端传入纯数字时，默认按 vUIN 处理；如需阻止推测，使用 `op_userid_type` 指定策略。

## 写入路径

收到腾讯侧事件时：

1. 将 OpenID 规范化为 `openid:QQ:<appid>:<openid>`。
2. 如果事件或历史信息能提供 rUIN，则规范化为 `ruin:QQ:<index>:<qq>`。
3. 查询 `identity_to_vuin`：
   - 任一身份已存在，复用对应 vUIN。
   - 都不存在，分配新的 vUIN。
4. 将本次消息实际出现的身份写入 `vuin_to_identity`。
5. 写入 `last_seen`，记录当前消息上下文用于后续反查。

绑定操作不再移动 row，也不覆盖旧反向键，而是把一个身份追加到目标 vUIN：

```text
BindIdentityToVuin(openid:QQ:<appid>:<openid>, 10001)
BindIdentityToVuin(ruin:QQ:0:2870338968, 10001)
```

## 反查路径

当下游用 vUIN 发起操作时：

1. 优先使用请求中携带的显式 OpenID 或 rUIN。
2. 其次读取当前消息上下文中的 `last_seen`。
3. 再从 `vuin_to_identity` 中选择一个可用于目标 API 的身份。

如果一个 vUIN 下有多个 OpenID/rUIN，反查只附加本次发送需要的身份，不全量附加所有身份。

## 旧库迁移

启动时如果检测到旧版单库 `idmap.db`，会阻塞执行一次性转换：

1. 打开 `openid-map.db` 与 `msgid-map.db`。
2. 读取旧 `ids` 桶，保留既有 `OpenID/raw -> vUIN` 关系。
3. 读取旧 `row-<vUIN>` 反向键，补齐缺失的身份关系。
4. 识别旧 `idmap_pro` 复合键，拆成普通身份映射保存。
5. 复制 `config` 与 `UserInfo`。
6. 不全量迁移旧 `cache` 桶；仅同步 message_id 计数器以避免虚拟 message_id 撞号。
7. 校验完成后在 `openid-map.db/meta` 写入迁移标记。

迁移完成后热路径只访问新库，不再惰性复制旧库内容。旧 `idmap.db` 不会被自动删除，确认稳定后可手动备份或删除。

## msgid-map

`msgid-map.db` 专门保存 message_id 映射，默认 TTL 为 1 小时：

```yaml
msgid_ttl_seconds: 3600
```

每次写入：

- `msg_to_virtual[real_message_id] = virtual_message_id`
- `virtual_to_msg[virtual_message_id] = real_message_id`
- `expires_at[virtual_message_id] = now + ttl`

后台清理每分钟扫描 `expires_at`，删除过期的三条记录。旧库 `cache` 不再全量迁移，避免把大量历史临时数据搬进新库。

## 运行日志

新 idmap 热路径默认输出状态日志，便于把平台原始事件和内部映射结果对齐：

- `[idmap] identity create/hit`：OpenID/rUIN/raw 身份新建或命中，并显示对应 vUIN。
- `[idmap] bind moved/skipped/failed`：手动或自动绑定时移动身份集合的结果。
- `[idmap] msg_id create/hit/lookup`：真实 message_id 与虚拟 message_id 的写入、刷新 TTL、反查结果。
- `[message] group/c2c/friend/group_member ... mapped`：Processor 层事件最终映射出的 vUIN 与虚拟 message_id。

## 兼容性

保留旧公开函数名，内部路由到新结构：

| 旧函数 | 新行为 |
|--------|--------|
| `StoreIDv2(id)` | 规范化身份并返回 vUIN |
| `RetrieveRowByIDv2(vuin)` | 按上下文解析 vUIN 对应的当前可用外部身份 |
| `UpdateVirtualValuev2(old, new)` | 将 old 对应身份绑定到 new vUIN |
| `StoreCachev2(id)` | 写入 `msgid-map.db`，默认 1h TTL |
| `RetrieveRowByCachev2(id)` | 从 `msgid-map.db` 读取真实 message_id |
| `StoreIDv2Pro` / `RetrieveRowByIDv2Pro` | 兼容 wrapper，不再依赖 `idmap_pro` 配置 |

## 故障恢复

| 故障 | 影响 | 恢复 |
|------|------|------|
| `msgid-map.db` 损坏 | message_id 回执可能丢失 | 停止程序，删除 `msgid-map.db` 后重启 |
| `openid-map.db` 损坏且旧 `idmap.db` 仍在 | 身份映射暂不可用 | 停止程序，删除 `openid-map.db` 后重启重新迁移 |
| `openid-map.db` 损坏且旧库已删除 | 身份映射丢失 | 从备份恢复 |
| 迁移校验失败 | 新库不写迁移完成标记 | 保留旧库，修复后重启重新迁移 |

建议定期备份 `openid-map.db`。`msgid-map.db` 是临时缓存，不需要长期备份。

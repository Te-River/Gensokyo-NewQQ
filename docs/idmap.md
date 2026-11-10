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

> **实际运行**：当前使用的数据库文件为 `idmap-identity.db`（桶: `ids`/`config`/`UserInfo`）和 `idmap-msg.db`（桶: `cache`）。上表描述的 `openid-map.db` / `msgid-map.db` 多桶架构是 v2 重构方案，可通过 `go build -tags map_idmap` 启用。两者的对外行为完全一致。

## 身份键格式

所有写入数据库的身份都必须带类型，不能只靠长度猜测：

| 类型 | 规范化键 |
|------|----------|
| QQ OpenID | `openid:QQ:<appid>:<openid>` |
| QQ rUIN | `ruin:QQ:<index>:<qq>` |
| OAuth/GitHub rUIN | `ruin:OAuth_Github:<index>:<id>` |
| 旧版或未分类原始值 | `raw:<value>` |

QQ OpenID 的身份键只包含 `QQ`、`AppID` 和 OpenID 本体，不再按群聊、私聊或群成员事件拆分。QQ 平台下同一个 QQ 号在同一个 AppID 内应复用同一个 OpenID，因此群聊、C2C、好友事件、群成员事件都会落到同一个 vUIN。

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

> **2026-08-01 修复：** `StartMigration` 已改为**阻塞式迁移**（此前为 `go backgroundMigration()` 非阻塞）。迁移完成才返回，确保 `main.go` 调用点之后才连 WS / 启动 HTTP，避免迁移期间 `storeIdentity` 双写与 `backgroundMigration` 迁入并发产生 2 row → 1 OpenID 重复映射。同时 `writeBatchToNewDB` 追加按 value 委重（逆向条目 `uin:row-*` 迁入前查正向 `uin:<OpenID>` 是否已存在，若已有则跳过）。

## 强制解绑工具

迁移异常或历史数据残留导致 2 个虚拟 ID 指向同一 OpenID 时，可用强制解绑工具按 OpenID 直接清理全部映射：

| 工具 | 调用方式 | 行为 |
|------|---------|------|
| `ForceUnbindID(id)` | Go 内部 | 入参支持**OpenID 字符串**或**虚拟 ID**两种形式。OpenID 时直接删正向 `uin:<OpenID>` + 扫删所有指向同一 OpenID 的逆向条目 `uin:row-*`（新库+旧库）；虚拟 ID 时先通过逆向条目 `uin:row-<N>` 反查 OpenID（新库+旧库回退），再按 OpenID 清理。返回清理条数 |
| `getid type=18` | HTTP API | `GET /getid?type=18&id=<OpenID或虚拟ID>`，返回 `{"status":"success","unbound_count":N}` |
| `UpdateVirtualValue(old, 0)` | `getid type=5` | `newRowValue=0` 解绑时已同步彻底清理：删正向 + 扫删所有指向同一 OpenID 的重复逆向条目（2026-08-01 修复，此前只删单条逆向不删正向，导致解绑失效） |

解绑后该 OpenID 彻底无映射，下次 `storeIdentity` 会重新分配唯一虚拟 ID。

> **2026-08-01 修复：** `ForceUnbindID` 原仅接受 OpenID 入参，但用户实际常传虚拟 ID（row 值），导致查正向 `uin:<虚拟ID>` 找不到直接返回，`unboundCount=0`（表现为 `getid type=18` 返回 false）。现已支持双形式入参：纯数字视为虚拟 ID，先反查 OpenID 再清理；非纯数字视为 OpenID 直接清理。同时去掉“正向条目存在检查”的提前返回，即便正向已被其他路径删除，也要扫删残留逆向条目，确保彻底清理。

## 重绑定（UpdateVirtualValue）

把某个身份从旧虚拟 ID 改绑到新虚拟 ID：`getid type=5`（单次）、`getid type=12`（两次 Pro）、
gRPC `UpdateVirtualValueV2` / `UpdateVirtualValueV2Pro` 共用同一条语义。

| 步骤 | Go（`idmap/service.go:1405`） | Rust（`IdMapStore::update_virtual_value`） |
|------|------------------------------|------------------------------------------|
| 旧值 → 身份 | 旧库反表 `uin:row-<old>` → 新库反表 `uin:row-<old>` → 新库正向扫描（值等于 old 的 key）→ 全落空 `不存在:<old>` | 反表前缀 `"<old>:"` 扫描 → 正表值等于 old 的 key → 全落空 `NotFound`；反表缺行时用 `last_seen` 还原原始身份串（Rust 没有旧库，第一段不存在） |
| `new != 0` 前检查 | 目标反查 key 已存在 → `%v :已存在`（`new == 0` 跳过，否则解绑永远报已存在） | 同；错误是独立变体 `StorageError::AlreadyExists`，gRPC 映射 `AlreadyExists` |
| `new != 0` 写入 | 删旧反查 key，写正向 `uin:<id>`=BE(new) + 新反查 key | 把旧 vUIN 的**全部**反表行迁到 `"<new>:"`（每条的原始身份串原样保留，正表一并改指 new；反表缺行时补上兜底选中的身份） |
| `new == 0` | 删正向 `uin:<id>` + 扫删所有指向同一 OpenID 的反查条目 | 同（反表同时比对 key 后缀与 value，覆盖迁移遗留的重复行） |
| 隔离模式 | 正向桶写 alias `uinKey(new)` → BE(old) 后**直接返回**（不删旧、不写反查、不查 exists） | 正表写 `raw:<new>` → BE(old)，其余映射不动 |
| Pro 变体 | 顺序调两次 `UpdateVirtualValue`（**非原子**） | 同，非原子：第一次成功第二次失败会留下半完成状态；两次调用之间也可能被其它写入（别的重绑定/分配）穿插 |
| 拒绝理由 | 旧值查不到 / 目标已存在 → HTTP 500 | 同（`/getid` 该分支没有 404） |

- **缓存**：重绑定提交后先推进缓存代际号（`bump_epoch`）再精确失效身份 key 与 old/new 两个 vUIN，
  因此「重绑定后立即读」在两个方向上都拿不到旧值，「查不到」的负缓存同样失效。
- **Rust 与 Go 的差异**：Go 会同时写迁移期旧库（`row-<N>` / `idmap_legacy_compat` 的裸 `<new>` 键），
  Rust 只有一份身份库，没有旧库、也没有「迁移完成」状态，故不双写；Go 只删 `uinRowKey(old)` 这
  一条反查 key，Rust 把 `"<old>:"` 整个前缀家族删掉（redb 反表同一 vUIN 下可能有多条遗留 key）。
- **未移植**：Go 的 lotus 远程分支（`UpdateVirtualValuev2` 在 `lotus && lotus_grpc` 时转发给远端
  gRPC，或在 `lotus && !lotus_without_idmaps` 时打自己的 `/getid?type=5`）在 Rust 没有对应实现，
  见 `reports/refactor/17-go-rust-parity.md` 的 lotus 行。

## 批量导出接口

迁移调试或数据核查时，可一次性导出所有身份映射：

| 工具 | 调用方式 | 行为 |
|------|---------|------|
| `ListAllIdentities()` | Go 内部 | 返回 `*IdentitySnapshot`，包含所有 OpenID <-> 虚拟 ID 映射及用户名缓存 |
| `getid type=19` | HTTP API | `GET /getid?type=19`，返回所有身份映射的 JSON 快照 |

Rust 的 `list_identities` 与 Go 一样不把隔离重绑定的 alias 当身份列出。Go 的规则是"key 含 `:`
就跳过"，而 redb 的真实身份 key 一律带 scheme 前缀（`openid:` / `ruin:` / `raw:`），照抄会一条
不剩，因此只跳过 alias 形态（`raw:<十进制 vUIN>`）。

**调用示例：**

```bash
# 本地访问（无 lotus_password 时）
curl http://127.0.0.1:15630/getid?type=19

# 带认证（有 lotus_password 时，需计算 HMAC 签名）
curl "http://<server>:<port>/getid?type=19&timestamp=<ts>&token=<hmac>"
```

**响应示例：**

```json
{
  "mappings": [
    {"real_id": "ABC123DEF456...", "virtual_id": 10001, "username": "张三"},
    {"real_id": "789GHI012JKL...", "virtual_id": 10002, "username": "李四"},
    {"real_id": "MNO345PQR678...", "virtual_id": 10003, "username": ""}
  ],
  "count": 3,
  "timestamp": 1723190400
}
```

**字段说明：**
- `real_id`：QQ 官方 OpenID（已去除内部 UIN 前缀）
- `virtual_id`：Gensokyo 分配的虚拟数字 ID（下游 OneBot 使用）
- `username`：用户名缓存（入站时自动缓存，10 分钟 TTL，可能为空）
- `count`：映射总数
- `timestamp`：快照生成时间（Unix 时间戳，秒）

**快照机制：**
- 接口使用 bbolt MVCC 事务，在请求到达时生成一致性快照
- 即使遍历期间有其他消息触发新的映射写入，也不影响本次导出结果
- `timestamp` 字段可用于判断数据新鲜度

**注意事项：**
- type=19 不走 lotus/gRPC 远程调用，始终查询本地 identityDB
- 受 `IDMapAuthMiddleware` 保护：有 `lotus_password` 时需 HMAC 认证，无密码时仅允许本地访问
- 数据量较大时响应可能较慢，建议仅在调试时使用

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

## 批量写与读缓存（Rust 实现）

> 本节只描述 `crates/gensokyo-storage` 的本地实现。表名、key 与 value 的字节编码仍与 Go 逐字节一致；
> 下面两项只改「提交次数」与「读从哪里来」，不改任何可观测结果。

### 事件内批量写（`IdMapStore::with_write_batch`）

同一条消息事件要落的 idmap 写入（用户/群身份、虚拟 message_id、群注册 `WriteConfigv2`）合并进
同一个写事务：

- 闭包返回 `Ok` 才提交；闭包报错或提交失败时未提交事务整批丢弃（回滚），入站层把这四项一律降级
  为原始字符串上报。
- 身份库与消息库是两个 redb 文件，跨文件没有真原子：身份库先提交，消息库提交失败时身份写入保留
  （与 Go 两个 bbolt 库各自 `Update` 同口径）。
- 与 Go 唯一的差异在失败路径：Go 逐条提交会保留失败点之前的写入，这里整批回滚。
- 闭包里只能调 `IdMapWriteBatch` 的方法：redb 同一库只允许一个写事务，在闭包里调 store 的写方法
  会等同库事务提交而卡住。

### 读路径热查询缓存

`lookup_virtual_value`（身份 → vUIN）与 `retrieve_row_by_id`（vUIN → 原始身份串）带一个进程内
有界 LRU，命中结果与「查不到」都缓存：

| 旋钮 | 默认 | 关闭方式 |
|------|------|----------|
| `IdMapOptions::lookup_cache_capacity` | 4096 条 | 填 `0` |
| `IdMapOptions::lookup_cache_ttl` | 60 秒 | 填 `Duration::ZERO` |

- **写后立即可读**：身份分配、`force_unbind`、批量写提交后精确失效受影响的正查/反查 key。
- **导入器整片失效**：JSONL 导入直连库句柄、绕过 store 的写路径，导入提交后调
  `invalidate_lookup_cache()`。
- **并发**：有两种失败形态——① **命中窗口**：提交成功到失效执行之间，`get` 仍可能返回提交前
  的旧值（写入方让失效紧跟提交，把窗口压到最短）；② **回填窗口**：读事务在提交前取到旧值、
  在失效后才回填，读方比对代际号，读窗口内有提交就放弃这次回填。最坏少缓存一次或短暂旧读，
  不会长期留下提交前的值。
- **多实例**：redb 对库文件加排他锁，同一份库只有一个写进程；跨进程形态走 gRPC 由宿主进程独占，
  缓存不越进程边界。只有「绕过 store 直接改库」的外部路径才会在 TTL 内看到陈旧读，仓库内没有
  这种路径（导入器已显式失效）。
- **关闭缓存零行为变化**：容量 0 或 TTL 0 时读写都直接打库，返回值与开着时逐值相同。

### 实测数量级

debug 构建、单机、`cargo test -p gensokyo-storage -- --ignored --nocapture perf_probe`（无 LTO，
绝对值不代表发布构建，只看数量级）：

| 场景 | 结果 |
|------|------|
| 4000 次身份写入：逐条提交 vs 每轮 200 个一批 | 14.1s → 0.34s（约 42×） |
| 4000 次 `lookup_virtual_value`：缓存开 vs 关 | 25.8ms → 72.2ms（约 2.8×） |

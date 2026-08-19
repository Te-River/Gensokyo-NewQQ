# 入群自动审批策略

> 🧪 **测试插件**：[join_approval](https://github.com/Te-River/Gensokyo-NewQQ-Test-plugins/tree/main/join_approval) — 入群申请转发审批（配合 `get_group_join_request_list` / `set_group_add_request`）

范围：`-`（不区分群，策略关联群由参数指定）

入群自动审批策略（QQ 官方 v2 接口 `/v2/groups/join_approval_strategy*`）：为群配置白名单，命中白名单 QQ 号码的入群申请自动通过。一个机器人最多 **20 个策略**；设置的规则仅在机器人拥有群管理员身份时生效。

本组扩展 API 包含 6 个 action：

| Action | 对应 QQ 接口 | 说明 |
|--------|-------------|------|
| [`join_approval_strategy_create`](#join_approval_strategy_create) | `POST /v2/groups/join_approval_strategy` | 创建策略 |
| [`join_approval_strategy_list`](#join_approval_strategy_list) | `GET /v2/groups/join_approval_strategy` | 查询策略列表 |
| [`join_approval_strategy_update`](#join_approval_strategy_update) | `PATCH /v2/groups/join_approval_strategy/{strategy_id}` | 修改策略 |
| [`join_approval_strategy_execute`](#join_approval_strategy_execute) | `POST /v2/groups/join_approval_strategy/{strategy_id}/execute` | 执行策略（全量扫描） |
| [`join_approval_strategy_whitelist`](#join_approval_strategy_whitelist) | `POST /v2/groups/join_approval_strategy/{strategy_id}/whitelist_users` | 修改策略白名单 |
| [`join_approval_strategy_delete`](#join_approval_strategy_delete) | `DELETE /v2/groups/join_approval_strategy/{strategy_id}` | 删除策略 |

---

## `join_approval_strategy_create`

创建入群自动审批策略，`strategy_id` 由服务端生成。

### 参数

| 参数 | 类型 | 必填 | 说明 |
|------|------|:----:|------|
| `group_openids` | array | 条件必填 | 关联的群 OpenID 列表，最多 100 个；与 `group_ids` 互斥 |
| `group_ids` | array | 条件必填 | 关联的 QQ 群号列表（uint64），最多 100 个；与 `group_openids` 互斥 |
| `is_enable` | string | 否 | `on`-启用 / `off`-关闭，默认 `on` |
| `expire_at` | string | 否 | 过期时间（RFC3339 格式）；不传默认一年过期 |
| `remark` | string | 否 | 策略备注，最多 255 个汉字 |

`group_openids` 与 `group_ids` 二选一必填，同时传入或均未传入均返回错误。

### 返回

| 字段 | 类型 | 说明 |
|------|------|------|
| `strategy_id` | string | 服务端生成的策略 ID |
| `is_enable` | string | 是否启用 |
| `expire_at` | string | 过期时间 |

### 示例

```json
{
  "action": "join_approval_strategy_create",
  "params": {
    "group_ids": [870389197, 870389198],
    "is_enable": "on",
    "remark": "读书会群自动放行"
  }
}
```

---

## `join_approval_strategy_list`

查询当前生效中的策略列表，按创建时间倒序。

### 参数

| 参数 | 类型 | 必填 | 说明 |
|------|------|:----:|------|
| `cursor` | string | 否 | 分页游标，首次请求可不传或传空串 |
| `limit` | int | 否 | 单页数量，默认 20，最大 100 |

### 返回

| 字段 | 类型 | 说明 |
|------|------|------|
| `strategies` | array | 生效中的策略列表 |
| `next_cursor` | string | 下一页游标，空串表示已到末页 |

`strategies[]` 元素：

| 字段 | 类型 | 说明 |
|------|------|------|
| `strategy_id` | string | 策略 ID |
| `group_openids` | array | 关联的群 OpenID 列表（创建时使用 `group_openids` 时返回） |
| `group_ids` | array | 关联的 QQ 群号列表（创建时使用 `group_ids` 时返回） |
| `whitelist_user_count` | int | 白名单中的号码数量（估算，可能存在少量误差） |
| `is_enable` | string | 策略是否启用 |
| `expire_at` / `created_at` / `updated_at` | string | RFC3339 格式时间 |
| `remark` | string | 策略备注 |

### 示例

```json
{
  "action": "join_approval_strategy_list",
  "params": {
    "limit": 20
  }
}
```

---

## `join_approval_strategy_update`

修改策略的生效状态、失效时间或增删关联群。

### 参数

| 参数 | 类型 | 必填 | 说明 |
|------|------|:----:|------|
| `strategy_id` | string | 是 | 策略 ID |
| `is_enable` | string | 否 | `on`-启用 / `off`-关闭 |
| `expire_at` | string | 否 | 过期时间（RFC3339 格式） |
| `remark` | string | 否 | 策略备注 |
| `op` | string | 条件必填 | 关联群操作：`add`-新增关联群 / `del`-删除关联群 |
| `group_openids` | array | 条件必填 | 待操作的群 OpenID 列表；与 `group_ids` 互斥 |
| `group_ids` | array | 条件必填 | 待操作的 QQ 群号列表；与 `group_openids` 互斥 |

`op` + `group_openids`/`group_ids` 组合成 `group_action` 增删关联群，群标识形式须与创建时一致。

### 返回

| 字段 | 类型 | 说明 |
|------|------|------|
| `is_enable` | string | 是否启用 |
| `expire_at` | string | 过期时间 |

### 示例

停用策略：

```json
{
  "action": "join_approval_strategy_update",
  "params": {
    "strategy_id": "st_d83eca11e9",
    "is_enable": "off"
  }
}
```

为策略增加一个关联群：

```json
{
  "action": "join_approval_strategy_update",
  "params": {
    "strategy_id": "st_d83eca11e9",
    "op": "add",
    "group_ids": [870389199]
  }
}
```

---

## `join_approval_strategy_execute`

对策略关联的全部群发起全量扫描，命中白名单号码的入群申请自动审批通过。异步执行，约 10 分钟完成。

### 参数

| 参数 | 类型 | 必填 | 说明 |
|------|------|:----:|------|
| `strategy_id` | string | 是 | 策略 ID |

### 返回

无 `data`。

### 示例

```json
{
  "action": "join_approval_strategy_execute",
  "params": {
    "strategy_id": "st_d83eca11e9"
  }
}
```

---

## `join_approval_strategy_whitelist`

对指定策略批量新增或删除白名单 QQ 号码，单次最多 10000 个，号码上限 10W。

### 参数

| 参数 | 类型 | 必填 | 说明 |
|------|------|:----:|------|
| `strategy_id` | string | 是 | 策略 ID |
| `op` | string | 是 | 操作类型：`add`-新增号码 / `del`-删除号码 |
| `whitelist_users` | array | 是 | QQ 号码列表（字符串，避免 JS 精度问题），单次最多 10000 个 |

### 返回

| 字段 | 类型 | 说明 |
|------|------|------|
| `strategy_id` | string | 策略 ID |
| `whitelist_user_count` | int | 操作后策略当前白名单号码数（估算） |
| `updated_at` | string | 策略更新时间（RFC3339 格式） |

### 示例

```json
{
  "action": "join_approval_strategy_whitelist",
  "params": {
    "strategy_id": "st_d83eca11e9",
    "op": "add",
    "whitelist_users": ["1234567", "1234568"]
  }
}
```

---

## `join_approval_strategy_delete`

删除指定的入群自动审批策略。

### 参数

| 参数 | 类型 | 必填 | 说明 |
|------|------|:----:|------|
| `strategy_id` | string | 是 | 策略 ID |

### 返回

无 `data`。

### 示例

```json
{
  "action": "join_approval_strategy_delete",
  "params": {
    "strategy_id": "st_d83eca11e9"
  }
}
```

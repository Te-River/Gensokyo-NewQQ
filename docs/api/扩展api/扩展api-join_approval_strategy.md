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
| `group_openids` | array | 条件必填 | 关联群列表（GSK 虚拟群号或 32 位群 OpenID），最多 100 个 |
| `group_ids` | array | 条件必填 | 同 `group_openids`（两个字段等价，插件填错字段也照处理） |
| `is_enable` | string | 否 | `on`-启用 / `off`-关闭，默认 `on` |
| `expire_at` | string | 否 | 过期时间（RFC3339 格式）；不传默认一年过期 |
| `remark` | string | 否 | 策略备注，最多 255 个汉字 |

`group_openids` 与 `group_ids` 任填其一（都不填由上游回错）。**两个字段的值按同一口径换算**：

- 32 位 OpenID → 直接使用；
- 其余（GSK 虚拟群号，如 `610636458`）→ 经 idmap 反查成群 OpenID；查不到回 `1400`，`message` 里带上字段名与该值。

上层发往官方接口时统一填 `group_openids`——官方 `group_ids` 收的是真实 QQ 群号，而插件侧只有虚拟号、服务端也没有虚拟号 → 真实群号的映射。

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
    "group_ids": [610636458, 610636459],
    "is_enable": "on",
    "remark": "读书会群自动放行"
  }
}
```

> `610636458` / `610636459` 是 GSK 虚拟群号（`string_ob11: false` 的数字形态），服务端会换算成真实群 OpenID 再上行。

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
| `group_openids` | array | 关联群的真实群 OpenID 列表（服务端创建/修改时统一用这个字段） |
| `group_ids` | array | 关联的真实 QQ 群号列表（仅策略由官方侧以 `group_ids` 建过时出现） |
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
| `group_openids` | array | 条件必填 | 待操作的群列表（GSK 虚拟群号或 32 位群 OpenID） |
| `group_ids` | array | 条件必填 | 同 `group_openids`（两个字段等价） |

`op` + `group_openids`/`group_ids` 组合成 `group_action` 增删关联群，取值与创建同口径：32 位 OpenID 直通、虚拟群号经 idmap 反查，查不到回 `1400`（`message` 带上字段名与该值）。

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
    "group_ids": [610636460]
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

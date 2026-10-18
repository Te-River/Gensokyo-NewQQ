# Changelog — Release016

> 自 Release015 以来的所有变更。

---

## ✨ 新功能 / 变更

### 生产主链接入 `internal/` + `adapter/` 分层架构（`arch_mode` 切换，2026-10-07）

生产主链接入 `internal/`（`application/`、`domain/`、`infrastructure/`）+ `adapter/`（`qq/`、`onebot/`、`media/`、`state/`、`identity/`）分层架构，与 legacy 两路同时装配，按 `arch_mode` 逐请求切换，legacy 保留回退：

- **配置**：新增 `arch_mode`（`legacy|shadow|new`，默认 `new`）；逐请求读取、热重载即时生效，空值/非法值回退 `new`；`legacy` 一键回退，行为零变化
- **六个接缝**：出站（`handlers/outbound_bridge.go`）、入站（`Processor/inbound_bridge.go`）、action（`callapi/dispatch_bridge.go`）、state（`handlers/state_bridge.go`）、config（`internal/infrastructure/config/bootstrap.go`）、media（`handlers/media_bridge.go`）；新链未注入/失败时自动落回 legacy
- **装配**：`main.go` 经 `configbootstrap.Bootstrap("config.yml")` 加载配置快照并注入新旧两路；bootstrap 失败时降级 legacy 语义（不 panic）

### 群管理三接口对齐官方现文档（2026-10-18）

`get_group_bot_state` / `get_group_join_request_list` / `set_group_ban`·`set_group_whole_ban` 三个接口对齐 QQ 官方现文档；字段对齐，旧字段保留为兼容别名：

- **`get_group_bot_state` 兼容双写**（`handlers/get_group_bot_state.go`、`botgo/dto/group.go`）：新字段 `joined_at`（RFC3339 原样）/`allow_proactive_msg`/`recv_msg_setting`/`member_role` 与官方一致；旧字段 `join_time`（由 `joined_at` 解析出的秒级时间戳，空串/非法格式置 0）/`can_push`/`push_msg_setting`/`role` 保留为别名
- **`get_group_join_request_list` 分页与响应对齐**（`handlers/get_group_join_request_list.go`）：请求改为官方 `cursor`（string，优先）/ `limit`（默认 20、上限 50），旧 `next_index`（int）为兼容回退；响应 `list` + `next_cursor`，并保留 `join_requests`（与 `list` 同内容）/ `next_index`（`next_cursor` 可转整数时同步，否则 0）兼容别名；元素新增 `union_openid`/`bot`/`risk_tips`/`review_qa_list`，`apply_at` 由秒级时间戳改为 RFC3339 字符串
- **禁言改官方增量语义**（`handlers/set_group_helpers.go`）：`set_group_ban` 请求体为 `members[{op,member_openid,mute_expire_at}]`——`duration>0` 时 `op=add` + `mute_expire_at=now+duration`（RFC3339），`duration<=0` 时 `op=del` + 空 `mute_expire_at`（立即解除）；官方 POST 按成功 op 增量生效，不再先查询当前设置再整体提交
- **`set_group_whole_ban` 与 `[CQ:set_group,action=whole_ban]` 固定失败**（`handlers/set_group_whole_ban.go`、`handlers/set_group_helpers.go`、`handlers/cqcode.go`、`handlers/cqparse_exec.go`）：官方 `POST /v2/groups/{group_openid}/restrict_chat_setting` 请求体只有 `members`，**无全员禁言写入字段**（`global_rule.mode` 仅出现在查询响应中），API 路径返回 `retcode=100` + message「官方未提供全员禁言写入字段，仅支持按成员禁言」；CQ 码路径无结构化回执，仅记录失败日志，CQ 码仍从文本移除。两条路径**都不再提交任何请求**（此前提交的 `all_mute` 官方并不接受）
- **`GROUP_JOIN_REQUEST` 事件字段对齐**（`Processor/ProcessGroupJoinRequest.go`、`botgo/dto/group.go`）：`apply_at` 兼容 RFC3339 字符串（保留历史数字时间戳兼容）；`auto_approved` 官方为对象（`AutoApprovedInfo{strategy_id}`，仅下行事件携带），改为**对象透传**，并新增布尔伴随字段 `auto_approved_bool`（对象是否存在）

### 入站事件字段富化（2026-09-16 官方新增字段）

`botgo/dto/message.go` 补齐官方 2026-09-16 新增的消息事件字段，`handlers/message_parser.go` 将其降级为 CQ 码 / 消息段上报：

- **DTO 新增**：`message_type`（`0` 文本 / `3` 结构化卡片 / `101` 并行消息 / `102` 聊天记录 / `103` 引用消息）、`ark_data`（`prompt`/`ark_type`/`ark_name`/`fields`，`ark_type` 含官方 2026-09-16 新增的 `picture`）、`msg_elements`（递归结构）、附件 `voice_wav_url`/`asr_refer_text`；`MediaResponse` 新增 `id`/`raw_url`
- **字符串路径**（默认 `array: false`）在图片 CQ 码之后追加：`[CQ:record,file=<voice_wav_url 或附件 url>,asr=<转义文本>]`（`asr` 为空则省略）、`[CQ:ark,ark_type=..,ark_name=..,prompt=..]`（无 `ark_data` 时输出裸 `[CQ:ark]`）、`[CQ:reply,msg_idx=..]`，以及嵌套元素正文（`101` 并行消息 / `102` 聊天记录按正文保留，不静默丢弃）
- **数组路径**（`array: true`）追加 `record`/`ark`/`reply`/`text` 段，排在图片段之后
- **出站兼容**（`handlers/cqcode.go`）：`[CQ:record]` 四个正则排除尾随 `,asr=<文本>`，插件把入站原文回传出站时 URL 捕获组不被 `asr` 参数污染（URL 自身含逗号仍完整保留）

### 富媒体上传阈值对齐官方：5MB 直传上限 + 自动分片

官方 `file_data` 二进制直传上限为 **5MB**，超过必须改走 URL 上传或分片上传：

- **判据收敛**（`handlers/upload_helper.go` 的 `maxDirectFileDataBytes = 5 * 1024 * 1024`）：`needsChunkedUpload` 判据只有一条——超过 5MB 即改走分片上传（`upload_prepare` → 预签名 PUT → `upload_part_finish` → 合并），群聊（`handlers/send_group_msg.go`）/ 单聊（`handlers/send_private_msg.go`）一致
- **软限制降级为提示**：按类型软限制（图片 20MB / 视频 30MB / 语音 20MB / 文件 200MB）不再作为直传判据，仅打印"平台侧将降级为文件类型下发"日志；硬限制 200MB（`getHardLimit`）不变
- **入口重构**：`images.CreateAndUploadMediaMessage` / `CreateAndUploadMediaMessagePrivate` 调用点统一换为 `handlers` 包内的 `createAndUploadMediaMessage` / `...Private` 包装（超限先分流，≤5MB 仍交 `images` 包走 `file_data` 直传）；`images/upload_api.go` 同步补充字段

---

## 🐛 修复

### 私聊 markdown 段内嵌 keyboard 按钮权限 `permission.type=0` 原样发出（点击提示"无权限操作"）

私聊 `[CQ:markdown]` 段内嵌 keyboard（markdown 段 `data.data` 同时含 `markdown` 与 `keyboard`，qbind 等下游插件的真实形态）路径漏调 `ResolvePlaceholderUserIDs`，QQ 官方 C2C 单聊不支持的按钮 `permission.type=0`（指定用户可操作）原样发给 QQ，用户点击按钮被服务端拒绝并提示"无权限操作"：

- **修改**：`HandleSendPrivateMsg` 的 markdown 分支在 `ResolveKeyboardImages` 前补调 `idmap.ResolveOriginalID(UserID)` + `ResolvePlaceholderUserIDs(kb, userOpenID)`，与同文件独立 `[CQ:keyboard]` 路径及 `generatePrivateMessage`（定义于 `send_group_msg.go`、私聊循环路径使用）行为对齐
- **影响**：内嵌键盘与独立 `[CQ:keyboard]` 行为一致——`specify_user_ids` 非空且 `permission.type=0` 时转为 `type=2`（所有人），`specify_user_ids` 中 `__USER_ID__` 占位符替换为实际用户 OpenID；`specify_user_ids` 列表保留不清空
- **测试**：新增回归测试 `TestSendPrivateMsgEmbeddedKeyboardPermissionType`（`handlers/send_msg_md_gate_test.go`），构造 markdown 段内嵌 keyboard（`permission.type=0` + `specify_user_ids`），断言出站 `MsgType=2`、`Permission.Type=2`（修复前为 0）、`specify_user_ids` 保留非空

### `get_group_bot_state` 字段名与官方不符，按官方字段取值恒空

官方 `GET /v2/groups/{group_openid}/bot_state` 现文档返回 `joined_at`/`allow_proactive_msg`/`recv_msg_setting`/`member_role`，DTO 却按 `join_time`/`can_push`/`push_msg_setting`/`role` 取，下游按官方新字段读取恒为空：

- **修改**：`botgo/dto/group.go` 补官方字段，`handlers/get_group_bot_state.go` 同时输出新旧两套字段——`join_time` 由 `joined_at` 解析（RFC3339 → Unix 秒，解析失败记日志并置 0）
- **影响**：按官方字段名的下游插件可正常取值；旧插件继续读别名，行为不变

### `get_group_join_request_list` 请求/响应字段与官方不符（`next_index`→`cursor`、`join_requests`→`list`）

官方入群申请列表按 `cursor`（string）+ `limit` 分页，响应为 `list` + `next_cursor`，`apply_at` 为 RFC3339 字符串；此前 Gensokyo 按 `next_index` 请求、返回 `join_requests`，`apply_at` 为秒级 int，下游按官方字段解析恒为空：

- **修改**：`handlers/get_group_join_request_list.go` 请求改取 `cursor`（string，优先）/ `limit`（默认 20、上限 50），`next_index`（int）作为兼容回退；响应输出 `list` + `next_cursor`，`join_requests` / `next_index` 保留为兼容别名；新增 `union_openid`/`bot`/`risk_tips`/`review_qa_list`，`apply_at` 改 RFC3339 字符串
- **影响**：官方 SDK 与下游插件可按现文档直接解析；仍返回旧字段名，老插件不破

### `auto_approved` 官方为对象而 DTO 为 bool，导致事件解析失败

`GROUP_JOIN_REQUEST` 事件的 `auto_approved` 官方为对象（`{strategy_id}`，仅下行事件携带），旧 DTO 声明为 bool，JSON 反序列化报错导致整个事件解析失败：

- **修改**：`botgo/dto/group.go` 新增 `AutoApprovedInfo{StrategyID}` 并以 `*AutoApprovedInfo` 承接；`Processor/ProcessGroupJoinRequest.go` 输出对象透传 `auto_approved`，另加布尔伴随字段 `auto_approved_bool`
- **影响**：事件不再解析失败；下游既能拿策略 ID，也能用布尔字段做简单判断

### 错误码表补码（16 个官方错误码）

`handlers/qq_error_codes.go` 补齐官方文档中缺失的错误码，调用失败时控制台可输出中文描述：

- **新增**：富媒体上传 `10000`/`850018`/`850019`/`850026`/`850027`/`850031`/`40093001`/`40093002`，群聊 v2 撤回 `40061001`/`40062003`/`40064004`/`50065001`，取 token `100001`/`100007`/`100016`
- **描述更新**：`11253` 改为官方措辞「应用无接口访问权限」
- **双语义说明**：`10004` 在不同接口语义不同（频道/Guild vs 取 token 的"机器人不存在"），以注释说明避免混淆

---

## 📝 工程与文档变更

### 文档同步（群管理三接口 + 入站新字段）

- `docs/api/扩展api/扩展api-get_group_bot_state.md`：字段表与示例改为官方字段 + 兼容别名
- `docs/api/扩展api/扩展api-get_group_join_request_list.md`：请求/响应改为 `cursor`/`list`/`next_cursor`，示例改为游标分页循环
- `docs/cq码/扩展CQ码/扩展cq码-cq-set_group.md`：补 `whole_ban` 固定失败说明（`set_group_ban` 增量语义）
- `docs/cq码/标准CQ码/标准cq码-cq-file.md`：补 5MB 直传上限与自动分片说明
- `docs/本版新增功能.md`：`API` 表三行更新、新增「入站事件新增字段（2026-09-16）」「富媒体上传：5MB 直传上限与自动分片」小节、`事件字段` 补 `GROUP_JOIN_REQUEST`、错误码补码说明

### 新增测试文件

- `handlers/group_contract_test.go`：bot_state / join_request_list / restrict_chat_setting 的 DTO 反序列化契约，`set_group_whole_ban` 明确失败回执
- `handlers/message_event_fields_test.go`：`ark_data`/`msg_elements`/语音附件/引用消息的段与 CQ 码降级、图片回归、`MediaResponse` 新字段
- `handlers/upload_threshold_test.go`：分片上传判据只由 5MB 上限决定（与软限制解耦）
- `handlers/cq_record_param_test.go`：`[CQ:record]` 正则捕获组语义（尾随 `,asr=` 不污染 URL、URL 含逗号不误切、管道级 `asr` 不泄漏进正文）
- `handlers/qq_error_codes_test.go`：已知/未知错误码格式化

---

## ✅ 提交记录

```
<!-- 由 lead 在 commit 后填入 <hash>  <subject> -->
```

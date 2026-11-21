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

- **判据收敛**（`handlers/upload_helper.go` 的 `maxDirectFileDataBytes`，取值见下文「官方口径再对齐」）：`needsChunkedUpload` 判据只有一条——超过 5MB 即改走分片上传（`upload_prepare` → 预签名 PUT → `upload_part_finish` → 合并），群聊（`handlers/send_group_msg.go`）/ 单聊（`handlers/send_private_msg.go`）一致
- **软限制降级为提示**：按类型软限制（图片 20MB / 视频 30MB / 语音 20MB / 文件 200MB）不再作为直传判据，仅打印"平台侧将降级为文件类型下发"日志；硬限制 200MB（`getHardLimit`）不变
- **入口重构**：`images.CreateAndUploadMediaMessage` / `CreateAndUploadMediaMessagePrivate` 调用点统一换为 `handlers` 包内的 `createAndUploadMediaMessage` / `...Private` 包装（超限先分流，≤5MB 仍交 `images` 包走 `file_data` 直传）；`images/upload_api.go` 同步补充字段

### 自定义数据库位置：本地/网络挂载路径 + 自动迁移（idmap 族一期，2026-11-17）

新增 `custom_db_enabled`（默认 `false`）/ `custom_db_path` 配置，可将 idmap 族数据库置于用户指定位置（本地相对/绝对路径、UNC、宝塔等面板挂载后的网络存储目录）：

- **修改**：新增独立单元 `dblocation/`（注册表 + 路径解析 + bbolt 锁探针 + 迁移裁决 + `Resolve`）；idmap 族五处打开点（`idmap/service.go`、`new_service.go`、`map_service.go`）全部经 `Resolve` 取路径，旧库惰性迁移逻辑原样保留在解析后的目录内
- **修改**：`main.go` 启动序列在任何数据库打开前挂接 `dblocation.Setup()`；五处打开点补 `30s` `Timeout`；`Compaction` 源/目标路径经 `Resolve`；`template/config_template.go` 与 `readme.md` 同步说明
- **影响**：开关关闭时行为零变化；开启后——目标有族内同名文件 → **拒绝启动**（防数据分叉）；本地有目标无 → 文件级复制 + 校验（大小 + 只读可打开），本地原件改名 `.bak.<时间戳>` 保留；复制/校验失败删目标残留并拒绝启动；协议 URL（含 `://`）拒绝并提示先挂载再用；启用即 WARN 网络文件系统锁风险（依据 `etcd-io/bbolt#257`，按单实例自担风险运行）
- **影响**：注册表驱动，`botstats.db`/`cookie.db`/`gensokyo.db` 等后续加一行即接入；本期不涉及
- **测试**：`dblocation` 表驱动单测 + 补测 + `idmap/dblocation_integration_test.go` 运行时集成用例；黑盒 e2e 六场景（本地有→迁移 / 目标有→直接用 / 两侧都有→拒绝启动 / `smb://` 拒绝 / 两边无 / 关闭零变化）通过；`go build ./...`、`go vet ./...`、`go test ./...` 全绿

### 入站 sender 昵称/群名片接入 `member_nickname`（2026-11-16 官方字段，2026-11-20）

QQ 官方 2026-11-16 起群消息事件的 `author` 新增 `member_nickname`（群内昵称/群名片），单聊事件也带发送者昵称；本仓 `botgo/dto` 的 `User` 无该字段，解析时被直接丢弃，`sender.nickname` 只认 `card_and_nick` 配置与 `username`（私聊则恒为空串）：

- **修改**：`botgo/dto/user.go` 的 `User` 新增 `MemberNickname`（JSON `member_nickname`）；新增纯函数 `resolveNicknameAndCard`（`Processor/nickname.go`），入站事件上报的 sender.nickname/card 统一走它——群聊四条路径（`Processor/ProcessGroupMessage.go`、`Processor/ProcessGroupNormalMessage.go` 各自 int64 / `string_ob11` 两分支）；`GROUP_MSG_RECEIVE` / `GROUP_MSG_REJECT` / `INTERACTION` 三个事件（`Processor/ProcessGroupMsgReceive.go`、`ProcessGroupMsgReject.go`、`ProcessInlineSearch.go`）不携带作者昵称，同样统一走该函数，行为仍只受 `card_and_nick` 配置影响
- **优先级**：`sender.nickname` 与 `sender.card` 均为 `card_and_nick` 配置（非空，同时覆盖两者） > `member_nickname` > `username`（`username` 只写 `nickname`，不写 `card`）；两者都空时 `nickname` 为空串，且不设置 `card`
- **私聊**：`sender.nickname` 由恒定空串改为 `member_nickname || username`（`Processor/ProcessC2CMessage.go` 的 int64 / `string_ob11` 两个 sender 与好友列表缓存 `structs.FriendData.Nickname`），私聊不设置 `card`（`PrivateSender` 无该字段）
- **影响**：群内 `sender.card` 现在可能是群名片而非空；`card_and_nick` 配置的优先级与取值不变，配置为空且事件无 `member_nickname` 时行为与旧版一致
- **测试**：`Processor/nickname_test.go` 表驱动覆盖群聊三档优先级、私聊回退、双空与 `author` 为 nil

### 发送类 action 上游失败改回 `retcode:1500` / `status:"failed"`（对外行为变更，2026-11-21）

发送类 action（`send_group_msg` / `send_private_msg` / 频道与 C2C 各变体，回执统一由 `handlers/message_parser.go` 的 `SendResponse`、`SendResponseSB`、`SendGuildResponse`、`SendC2CResponse`、`SendGuildPrivateResponse` 构造）上游调用失败时，此前恒回 `retcode:0` + `status:"ok"`，只在 `message` 里塞错误文本，调用方无法进入错误分支；活体差分显示 Rust 侧同类失败回非零码。现改为失败回 `retcode:1500` + `status:"failed"`，与 Rust 侧约定对齐（`1400` 参数/协议、`1404` 未知 action、`1500` 上游/存储/IO 失败、`100` 管理类失败）：

- **修改**：五处回执构造统一使用包级常量 `sendFailRetCode = 1500` 与 `sendFailureMessage(err)`（`handlers/message_parser.go`）；成功分支仍为 `retcode:0` / `status:"ok"`，`message` 为空
- **异步审核提示保留**：`message` 为上游原始错误文本 + 「（QQ 官方审核为异步流程：若为审核中，消息可能已发出）」——既让调用方能识别失败，也不会让人误判消息一定没发出
- **影响**：**对外行为变更**。依赖"发送恒成功"的下游（如只判 `retcode` 就认为已发出的插件）在审核中/频控/事件过期等场景会看到 `failed`，需自行决定是否重试；`data.message_id` 语义不变
- **文档**：`docs/本版新增功能.md` 的 API 段补充该回执约定

### 新增 `get_user_info`：获取用户基本信息（2026-11-16 官方新增接口）

QQ 官方 2026-11-16 新增 `GET /v2/users/{user_openid}/info`，返回用户昵称、是否允许主动推送与头像直链：

- **新增**：`handlers/get_user_info.go` 注册 action `get_user_info`；`botgo` fork 增加 `dto.UserInfo` 与 `openapi.UserAPI.UserInfo`（v2 实现，v1 明确不支持）
- **入参**：`{"user_id": <string|number>}`——虚拟号走 idmap 反查，32 位原生 openid 直接使用（与 `get_avatar` 同口径）
- **回执**：成功 `{"retcode":0,"status":"ok","data":{"user_id"(原样回显),"username","allow_proactive_msg","avatar"},"message":""}`；上游失败 `retcode:1500` + `status:"failed"` + `data:null` + 上游错误文本；`user_id` 缺失/非法 `retcode:1400`
- **测试**：`handlers/get_user_info_test.go` 覆盖缺参 1400 / 上游成功（`data` 四字段）/ 上游失败 1500

### 群类事件透传 `union_group_openid`（2026-11-16 官方新增字段）

群相关事件自 2026-11-16 起新增跨应用群标识 `union_group_openid`：

- **修改**：`botgo/dto` 的群消息事件（`dto.Message`，即 `WSGroupMessageData`/`WSGroupATMessageData`）、`GroupAddBotEvent`、`GroupMemberEvent`、`GroupJoinRequestEvent`、`GroupInfo` 增加 `UnionGroupOpenID`
- **上报**：`GROUP_MESSAGE_CREATE`、`GROUP_AT_MESSAGE_CREATE`、`GROUP_ADD_ROBOT`、`GROUP_DEL_ROBOT`、`GROUP_MEMBER_ADD`、`GROUP_MEMBER_REMOVE`、`GROUP_JOIN_REQUEST` 的上报 JSON 顶层原样带出 `union_group_openid`，**非空时才出现**（`omitempty`）；不塞进 `group_id`、不改写现有字段
- **测试**：`Processor/zzzzzz_union_group_openid_test.go` 覆盖带字段→上报含它、不带→不出现（成员变动 / 入群申请 / 群消息三条路径）

### `send_private_msg` 支持显式 `message_id`：群消息 `msg_id` 可用于私聊被动回复（2026-11-17 官方放开）

QQ 官方自 2026-11-17 起允许群消息事件的 `msg_id` 直接用于私聊接口 `POST /v2/users/{openid}/messages`；Rust 侧已支持（`plan_send_private_msg` 把 `params.message_id` 映射为被动凭据），Go 侧此前只查 lazy 池 / echo 键 / 用户-群映射，**从不读 `params.message_id`**，凭据被静默丢弃：

- **修改**：`handlers/send_private_msg.go` 的 msg_id 解析链最前面新增显式来源——`params.message_id` 优先于 lazy 池与 echo 缓存；命中后不再回落到缓存查询
- **修改**：新链同步（`handlers/outbound_bridge.go`）——`outboundMsgCtx` 新增 `explicitMsgID`/`hasExplicitMsgID`，由 `outboundMsgCtxFor` 从请求参数填充；`resolvePrivateMessageContext` 与 `resolveGroupMessageContext` 均以它优先
- **边界**：`message_id: "0"` 沿用主动推送约定（不设 `msg_id`，且不回落缓存）；`[CQ:active]` 仍强制清空 `msg_id` / `event_id`（优先级不变）；参数校验与回执形状不变
- **测试**：`handlers/send_private_msg_msgid_reply_test.go`（显式 `message_id` 透传为 `MessageToCreate.MsgID`、`EventID` 为空）

---

## 🐛 修复

### `send_group_msg` 缺参导致无限递归 → `fatal error: stack overflow` 进程退出（1400 回执替代）

直接向实例发 `{"action":"send_group_msg","params":{"message":"x"}}`（不带 `group_id`/`user_id`，或 `group_id` 传空串 / `0`）会让整个进程退出。`ParamsContent.UnmarshalJSON` 把缺失字段归一为空串 `""`，而参数守卫把空串判为「有效 id」（空串既不等于 `nil` 也不等于 `"0"`）→ 不返回 1400；随后递归分支在 `idInt64 == 0` 时无条件自调用，注入的 msgType 写入 echo 键 `appID_0`，探测读的却是 `appID_`，永远读不回 → 无限递归 → `fatal error: stack overflow`（Go 无法 recover）。实例日志表现为 `send_group_msg获取到信息类型:` 重复上万次后进程消失。

- **修改**：`handlers/send_group_msg.go` —— 入口统一用 `groupIDStr`/`userIDStr` 两个安全类型断言取值（缺省不再 panic），`hasUsableID`（非空且非 `"0"`）替代把空串判为有效的 `checkZeroGroupID`/`checkZeroUserID` 闭包；缺参与 `group_id:0` 由守卫回 1400
- **修改**：递归前必须可推进——`err == nil`、`idInt64 > 0`、且该 id 尚未被探测过（`echo.GetMapping(idInt64) == 0`）；递归一次后仍无消息类型直接回 1400（`无法确定消息类型: group_id/user_id 无效`），结构上不可能二次递归
- **修改**：`handlers/send_private_msg.go` 同构修复（原递归的返回值还被丢弃），`handlers/send_msg.go` 同一递归形态一并收口；`handlers/send_group_msg_raw.go` 的递归目标即 `send_group_msg`，随其收敛
- **影响**：可发送路径行为不变（探测成功时仍按 `group_private` → `group` 枚举）；缺参 action 由「打死进程」变为一条 `retcode=1400`、`status=failed`、`data=null` 回执
- **测试**：`handlers/send_msg_missing_id_test.go` 覆盖 `send_group_msg` 无 id / `group_id:""` / `group_id:0` 与 `send_private_msg` 无 id / `user_id:""`，断言收到 1400 回执、handler 1s 内返回、且 id 0 上未写入递归计数

### 发送类 action 未映射 id 时无回执，客户端只能等超时

`send_private_msg` 传入未在 idmap 中映射的 `user_id` 时，Go 侧在 id 反查失败分支直接 `return "", nil`，反向 WS 客户端收不到任何 action 回执，实测 15 秒后超时断开（日志只有 `Error reading config: key not found` → `Unknown message type:`）。

- **修改**：`handlers/send_private_msg.go` 11 处裸 `return "", nil` 逐个审阅——参数错误 / id 反查失败 / 上游发送失败 / 消息类型断言失败改走 `sendActionResult`（参数与协议错误 1400，上游与内部失败 1500）；已由 `SendC2CResponse` 回执的分支保留并注明「已回执」
- **修改**：同类分支一并处理——`handlers/send_group_msg.go`（7 处）、`send_group_msg_raw.go`（6 处）、`send_msg.go`、`send_group_forward_msg.go`、`send_private_msg_wakeup.go`（4 处）、`send_private_msg_sse.go`（6 处）
- **修改**：`send_group_msg` 非 Pro 分支反查群 OpenID 失败此前只记日志继续执行（群号被覆盖为空串，发送必然失败且无回执），改为返回 1400 回执
- **修改**：`sendActionResult`（`handlers/set_group_add_request.go`）补齐 OneBot V11 回执字段——新增 `data`（失败时 `null`），`echo` 去掉 `omitempty` 恒存在；新增 `sendParamRetCode = 1400` 常量，上游失败复用 `sendFailRetCode`（1500）
- **修改**：`Params` 字段为 `interface{}`，缺省时为 `nil` 而 `nil != ""` 成立——`send_private_msg` / `send_group_msg` 的 ID 转换分支补 `nil` 判空，避免只传 `user_id` 时走进 `group_id` 分支、消息类型枚举递归被跳过（同样表现为无回执）
- **影响**：可发送路径行为不变；原先静默返回的分支现在都会下发一条 `{"status":"failed","retcode":1400|1500,"data":null,"echo":<原样>}`，HTTP 模式返回体同步带上 `data: null`
- **测试**：`handlers/send_action_receipt_test.go` 覆盖「未映射 `user_id`」「未映射 `group_id`」两条路径——断言 mock client 至少收到一次 `SendMessage`（不静默无响应）、`retcode=1400`、`status=failed`、`echo` 原样、`data` 为 `null`；群聊用例额外断言未调用 `PostGroupMessage`

### 查询类 action 未映射 id 时同样无回执（同一病根，一并收口）

`get_group_member_list` 传入未映射的 `group_id` 时，`idmap.ReadConfigv2` 失败分支直接 `return "", nil`——与上一节发送类 action 同一病根：实测客户端 20 秒无响应后断开。对 `handlers/`（含子目录）的 `return "", nil` / `return "", <err>` 逐处普查后，其余未回执分支一并收口。

- **修改**：`handlers/get_group_member_list.go` —— `group_id` 缺失、`ReadConfigv2` 失败（群号未映射）、私聊虚拟群场景、未知群类型四处改为 `sendActionResult` 回执（1400）；入口改为安全的类型断言，缺参不再 panic
- **修改**：`handlers/get_avatar.go` —— `user_id` 反查失败此前仍回 `retcode 0` + 空 openid 拼出的无效头像 URL，改为 1400 失败回执
- **修改**：`delete_msg`（`message_id` 缺失、`user_id` 反查失败）、`put_interaction`（`echo` 非字符串、`post_type` 非法、上游失败）、`get_robot_share_link`（上游失败 1500）、`set_group_ban` / `set_group_whole_ban`（群类型读取失败 / 非群聊场景）改走 `sendActionResult`
- **修改**：`get_user_info` 反查失败口径由 1500 归一到 1400（参数 / id 反查失败），上游失败仍为 1500；历史 handler 的既有回执（如 `get_group_info` 的 `retcode 100`）按 `set_group_add_request.go` 常量注释保留旧口径
- **已回执**：`get_status` / `get_login_info` / `get_online_clients` / `get_version_info` / `mark_msg_as_read` / `get_friend_list` / `get_group_info` / `get_group_list` / `get_group_member_info` / `get_group_member_list` / `delete_msg` / `put_interaction` 的序列化失败分支在 `SendMessage` 之后，保留并注明「已回执」
- **修改**：分发层同病根——`callapi/callapi.go` 的 `callAPIFromDictLegacy` / `callAPIFromDictNew` 在**未知 action** 与 **handler 返回 error** 时此前只记日志 `return ""`（发不存在的 action 时客户端 20 秒超时），现统一经 `actionFailureReceipt` 回执：未知 action `retcode:1404` + `message:"未知 action: <name>"`，handler 异常 `retcode:1500` + `message:"<err>"`，均为 `status:"failed"`、`data:null`、`echo` 原样，返回值仍给出该 JSON 串供 HTTP 传输层使用；`shadow` 的 new 对比执行（dry-run）只构造不发送，避免与 legacy 双发
- **测试**：`callapi/receipt_test.go` 覆盖 legacy/new 两链的未知 action（1404，mock client 恰收到一次 `SendMessage`）与 handler 异常（1500），并断言 dry-run 不下发
- **测试**：`handlers/query_action_receipt_test.go` 覆盖未映射 `group_id` 的 `get_group_member_list`（1400）、未映射 `group_id` 的 `get_group_info`（失败回执 + `echo` 原样）、未映射 `user_id` 的 `get_avatar`（1400）

### 消息已发出但虚拟 message_id 写缓存失败时无回执（成功回执 + `message_id:0`）

`SendResponse` / `SendGuildResponse` / `SendC2CResponse` / `SendGuildPrivateResponse`（`handlers/message_parser.go`）在 `echo.StoreCacheInMemory` / `idmap.StoreCachev2` 写虚拟 `message_id` 缓存失败时，只打一条 `Error storing ID` 日志便 `return "", nil`——消息其实已经发到 QQ，但客户端既收不到 `SendMessage` 回执也收不到 JSON 串，只能等超时断开。

- **修改**：9 处裸返回（`SendResponse` 内存/idmap 两分支、`SendGuildResponse` 内存/idmap/`channel_id` 三处、`SendC2CResponse` 内存/idmap 两分支、`SendGuildPrivateResponse` 内存/idmap 两分支）抽出统一入口 `storeVirtualMessageID` / `cacheMessageIDForReceipt`（按 `memory_msgid` 在内存与 idmap 间切换，保留为用例注入缝）；失败时保留 `Error storing ID: <err>（缓存写入失败，消息已发出，回执 message_id=0）` 日志，`message_id` 以 `0` 兜底后继续走成功分支
- **修改**：`SendGuildResponse` 的 `channel_id`（`idmap.StoreIDv2`）写失败同样不再裸返回——降级为 `channel_id:0`（字段 `omitempty`，回执中省略）后照常回成功
- **修改**：`SendResponse` 的自动撤回增加 `messageID64 > 0` 守卫——缓存失败时没有真实可撤回 id，不再发出 `delete_msg`（`message_id=0` 的无效请求）
- **影响**：这 9 个分支的回执统一为 `retcode:0` / `status:"ok"` / `data:{"message_id":0}` / `message:""` / `echo` 原样。**刻意不回 `status:"failed"`**：消息确已发出，报失败会误导插件重发；上游发送失败（`err != nil`）仍按 1500 失败回执
- **确认为设计行为（无需改动）**：`[CQ:group_info]` 的 `group_id` 反查失败（`handlers/cqparse/handlers_groupinfo.go:114`）在 `ResolveBatch` 内即被降级为 `fallback` 文本（默认空串）+ 日志，不向 action handler 上抛 error；外层发送照常回成功回执，客户端不会超时。该「失败替换 fallback」是既定设计（见 `docs/cq码/扩展CQ码/扩展cq码-cq_group_info.md`「失败分级」），改成 1400 失败回执会与文档及「不泄漏码原文」的取舍冲突
- **测试**：`handlers/cache_receipt_test.go` 覆盖四种回执形态（群 / 频道 / C2C / 频道私信）：注入写缓存失败后断言 mock client 收到 `retcode:0`、`status:"ok"`、`data.message_id:0`、`message:""`、`echo` 原样，且返回值不是空串

### action 回执不再转义 `&`，与 Rust 端逐字节对齐

`get_user_info` 两侧 `data` 内容一致，但 Go 回执把 `&` 转义成 `\u0026`（`"avatar":"https://thirdqq.qlogo.cn/bqqthird?key=…\u0026kti=…\u0026s=640"`），Rust 端原样输出 `&`——凡是带签名的 URL（头像/图床/短链）回执都逐字节不同：

- **修改**：新增 `callapi/marshal.go` 的 `MarshalNoEscape`——`json.Encoder` + `SetEscapeHTML(false)`，裁掉尾随换行，输出与 `json.Marshal` 逐字节对齐，仅 `&`/`<`/`>` 不再被转义
- **修改**：回执序列化全部改走该函数——`handlers/message_parser.go` 的 `SendResponse`/`SendResponseSB`/`SendGuildResponse`/`SendC2CResponse`/`SendGuildPrivateResponse` 与 `ConvertMapToJSONString`；`handlers/` 下 `get_user_info`/`get_avatar`/`get_group_info`/`get_group_list`/`get_friend_list`/`delete_msg`/`delete_group_msg`/`put_interaction`/`get_status`/`get_login_info`/`get_online_clients`/`get_version_info`/`mark_msg_as_read`/`set_group_add_request` 及 `outbound_bridge.go` 的回执
- **修改**：回执还经 `Client.SendMessage` 走 WS 下发，底层序列化同口径——`wsclient/ws.go`、`server/wsserver.go` 的 `SendMessage` 一并关闭 HTML 转义（HTTP API 模式由 handler 返回的字节直接下发，无需二次处理）
- **影响**：带签名的 URL 回执与 Rust 端逐字节一致；输出仍是合法 JSON，解析语义不变。**未改动**发往 QQ 上游的请求体（`cqcode_pipeline.go`/`handlers_content.go` 的 base64 编码、`normalize.go`）与内部存储/日志序列化
- **测试**：`callapi/marshal_test.go` 断言 `https://example.com/a?x=1&y=2` 原样输出、不含 `\u0026`、无尾随换行且往返语义不变；`handlers/action_response_test.go` 经 `get_user_info` 全链路断言回执同样不转义

### 群名片 `member_nickname` 绕过入站敏感词过滤，`sender.nickname` / `sender.card` 原样上报敏感词

入站敏感词过滤只处理 `author.username`，而 `sender.nickname` / `sender.card` 自 2026-11-16 起优先取 `member_nickname`（`Processor/nickname.go` 的 `resolveNicknameAndCard`），群名片含敏感词时会绕过过滤被原样上报：

- **修改**：`Processor/nickname.go` 新增 `FilterAuthorFields(author)`，对 `username` 与 `member_nickname`（非空时）应用同一个 `acnode.CheckWordIN`；`main.go` 三条入站路径（`GROUP_AT_MESSAGE_CREATE`、`C2C_MESSAGE_CREATE`、`GROUP_MESSAGE_CREATE` 全量群消息）统一改调该函数
- **修改**：`GROUP_AT_MESSAGE_CREATE` / `C2C_MESSAGE_CREATE` 的过滤块由「`runWithTimer` 之后」**前移到启动处理器之前**——此前过滤与 `Process*` 并发执行，`nickname` / `card` 可能已用未过滤的值合成；`GROUP_MESSAGE_CREATE` 原本就在处理器之前，仅替换调用
- **影响**：`enable_change_word` 开启时 `sender.nickname` / `sender.card` 与 `username` 口径一致，群名片敏感词不再泄露；过滤关闭时两个字段行为不变
- **测试**：`Processor/nickname_test.go` 新增四组用例——群消息两字段都含敏感词、仅群名片含敏感词、私聊群名片含敏感词、过滤关闭四场景，断言过滤后 `resolveNicknameAndCard` 取到的是替换值

### 私聊 keyboard 按钮权限全类型归一化（`type=1`/`3` 与空名单同样是死按钮）+ 群聊合法 `permission.type` 不再误转

C2C 单聊只支持按钮 `permission.type=2`（所有人）：此前的兜底只处理「`specify_user_ids` 非空且 `type=0`」，插件带 `type=1`（仅管理者）、`type=3`（指定身份组）或不带名单的 `type=0` 时按钮依旧点不动；且该兜底写在群聊、私聊共用的 `ResolvePlaceholderUserIDs` 内，群聊里合法的 `type=0` 名单也被误转成 `type=2`，破坏群权限控制：

- **修改**：`handlers/message_parser.go` 拆分职责——`ResolvePlaceholderUserIDs` 只替换 `specify_user_ids` 中的 `__USER_ID__` 占位符（群聊、私聊共用，不再改动 `permission.type`）；新增 C2C 专用 `NormalizeC2CKeyboardPermission`，遍历全部按钮，`permission.type != 2` 一律置 2（`specify_user_ids` / `specify_role_ids` 原样保留）
- **修改**：私聊每条构造路径都调用两者——`handlers/send_private_msg.go` 三处（markdown 内嵌、独立 `[CQ:keyboard]`、foundItems 循环）、`handlers/outbound_bridge.go` 两处（`applyPrivateMarkdownKeyboard`）、`handlers/send_group_msg.go` 中 `generatePrivateMessage` 的 markdown 分支（`send_private_msg_wakeup` 等私聊路径共用，此前漏归一）
- **影响**：私聊任何非 2 类型按钮统一归一为 2，点击不再提示"无权限操作"；群聊保留插件原始 `permission.type`（`type=0` 指定用户名单照旧生效），群、私聊各自符合 QQ 能力边界
- **测试**：新增 `handlers/keyboard_permission_test.go`——`type=0`（带名单 / 空名单）、`type=1`、`type=2`、`type=3` 归一表驱动；占位符解析不改 `permission.type`；私聊 markdown 内嵌（`type=3`）与独立 `[CQ:keyboard]`（`type=1`）出站断言 `type=2`；`generatePrivateMessage` 私聊分支；群聊 legacy（`HandleSendGroupMsg`）与新链（`SendFoundItemsGroup`）均断言 `type=0` + 名单保留

### 官方口径再对齐（2026-10-19）

按 QQ 官方现文档核对三处口径，均不影响对外契约：

- **修改**：`botgo/token/authtoken.go` 的 `getAppAccessToken` 地址由 `https://bots.qq.com/app/getAppAccessToken` 改为官方 access-token 页（2026-09-16 更新）的 `https://api.bot.qq.com/app/getAppAccessToken`；`develop_acdir` 覆盖逻辑不变
- **修改**：`handlers/upload_helper.go` 的 `maxDirectFileDataBytes` 由 `5 * 1024 * 1024` 改为 `5 * 1000 * 1000`。官方 files 页（2026-10-09）只写"5M / 数据大小不能超过 5M"，未定义十进制/二进制；取保守十进制 5,000,000 字节，超过即自动走分片上传（`upload_prepare` → 预签名 PUT → `upload_part_finish` → 合并），结果仍正确；`defaultBlockSize = 5 * 1024 * 1024` 为分片块大小兜底，未改动
- **修改**：`botgo/dto/user.go` 的 `User` 补 `user_openid` / `member_openid`（`omitempty`），**零行为差异**——业务仍用 `Author.ID`，未改动任何使用方
- **影响**：token 获取改走官方现域名；5,000,001~5,242,880 字节区间从直传改为分片，平台侧接收结果一致；DTO 新字段仅提供下游可选读取
- **测试**：`go build ./...`、`go vet ./...`、`go test ./handlers/ -run 'Upload|Threshold' -count=1`、`cd botgo; go build ./...` 全通过

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

### ChatGLM 短签名外链在出站前解析 302 直链（2026-11-07）

`oss_type=7`（ChatGLM）图床外链带两层短签名（约 30 分钟 + 302 跳转后再叠 15 分钟），旧行为把短链直接写进 Markdown/卡片等出站字段，QQ 服务端不跟随短链的 302，转存拉取失败或用户查看时签名已过期，图片不显示：

- **修改**：`images/upload_api.go` 的 `UploadBase64ImageToServer` 在返回图床外链前先按 `imagehosting.IsShortLivedImageURL` 判定，命中则用 `imagehosting.ResolveRedirectURL` 解析 302 末跳直链（只读响应头、`Range: bytes=0-0`，首跳与每一跳复检 SSRF）并返回该直链；解析失败（无重定向 / 末跳非 2xx / SSRF 阻止 / 超时）回退 `oss_type=0` 的本地公网 URL 链
- **修改**：`handlers/shortlived_image.go` 的 Markdown/keyboard 消费点优先走同一条直链解析（省一次下载与一次上传），解析失败才自取字节走既有上传流程；上传流程若仍回短签名外链，降级路径内再解析一次，成功即采纳直链、失败才拒绝替换并维持原外链（不再沿用旧逻辑"直接使用／直接替换为短链"）
- **影响**：出站字段拿到的是 OSS 预签名直链或本地公网 URL，QQ 可正常拉取；Refactor 分支走 `qqbot-media://` 素材链上传（素材 ID 非外链），不受本修复影响
- **测试**：`handlers/shortlived_image_test.go` 新增直链解析成功分支与降级路径内层解析采纳用例；`images/upload_api_stable_test.go` 覆盖解析成功/回退本地公网 URL

### Markdown 图片改走官方素材管理 API（2026-11-07）

实测即便把 ChatGLM 外链解析为 pfiles 直链、请求体里的 `&` 以原始形式发出，QQ 服务端转存仍然拉取失败。按官方文档「Markdown 消息 > 图片」改用素材管理：

- **修改**：新增 `handlers/media_asset.go`，本地图片字节走 `POST /v2/media-images/prepare`（file_size/md5/sha1/content_type，≤10MB、png/jpeg/gif/webp）→ `PUT` 预签名（不带 Authorization）→ `POST /v2/media-images/complete`（md5 取 PUT 的 ETag，去引号并归一大小写），得到平台素材引用（滑动有效期 90 天，每次被消息引用自动顺延；可复用，发送后不删除） `qqbot-media://image/v1/<token>`
- **接线**：`oss_type=7`（ChatGLM）即「官方素材 API 模式」——Markdown/keyboard 本地图只走素材链，不再先传 ChatGLM 图床（双上传消除），图床仅作素材链失败后的兜底，两条链不并存；`qqbot-media://` 在后续图片解析中原样透传，且仅适用于 Markdown/keyboard（卡片 pic_url、`/uploadpic` 等必须公网 URL 的路径维持现状）；失败按「素材链 → 直链解析 → 外链上传」降级，不阻断发送
- **配额**：单机器人 100 张，平时不删（素材引用滑动 90 天可复用）；仅当 prepare/complete 返回 40035007 时清理一次——`GET /v2/media-images` 拉全量列表，按 expires_at 升序删 20 张（最久未被引用的先删：图片为滑动过期，每次被消息成功引用后顺延至 90 天，刚发送过的图自然沉底不被优先删；官方说明删除只影响后续新引用、已发送消息不受影响），再重试一次；仍失败才降级外链。原实现按 created_at 最旧 + 跳过 24 小时内新建，100 张全部为 24 小时内新建时一张都删不掉、被迫降级外链导致图片不显示（2026-11-07 修复，移除时间保护窗；同日加固：并列按 created_at 升序、expires_at 缺失/为 0 视为「尚未被引用」排在最后、40035008 视为配额已释放并计入重试条件）。同日 #57：清理删除由串行改为 5 路有界并发执行（`mediaQuotaPurgeConcurrency`＝5，`sync.WaitGroup`＋信号量＋互斥计数，stdlib 零新依赖），候选集仍先按 expires_at 升序取最旧 20 张再并发派发、尊重 ctx 取消，40035008 计入重试与 `deleted==0` 错误文案语义逐字不变，20 张串行删除约 6 秒的发送尖峰压至约 1.2 秒。同日 DTO 修复：官方列表数值字段（total/created_at/expires_at）实发为字符串形态（线上日志 `json: cannot unmarshal string into Go struct field MediaImageList.total of type int`），导致拉列表失败、清理永不重试、白白降级外链；fork 的 `botgo/dto/media_image.go` 对 `MediaImage`/`MediaImageList`/`MediaImageCompleteResponse` 增加宽容解析（shadow struct + `flexInt64` 同收 number 与数字串、null/缺省回零值、其余非法形态仍报错），字段 Go 类型与调用点零改动，序列化仍输出 number（Issue #56）
- **SDK**：`botgo` 增加素材管理 4 方法（`dto/media_image.go`、`openapi/v2/media_image.go`、v1 桩、`iface` 声明）；错误码 40035001~40035010 中文映射并保留 trace_id
- **影响**：Markdown 图片不再依赖任何外部 URL，平台侧直接引用素材；Refactor 分支此前已实现同一链路，本次为 main/hotfix 补齐
- **测试**：`handlers/media_asset_test.go` 覆盖 PUT body 一致性、ETag 缺失回退本地 md5、ETag 覆盖、PUT 非 2xx、>10MB 跳过；Markdown/keyboard 端到端断言产出 `qqbot-media://`

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

### 依赖安全升级（CVE 排查，2026-10-22）

依据 govulncheck 确认的 29 条可达告警，升级受影响的可达依赖与 Go 工具链；不改动业务逻辑，无调用点适配。

- **修改**：`golang.org/x/net` v0.25.0 → v0.60.0（GO-2026-6617 / 6612 / 6611 / 6610 / 6603 / 5026 / 4918 / 3595）
- **修改**：`google.golang.org/grpc` v1.65.0 → v1.84.0（GO-2026-6443 / 6348 / 6061 / 4762）；GO-2026-6443 的修复线经复扫确认为 v1.83.2，故取 v1.84.0
- **修改**：`github.com/gorilla/websocket` v1.4.2 → v1.5.3（GO-2026-6278）
- **修改**：`golang.org/x/text` v0.15.0 → v0.42.0（GO-2026-5970）；`x/net@v0.60.0` 要求 `x/text >= v0.42.0`，故取 v0.42.0（高于修复版 v0.39.0）
- **修改**：`go.mod` 的 `go` 指令 1.25.0 → 1.26.10，标准库部分告警由工具链补丁修复（GO-2026-6809 / CVE-2026-111943，x509 中间证书信任）
- **修改**：`.github/workflows/cross_compile.yml` 的 `node-version` 24 → 24.22.1（CVE-2026-111497）
- **影响**：`go build ./...`、`go build -tags=small ./...`、`go vet ./...`、`go test ./handlers/` 全部通过
- **说明**：最小版本选择连带抬高的间接依赖为 `golang.org/x/sys` v0.48.0、`golang.org/x/crypto` v0.57.0、`google.golang.org/protobuf` v1.36.11、`google.golang.org/genproto/googleapis/rpc`（2026-07-06 版本），均为上层模块的强制下界

---

## ✅ 提交记录

```
eca1843  fix: 官方口径再对齐（token 域名 / 5M 直传阈值 / dto.User 字段）
8f3cfdb  docs: 修复 3 处失效相对链接（语法参考 cq-set_group / API介绍 本版新增功能 / 入群申请列表 readme）
c1c6470  feat: 对齐官方 v2 现文档（群管理契约 / 入站事件字段 / 5MB 直传 / 错误码）
bd73b1f  feat: 生产主链切换到 internal/+adapter 分层架构，新增 arch_mode 三态开关
1e2691e  fix: 私聊 markdown 内嵌 keyboard 补齐 permission.type=0→2 兜底
b4519d3  docs: readme 顶部小图标的引用仓库改为本仓库
d719aef  docs: AGENTS.md 补充发版与 CHANGELOG 惯例及 Windows 提交编码提示
```

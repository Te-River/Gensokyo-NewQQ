# Changelog — Release 016 Rust

> **Rust 重构分支的首个 Release，发布名（GitHub Release 标题）为 `Release 016 Rust`**（tag 建议 `Release016Rust`）。
> 本文件随 `Refactor` 分支维护；Go 主线仍是 `Release016`。
>
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

### 素材管理三接口（官方 2026-10-21）

新增 `upload_media_image` / `get_media_image_list` / `delete_media_image` 三个扩展 action，链路为 `prepare` → 预签名 PUT → `complete`：

- **新增**：`handlers/media_images.go` 注册三 action；`botgo/openapi/v2/media_image.go` + `botgo/dto/media_image.go` 提供 v2 端点与 DTO；Rust 新链同步（`crates/gensokyo-qq/src/media_image.rs`、`crates/gensokyo-actions/src/media_image.rs`）
- **入参**：`upload_media_image` 的 `file` 支持本地路径 / `file://` / `base64://` / `data:`（不支持 `http(s)` 链接）；`file_name` 与 `content_type` 可选，扩展名与 `content_type` 必须同族一致
- **返回**：`image_id`、`url`（`qqbot-media://image/v1/<token>`）、`ttl`（`7776000`）、`preview_url`、`expires_at`；列表按 `cursor` 分页带 `next_cursor`/`total`；删除按 `image_id` 释放配额（已发送消息不受影响）
- **限制**：单图 ≤10MB（十进制 10,000,000 字节）、png/jpeg/gif/webp、单机器人 100 张、自最后引用起 90 天滑动过期；官方素材错误码（`40035001`~`40035010` 中已覆盖的 8 个）转中文说明
- **透传**：`qqbot-media://` 在 Markdown 图片与 keyboard 按钮图片字段中**原样透传**（不下载、不转存，`handlers/message_parser.go`）

### 群聊 @全员（官方 2026-10-26）

`[CQ:at,qq=all]`（宽松别名 `qq=everyone`）与字面 `<qqbot-at-everyone />` 归一为官方标签，群聊文本与 Markdown 均支持：

- **新增**：`handlers/at_everyone.go` 负责归一与「每群每自然日 1 次」本地限流；Rust 新链同步（`crates/gensokyo-runtime/src/at_everyone.rs`）
- **配置**：新增 `enable_at_everyone_limit`（默认 `true`）；关闭后既不计数也不拦截
- **行为**：先占当天名额再发送、发送失败自动回滚；同日第 2 次与单聊带 @全员 均回 `1400`；markdown 优先时标签迁移进 `markdown.content`
- **时区**：自然日固定按 `+08:00`（北京时间）切分，Go / Rust 两侧一致，不受主机时区影响

### 图文卡片支持单聊（官方 2026-10-28）

`[CQ:card]` 由仅群聊扩展为群聊 / 单聊同构（`msg_type=8` + `card{type:"tuwen",content{...}}`）：

- **新增**：`handlers/card_payload.go` 统一构造与校验；`handlers/send_private_msg.go` 接入卡片发送
- **字段**：四字段必填——`title` 1~50 字符、`description` 1~200 字符、`url` / `pic_url` 必须 `https://` 且 ≤1024 字节；缺任一项即回 `1400`
- **失败**：任一字段非法、`title` 缺失或 JSON 解析失败时整条消息回 `1400`

### CQ 码空白容错全量适配（2026-11-03）

Go 与 Rust 两侧的 CQ 码解析统一容忍参数空白，识别宽松书写形态，与既有规范形态同行为：

- **覆盖**：全部 CQ 段类型——Go legacy（`handlers/cqcode.go`、`message_parser.go`、`cqcode_pipeline.go`、`at_everyone.go`、`avatar.go`、`send_group_msg_raw.go`）、新链（`internal/domain/message`、`handlers/cqparse`）、Rust 协议层（`crates/gensokyo-protocol/src/cqcode.rs`）
- **口径**：段名后、参数间、`=` 两侧允许任意空白；键 trim；值仅首尾 trim、内部空白保留（`[CQ:record,file = my voice.amr]` 的值为 `my voice.amr`）；`qq` 允许出现在任意参数位（如 `[CQ:at,name=x,qq=123]`）
- **保真**：Rust 解析后 Segment 保留原文 raw 切片，透传臂逐字节输出原样（空白形态不被静默规范化）；解析采用与 Go 同款花括号配平，含逗号的 JSON 参数（`[CQ:keyboard,data={…,2}]`）不截断，配平失败不吞后文合法码
- **空白集**：词法切分层统一 ASCII `{空格,\t,\n,\f,\r}`（同 Go 正则 `\s`；`\v`/全角空格不算段名空白）；键值 trim 双侧统一 Unicode `TrimSpace` 语义
- **不变项**：CQ 转义口径（出站仅转 `,` `]` `&`、入站认 `&#91;`）、段名/键大小写现状（at 与 markdown 路径不敏感，其余区分）、stream 冒号语法、Go 媒体正则位置锚定等既有差异均保持
- **测试**：`go build/vet`、`go test ./...` 全绿（28 包含测）；`cargo test --workspace` 33 suites 零失败、`cargo clippy -D warnings` 零告警；双侧镜像契约用例表钉死行为
- **注**：本条「不变项」中的大小写/键序/stream 单语法口径已被下文 2026-11-04「全路径最大兼容统一」取代喵~

### CQ 码解析全路径最大兼容统一（2026-11-04）

CQ 码识别层在**全部解析路径**（Go legacy 正则群、Go `cqparse` 新解析器、`internal/` 领域新链、Rust 协议层）统一为最大兼容并集口径，此前仅部分路径支持的宽松写法现四路同行为：

- **大小写**：段名与参数键不敏感（`[CQ:Image,File=x]` 按 `image`/`file` 消费）；`[cq:` 前缀本身仍区分大小写，小写 `[cq:` 继续整码透传喵~
- **任意键序**：`file`/`data`/`id`/`qq`/`msg_idx` 等目标键不再限定首位（`[CQ:image,name=x,file=http://y]` 正常识别），且**尾部附加键不再污染值**——修复了整码化改造中断遗留的坏 URL 直发与 keyboard base64 解码失败静默丢按钮问题
- **stream 双语法**：`[CQ:stream,type:start,qq:1]`（冒号）与 `[CQ:stream,type=start,qq=1]`（等号）及混写均可识别；冒号取值语法仅对 stream 段生效，其余段的 `key:value` 形态不误当参数（Rust 同步收紧）
- **段名前空白**：`[CQ: at,qq=all]`、`[CQ: image,file=x]` 正常识别；markdown-at 的 `errMarkdownAtMalformed` 拒绝分支随之删除，不再对可渲染形态返回 1400
- **段名字符集**：四路统一 `[A-Za-z0-9_]`（`[CQ:3d,x=1]`、`[CQ:h2o,x=1]` 各路径同等按未知段处理）；含 `!`/`(`/全角空格等非法字符的段名整码按文本透传
- **同口径覆盖家族**：music / reply / input_notify / active / wakeup / ark / card / 群管理动作（`[CQ:Set_Group , ACTION = ...]`）；card 的 JSON 四字段键名统一小写落地，`[CQ:card,Title=…]` 不再因键名不符回 1400
- **前缀边界**：`[CQ:CARDIGAN,x=1]` 一类以 `card`/`ark` 起头的未知码不再被前缀匹配吞掉（整词段名判定），字面文本不再丢失
- **Rust 链**：CQ 字符串解析期参数键归一小写（`subType`→`subtype`，再序列化输出小写键；数组/JSON 入站形态键名保持原样）；at 识别层支持 `[CQ:` 后空白（消除 `[CQ: at,qq=all]` @全员静默丢）；裸尾段并入前一键值（`[CQ:reply,id=1,2]` 与 Go `cqUnionParams` 同口径）；透传臂 raw 逐字节保真机制不变
- **Go 新链补齐**：`internal/domain/message` 段尾查找改花括号配平状态机（`[CQ:markdown,data={"a":1]` 等畸形整码按文本，不再截断产段），case 表补 `stream`；`acnode` 超长 base64 图片豁免谓词适配宽松写法
- **语义明确**：`[CQ:reply,id=…,msg_idx=…]` 两类键同现时整码按 `reply` 消费、`msg_idx` 随段移除（不保证双写），已有用例钉桩；同一条码建议避免同现
- **不变项**：CQ 转义口径（出站仅转 `,` `]` `&`、入站认 `&#91;`）、golden/回归样本、值内部空格保留、未闭合与配平失败按文本透传、legacy 键值不做实体解码（`cq_parse_mode=new` 才反转义，属模式设计本意）
- **测试**：Go 25 包含测全绿（build/vet/small 全 0）；`cargo test --workspace` 33 suites / 719 passed / 0 failed、clippy 零告警；镜像契约矩阵 `handlers/cq_union_contract_test.go`、三模式矩阵、internal 状态机用例、Rust 形态表

### 出站 URL 转换与二维码管线（Rust 侧 P56-T1~T4，2026-11-13）

Rust 版接齐出站 URL 管线（对齐 Go `transformMessageTextUrl` + `generateQRCode` + `CallAPIFromDict` 侧发送）：出站文本按 xurls Relaxed 口径逐 URL 扫描，按 IP 隐藏 → QR 图 → 短链（lotus 远程 / 本地落库）互斥优先级原位替换；每个 QR URL 生成二维码图片构造单图子请求先行发送、响应仅日志不回推客户端：

- **xurls 移植**（`crates/gensokyo-protocol/src/xurls.rs`，P56-T1）：gtld / otherScheme 常量逐字拷 xurls v1.1.0（MIT 署名注明），RE2 leftmost-longest 以 path 分支重排 + 同起点 strict/webURL 双式取最长模拟，`(?-u:\b)` 对齐 ASCII 词边界；Go 金样 harness corpus 逐 span 断言
- **QR 码**（`crates/gensokyo-media/src/qr.rs`，P56-T2）：`qr_png_base64(url, qr_size)` 容错级=Go `qrcode.High`（ISO Q 级）、无静区、Go qrcode PNG 尺寸语义（负数=每模块像素数、过小静默放大到模块数）；模块矩阵与 Go `Bitmap()` 金样一致（PNG 字节跨库不可逐字节一致，lead 裁决接受矩阵级 parity）
- **变换管线**（`crates/gensokyo-runtime/src/url_pipeline.rs`，P56-T3）：门控仅 `transfer_url`（lead 裁决：Go handler 不消费 `auto_escape`，URL 路径恒由 `transfer_url` 决定）；原位替换、周围空白不 trim；QR 开时 URL 删除且图片生成在 hook 侧做、失败也删（Go :1766-1775）；`UrlMint::Local` 短码落 `short-urls.redb` 替换 `https://{server_dir}/url/{短码}`，mint 失败**照抄 Go 怪癖仍拼悬空 `https://{server_dir}/url/`**（`message_parser.go:1784-1789`，同 HTTP 无 token 缺陷口径，修复另立任务）；`LotusMint` POST 远端 `/url`（form `url` + `token=hex(HMAC-SHA256(lotus_password, 长链))`、30s 超时、非 200/缺 shortURL → 空串、成功返回远端完整 shortURL 原位替换）；`mint: Option<&UrlMint>`——`None`（短链库未装配）为 Rust-only 防御路径：**保留 URL 原文不改写**，整次 `transform` 最多一条 `warn`（带 `count`、不含全 URL；Go idmap/db 恒存在，无对应态）；lotus mint 五个失败分支逐分支 `warn`（只带 `error`/`status`/`request_url`，**绝不**含 token/密码），返回值仍空串（Go 口径）；本地 mint 失败仍是悬空 base（同上）
- **hook 与装配**（`crates/gensokyo-runtime/src/deferred.rs` / `boot.rs` / `state.rs` / `dispatcher.rs`，P56-T4）：`execute_inner` compose 前插 URL 变换阶段；每个 QR URL 生成 PNG 后构造 `[CQ:image,file=base64://…]` 单图子 SendRequest **递归重入执行链先行发送**（发送序 QR 图条在前、主消息在后；QR 条 `msg_type=7` 无 reply；子请求独立解析被动身份；返回值仅 `tracing::info` 日志、不回推 OneBot 客户端——客户端只收到原 action 一份响应，Go 同）；`depth` 参数防递归（QR 子请求 depth=1 跳过变换）；`execute` / `execute_forward` 自动覆盖；三类出站路径不挂、均与 Go 同：`execute_sse`（Go SSE 同样不过 parseMessageContent）、SSM 补发（`deferred.rs:560-563` 直发存量 DTO ↔ Go `send_group_msg.go:2676` `PostGroupMessage(pair.GroupMessage)`）、WS 掉线兜底（`downtime.rs:136/140` ↔ Go `Processor.go:379` `GetDowntimeMessage`）（登记）；boot `attach_url_mint` 按 `lotus` 选变体经 `AppState`→`Dispatcher`→`DeferredDeps` 传入
- **行为边界与升级提示（M-A，重要）**：`transfer_url=false` 时代码层默认值零改写（集成回归锁用例钉桩）。但 `templates/config.yml` 模板的 `transfer_url` 默认值是 **true**，而本仓 Rust 侧此前不消费该键——**沿用模板配置升级的实例，本版起出站文本 URL 会开始被改写**（`server_dir` 为域名→替换成 `https://{server_dir}/url/{短码}`；为 IP 且 `visible_ip:false`→删除），这在对齐 Go 的意义上是正确的（Go 默认亦改写），但不想被改写的实例需**显式设 `transfer_url: false`**（即时回退、无编译期嵌入）；保持 true 则 `server_dir` 须为已在 q.qq.com 后台报备 `/url`、`/image` 校验的域名。QR 侧发送另受 `url_to_qrimage`（默认 false）控制，属 opt-in
- **与 Go 差异（登记）**：mint 拼接参数（`server_dir`/`port`/`force_ssl`/`lotus_password`）逐调用现读 live 快照（Go `GenerateShortURL` 每次 `config.GetXxx()` 同口径），boot 只固化变体选择——`server_dir`/`lotus`/`lotus_password` 属重启项、改后快照整体冻结（`manager.rs:114-122`），残留差异只剩「Go 重启项变更→自动重启（`config.go:166-173` `sys.RestartApplication()`）vs Rust 手动重启」这一条 config 层既有口径；QR PNG 字节级跨库差异（矩阵级 parity）；PNG 生成失败/成功日志口径同 Go（`tracing` 替代 `mylog`）；`serde_json::from_str` 不容忍 JSON 尾随垃圾 vs Go `json.Decoder.Decode` 容忍（仅畸形响应有别）；长混合字母数字内容两库可能选不同 QR 版本（module 数不同、解码载荷相同、rqrr 已验）；lotus mint 最坏延迟 = 30s × URL 条数且未复用 `boot::shared_transport()`（本轮不做，后续项）；`transfer_url`+`url_to_qrimage` 同开时 QR 条与正文条各推一条 wakeup notice（对齐 Go 每出口 `sendWakeupNotice`，被动额度占 2）且存在「图已发、正文失败」残缺窗口（运维知悉）；**M-B 已闭环（P56-F3）**：`LotusMint::try_new` 在构造 client 前预检 `CryptoProvider::get_default()`，未安装 provider 返回 `Err`（`idmap/http_remote.rs` 同款口径，reqwest 0.13.5 无 provider 时 `build()` 直接 panic，`Result` 单独兜不住）并由 `attach_url_mint` 记 `error!` 不注入 mint；**不注入的后果**是短链分支保留 URL 原文（M-C 语义），不崩进程；runtime 的 `rustls` 依赖从 dev 升入 `[dependencies]`（workspace 定义无 provider，惯例不破）
- **测试**：`url_pipeline` 单测 17（T3 的 15 + T4 悬空 base/None-mint 修正新增 2）+ runtime 集成 9（既有 7：`transfer_url=false` 回归锁、QR 发送序/`msg_type=7` 无 reply/单份响应/精确文本、IP 隐藏优先、本地短链落库、mint 未装配保留原文、lotus stub 原位替换、forward 逐 node 覆盖；P56-F2 增 2：管线 mint 短码→运行时 `GET /url/{code}` 301 闭环（与生产同款共用 `Arc<ShortUrlStore>`）、QR 生成失败仍删 URL）+ boot `attach_url_mint` 装配用例 4（P56-F3 落地 `try_new` provider 预检后全绿）；合并树 `cargo test --workspace` 43 suites / 1356 passed / 0 failed

### `get_friend_list` 读侧对齐（Rust 侧 P64a，2026-11-14）

Go 的 `get_friend_list` 不是上游接口，是本地 idmap 活实现（枚举 `UserInfo` 桶，`handlers/get_friend_list.go:29-47`），Rust 侧此前与「官方无端点」action 并列回 `1500`；本版补齐读侧与其对齐（写侧 `StoreUserInfo` 另立 P64b）：

- **修改**：`crates/gensokyo-storage/src/idmap/mod.rs` 新增只读 `IdMapStore::list_all_users()` 与 `UserInfoRow`（Go `ListAllUsers` 口径：按 key 字节序升序、无去重、value 三字段 JSON 解码、损坏行整次 Err、表不存在回空）；`crates/gensokyo-actions/src/users.rs` 的 `GetFriendListHandler` 从 `context.idmap()` 直读，仅滤空 `user_id`（Go `get_friend_list.go:36-40`），条目按 Go 声明序 `nickname`、`remark`、`user_id` 装配；死代码清账——`QqApi::get_friend_list` 占位 trait 方法、`FriendEntry` 及再导出、`testing.rs` 的 `Call::FriendList` / `friends` 槽零消费，全部移除
- **影响**：空表 / 未装配 idmap / 枚举出错均照 Go 恒 `retcode=0 status="ok"` + `"data":null`（不是 `[]`），错误只记 warn 日志；`user_id` 输出表内原串（JSON 字符串），不做虚拟号转换；**写侧已由 P64b 补齐**（见下条）——Go 在入站 C2C 时写 `UserInfo` 桶（`ProcessC2CMessage.go:231-241`），Rust 接上同一条写入后，纯 Rust 部署**收过私信就会在运行期自然积累** `user_info` 表；`"data":null` 只剩「既没跑过 M1e 导入、也还没收到过私信」的初始状态，不再是长期结论
- **测试**：storage 新增 3 例（key 字节序与内容 / 含空 user_id 与缺字段未知字段原样返回 / 损坏行整次枚举 Err）+ actions 新增 4 例（非空列表外部 JSON 字节断言、空表 `"data":null`、未装配 retcode=0、非对象 params 1400）；`cargo test -p gensokyo-storage -p gensokyo-actions` 141 + 169 passed / 0 failed，两包 clippy `--all-targets -- -D warnings` 零告警，`cargo fmt --all -- --check` 绿

### `get_friend_list` 写侧接线（Rust 侧 P64b，2026-11-14）

Go 在每条 C2C 私信末尾把好友写进 idmap 的 `UserInfo` 桶（`Processor/ProcessC2CMessage.go:231-241` → `idmap.StoreUserInfo`，实现 `idmap/service.go:2267-2287`）；Rust 侧此前只有读侧（P64a），纯 Rust 部署的 `user_info` 表只有 M1e 导入器数据，**写侧缺失期间收过私信的好友也不可见**（该接口只回 `"data":null`）。本版补上写侧：

- **修改**：`crates/gensokyo-storage/src/idmap/mod.rs` 新增 `IdMapStore::store_user_info(raw_id, row)`——键 `"<rawID>:<user_id>"` 与 value 三字段紧凑 JSON 逐字对齐 Go `idmap/service.go:2270-2276`（字段序 `nickname`/`remark`/`user_id`，均无 omitempty ⇒ 与 Go `json.Marshal` 字节一致），复用读侧同一个 `UserInfoRow`、**不设第二套编码器**；自开单写事务（与邻近 `write_config` 同风格），**不得在 `with_write_batch` 闭包内调用**（redb 每库单写者，`idmap/batch.rs:3-5` 已注明）；**重复键报错而不覆盖**，承载方式是 `error.rs` 新增独立变体 `StorageError::DuplicateKey(String)`（载荷为**完整 key**，`Display` 与 Go `service.go:2272` 的 `duplicate key: <key>` 逐字节一致）——评审收口把原先借用的 `StorageError::Corrupt` 改开：重复键是稳态不代表库损坏，借 `Corrupt` 会让调用方把真损坏一起吞掉；`AlreadyExists` 装的是 vUIN（重绑定目标被占），也不改借它。`idmap/grpc.rs` 的 `status()` 同步补 `DuplicateKey => already_exists` 臂（全仓唯一不带通配的 `StorageError` 穷举点；该 enum 为 `#[non_exhaustive]`，跨 crate 匹配本就须带通配，无第二补臂点）；与 Go 一致只写本地库，`remote`（lotus）挂接时不改走远端
- **接线**：`crates/gensokyo-runtime/src/inbound.rs` 的 C2C 臂（`record_user_info`，`:756`；调用点 `:1106`）——raw 段取事件 OpenID（C2C 不做 union 切换），`user_id` 取上报形态（数字模式十进制虚拟号 / `string_ob11` 下原始 OpenID），昵称·备注照 Go 恒空串；写失败不阻断事件（Go 忽略返回值）；日志按错误类型分级（评审收口修正）——`DuplicateKey` 压 debug、其余失败含真·行编码损坏的 `Corrupt` 走 warn，此前一个分支同时吞两类，等于把真损坏藏进 debug；**空 OpenID 守卫**：`raw_user` 是空串就直接返回不写库（Go `ProcessC2CMessage.go:30-33` 对 `Author` 为空是整条事件早退，Rust 原先照写不误、会产出键为 `":"` 的垃圾身份行），只判空串、不 trim ⇒ 与 Go 的两个子条件等价
- **影响**：纯 Rust 部署收过私信的好友**立即可被 `get_friend_list` 读到**，重启后仍在（落身份库）；该表自此**在运行期自然积累**，出站 JSON 零变化。差异登记两处：① idmap 身份批写失败时本仓回退原始串（key 形如 `"<raw>:<raw>"`），Go 照旧用 `userid64=0`——P35 既有 ID 回退口径，正常路径无差异；② C2C `Author` 为空时 Go 是**整条事件不上报**（`ProcessC2CMessage.go:30-33` 直接 `return nil`），Rust 只取「不写库」这一半、上报字节不变——改上报形态会动外部 JSON，刻意保留，已登记 `docs/rust-handover.md` §6
- **测试**：storage 3 例（`store_user_info_writes_go_key_and_value_layout` 键与 value 字节形态 / `store_user_info_rejects_duplicate_without_overwrite` 断言变体是 `DuplicateKey` 且载荷为完整 key、`Display` 与 Go 文案**全串比对**、原值不被覆盖、空 `user_id` 照写 / `store_user_info_survives_reopen` 关库重开仍在）+ runtime 2 例（`c2c_message_stores_user_info` 事件后 `list_all_users` 可读、重复私信不影响上报 / `c2c_message_without_author_id_writes_no_user_info` 覆盖 `author.id=""` 与 `author` 字段缺失**两种空法**）；包级 clippy `--all-targets -- -D warnings` 与 `cargo fmt --all -- --check` 在门禁内，用例数与产物体积**不预填**，以最终门禁（`cargo test --workspace`）输出为准

### 出站管理 CQ 码真执行（Rust 侧 P83，2026-11-17）

`[CQ:member]` / `[CQ:set_group]` / `[CQ:remove]` 此前在 Rust 主线被当作「不可消费码」：要么整条 `1400`，要么留字面发进群。本版与 Go `ProcessOutboundCQCodes`（`handlers/cqcode.go:226-244`）对齐，在发送前真正执行管理动作（成员身份、禁言 / 踢人 / 黑名单 / 入群审批、撤回）：

- **修改**：`crates/gensokyo-protocol/src/outbound.rs` 把三码解析成类型化 `OutboundPlan::admin_ops`（L1 缺陷留字面、`[CQ:member]` 缺陷按 Go 吞码；数值非法以 `raw_*` 带到运行期），新增 `OutboundPlan::from_wire_segments` 让序列化往返后仍能识别 `remove` 的字符串路径；`segment.rs` 的同名参数改为**后者覆盖**（Go `cqParseParams`）。新增 `crates/gensokyo-runtime/src/cq_admin.rs`，由 `deferred::execute_inner` 在**判空与发送之前**执行（`depth==0` 守门，URL 变换的 QR 子请求不双执行；`member` 的 `event_id` 覆盖沿用既有被动身份流程）。私聊 `[CQ:remove]` 走 C2C 撤回（`RetractRequest.user_openid`），撤回目标恒为发送目标、**不读 `group_id`**；`whole_ban` 官方无写入字段，记 WARN 后只跳过该码；新增配置 `cq_admin_actions`（`legacy|off`，默认 `legacy`），`off` 为逃生阀：消息含任何管理码即整条 `1400`、零上游调用
- **影响**：**动作在发送前执行、失败不回滚已执行的动作**——多码按到达顺序串行，任一条 L2 失败 / 上游失败时以第一个错误作为整条结果（`1400`/`1500`），**正文整条丢弃**（`crates/gensokyo-runtime/src/deferred.rs:303` 的 `?`）；这与 Go「吞码但正文照发」不同，属刻意差异。**默认 `legacy` 下正文里夹带的管理码会被真执行**：转发、复读或未转义的用户输入只要含 `[CQ:set_group,action=kick,…]` / `[CQ:remove,…]` 就会真踢人 / 禁言 / 撤回——插件须对不可信文本转义，或把 `cq_admin_actions` 设为 `off`（整条拒绝）。其余登记差异：私聊 member/set_group 留字面（Go 私聊不消费）；反查失败 / 上游失败大声拒绝（Go 仅日志）
- **测试**：`crates/gensokyo-runtime/tests/cq_admin.rs` 7 例（入群申请 reason 反转义、上游失败 `1500`、多码保序且中间失败不中断、私聊 `set_group` 留字面、私聊 `remove` 走 C2C、`member add/remove` 身份覆盖）+ 协议单测（三码解析与 L1 留字面 / 吞码、wire 往返、同名参数后者覆盖）；门禁以最终 `cargo test --workspace` 输出为准

### 自定义数据库位置（Rust 侧移植 Go `dblocation/`，2026-11-18）

对齐 Go `dblocation` 包与 `main.go:167-169` 的挂接：idmap 族数据库可在启动时迁到自定义目录，开关关闭时行为零变化。Go 侧第一阶段注册表是 5 个 bbolt 文件（`idmap.db` / `idmap-identity.db` / `idmap-msg.db` / `openid-map.db` / `msgid-map.db`），Rust 侧等价物是 **2 个 redb 文件**：`openid-map.redb`（身份库）/ `msgid-map.redb`（消息库）；botstats / 短链 / union 缓存不进注册表。

- **配置**：新增 `custom_db_enabled`（默认 `false`）/ `custom_db_path`（默认 `""`，空串即未启用），访问器 `get_custom_db_enabled` / `get_custom_db_path`，模板 `templates/config.yml` 照 Go 模板措辞补两键；两项均登记为重启项（`RESTART_REQUIRED_TAGS` 34 → 36，Go `restartRequiredFields` 同步口径）
- **修改**：新增 `crates/gensokyo-storage/src/db_location.rs`——注册表 `KNOWN_DB_FILES`；`normalize`（相对→工作目录、绝对/UNC 直用、含 `://` 拒绝并预留协议客户端扩展缝）；`probe`（建目录 + 写探针文件 + 一次性 redb 库的锁探针，3 次退避重试，成功即删）；`migrate` 四裁决（目标与本地同在→冲突拒绝启动；目标有→直接用；本地有→复制 + 大小校验 + 只读打开校验 → 本地改名 `.bak.<YYYYMMDD-HHMMSS>`；两边无→全新）；`setup` 进程级一次性、`resolve_file` 作为 idmap 族唯一取路径入口
- **挂接点**：`gensokyo_runtime::Runtime::start_with` 在 `attach_echo` / `attach_idmap` 之前调用（library 消费者兜底），`gensokyo-bin` 的 `serve` 在 `logging::init` 之后、启动期自动导入 `auto_import_idmap`（最早打开库的一步）之前调用；两处失败都按 Go `Fatalf` 口径**拒绝启动**，绝不静默回退本地。idmap 打开参数（`boot.rs` / `startup.rs` 两份 `idmap_options`）改经 `resolve_file` 取路径
- **影响**：启用时 WARN 网络文件系统（NFS/SMB/挂载盘）锁语义无官方保证、按单实例自担风险（Go 同口径，引用 etcd-io/bbolt issue #257）；`custom_db_path` 与工作目录相同视为未启用（与 Go 一致）
- **测试**：`db_location` 单测 12 例（normalize 表驱动含空路径 / `smb://`·`http://`·`https://` 拒绝、探测可写目录 / 可创建子目录 / 目标是文件 / 探针库被锁、四裁决表、校验失败保留本地、工作目录同路径视为未启用、备份名 `YYYYMMDD-HHMMSS`）+ 集成 1 例（开关关闭零变化；开关打开迁移后按 `resolve_file` 打开 idmap 写入读回）；配置侧 fixture/重启项断言同步更新。门禁以最终 `cargo test --workspace` 输出为准

### `get_group_member_list` 内部翻页拉全量（Rust 侧，2026-11-18）

对齐 Go `handlers/get_group_member_list.go:96-154` 的 `fetchMembersByAPI`：Rust 新链的成员列表不再只回一页，改在 handler 内部按游标链拉到末页，响应形态保持 Go `buildResponse` 口径。

- **修改**：`crates/gensokyo-actions/src/group.rs` 的 handler 由「单页透传」改为游标循环——入参 `cursor` 只当**起始游标**（缺省 / 空串即首页），逐页按 `next_cursor` 推进并合并；终止条件为末页（`next_cursor` 空串）/ 页数上限 `MAX_MEMBER_PAGES = 100`（对齐 Go `groupMemberListMaxPages`）/ **游标原地重复**（新增死循环防护，Go 只用页数上限兜底，代码内注释标明这是刻意增强）
- **节流**：页间按 `get_g_list_delay` 等待（与 `get_group_list` 共用 Go `config.GetGroupListDelay()` 同一配置项），每请求读一次配置快照、热重载即时生效；`<= 0` 不等待，末页后不等待
- **错误路径对齐 Go**：中途页失败返回**已收集成员 + `retcode 0`**（不整表丢弃、不再报 `1500`）；首页失败或全量为空回退 idmap `find_sub_keys_by_id_pro` 本地缓存，条目与 Go `fetchFallbackMembers` 同款（`nickname`/`card="未知"`、`sex="0"`、`level`/`title="0"`、`role="member"`、`join_time`=昨天）；无 idmap 或反查失败时回空数组，仍是 `retcode 0`
- **响应**：仍只有 `data` / `echo` / `message` / `retcode` / `status`，**不含游标**（Go `buildResponse` 没有 `next_cursor`）
- **影响**：一次 `get_group_member_list` 返回自 `cursor` 起的剩余全量（默认单页 30、至多 100 页），大群首次调用耗时随成员数上升，页与页之间有 `get_g_list_delay` 间隔
- **测试**：`crates/gensokyo-actions/src/group.rs` 的替身改为多页脚本（`VecDeque<Result<..>>`），用例覆盖两页游标链与末页即停、入参游标作起点、中途页失败保留首页成员、首页失败回退空列表、100 页上限截断、游标不推进即停、`delay = 0` 不等待 / 正延迟只在页间等一次；并持续断言响应不含 `next_cursor`

### 入站 `sender` 昵称/群名片接入 `member_nickname`（官方 2026-11-16 字段，2026-11-20）

QQ 官方 2026-11-16 起群消息事件的 `author` 新增 `member_nickname`（群内昵称/群名片），单聊事件也带发送者昵称；Rust 侧 `QqAuthor` 没有该字段，解析时被 serde 直接丢弃——群聊 `sender.nickname` 只认 `username` 且从不设置 `card`，私聊则恒为空串。与 Go 侧提交 `e6c7e32` 同步口径：

- **修改**：`crates/gensokyo-runtime/src/inbound.rs` 的 `QqAuthor` 新增 `member_nickname`（`#[serde(default)]`，群消息与 C2C 共用同一结构）；新增纯函数 `resolve_nickname_and_card(author, card_and_nick)` 统一取值——群消息调用点传入 `settings.get_card_and_nick()` 并同时设置 `sender.nickname` / `sender.card`，私聊调用点传 `""` 只取昵称、`card` 仍不设置
- **优先级**：群聊 `nickname` 与 `card` 均为 `card_and_nick` 配置（非空，同时覆盖两者）> `member_nickname` > `username`（`username` 只写 `nickname`，不写 `card`；空串即不上报 `card`）；私聊为 `member_nickname > username`，不套用 `card_nick` 配置
- **影响**：配置了 `card_nick` 的部署行为不变；未配置时群内 `sender.nickname` / `sender.card` 改用群名片，私聊 `sender.nickname` 由恒定空串变为发送者昵称
- **测试**：`resolve_nickname_and_card_matches_go_precedence`（7 例表驱动：配置覆盖两者 / 群名片优先于 `username` / 空群名片回落 `username` 且不设 `card` / 两者皆空 / 私聊两例 / 无 `author`）+ 事件级 `group_message_maps_member_nickname_to_sender`、`c2c_message_maps_member_nickname_to_sender`

---

## 🐛 修复

### `[CQ:remove]` 不带 `msg_id` 恒失败：入站补写「群 × 用户」最新消息（2026-11-20）

`[CQ:remove,group_id=…,user_id=…]` 省略 `msg_id` 时应撤回该用户在该群的最后一条消息，但 Rust 入站**从不写**「群 × 用户 最新 msg_id」，读端 `cq_admin`（`get_latest_msg_id`）永远查不到 ⇒ 恒回「没有可撤回的最新消息，请带 msg_id 重试」。Go 在 `Processor/ProcessGroupMessage.go:179` / `ProcessGroupNormalMessage.go:168` 每条群消息都落这条记录（键 `<group_openid>:<user_openid>`，`idmap/map_service.go:702`）。

- **修改**：`Inbound::derive_message_ids`（`crates/gensokyo-runtime/src/inbound.rs`）在既有的单个 idmap 写事务里补 `batch.store_latest_msg_id(group, user, msg_id)`——取**原始** OpenID（union 切换后的 `author.id`，与 Go 传参同源），排在群注册之后以维持批次内「先身份库后消息库」的锁序；`GROUP_AT_MESSAGE_CREATE` / `GROUP_MESSAGE_CREATE` 与 `string_ob11` 开/关两形态共用同一条路径；C2C 不写（无群撤回语义，与 Go 一致）
- **影响**：不带 `msg_id` 的群撤回从恒失败变为真实调用 `/v2/groups/{group_openid}/messages/{message_id}/recall`；记录有效期沿用 `msg_id_ttl`（与 Go 的 `GetMsgIDTTLSeconds` 同口径），过期后仍回原本的「没有可撤回的最新消息」提示；`outbound.rs` / `deferred.rs` 发送主链零改动
- **测试**：新增 `inbound::tests::group_message_records_latest_msg_id_for_cq_remove`（两种群事件 × `string_ob11` 两形态）与 `tests/cq_admin.rs::group_remove_without_msg_id_uses_the_latest_inbound_message`（入站落库 → `[CQ:remove]` 无 `msg_id` → 断言上游 `delete_msg` 入参）；`cargo test -p gensokyo-runtime -p gensokyo-storage -p gensokyo-actions --lib` 全绿

### 日志噪声：每条群消息 6 行 DEBUG，Go 只有 3 行 INFO（2026-11-20）

真机 `log_level: 0`（debug）下每条群消息固定多出 6 行 DEBUG——idmap 身份命中 ×2、消息 ID 已分配 ×1、
`watcher` 忽略非目标文件事件 ×3——而 Go 同一时刻每条消息只打 3 行 INFO，排障信息被噪声淹没。

- **修改**：① 热路径 `debug!` 逐条降到 `trace!`——`idmap/batch.rs`（身份命中/已分配、消息 ID 已分配/命中）、
  `config/watcher.rs` 的非目标文件事件、`gateway.rs`/`webhook.rs` 的事件已发布、`reporter.rs` 的上报完成、
  `echo/mod.rs` 的 SSM 入队、`inbound.rs` 的 `user_info` 重复键、`acnode.rs` 的超长 base64 放行；
  生命周期级 `debug!`（启动、连接、重载、注册表就绪、清理线程）全部保留
- **修改**：`inbound.rs` 补 Go 的两行 INFO（`target: "gensokyo_runtime::message"` → 控制台 `[message]`）：
  群消息 `group id mapped: raw_group=… vGroup=… raw_user=… vUser=…` + `group msg_id mapped: raw_msg=… vMsg=…`，
  C2C 对应 `c2c id mapped: …` + `c2c msg_id mapped: …`（对齐 Go `ProcessGroupNormalMessage.go:87/165`、
  `ProcessC2CMessage.go:74/96`，同样只在 `string_ob11` 关闭时打印）
- **影响**：`log_level: 0` 下每条群消息从 6 行 DEBUG + 0 行 INFO 变为 0 行 DEBUG + 2 行 INFO（Go 为 3 行，差的
  一行是 handler 侧「收到群消息」，本仓无对应打印）；逐条明细仍可用 `trace` 档位看；`logging.rs` 的格式与过滤未动
- **测试**：`cargo test -p gensokyo-config -p gensokyo-storage -p gensokyo-runtime --lib` 全绿；
  计数探针实测一条群消息 5 条 tracing 事件（2 INFO + 3 TRACE）、C2C 4 条（2 INFO + 2 TRACE），debug 档位只剩 2 行

### 数组消息段不再把未知段与 `remove` 发成字面量（2026-11-20）

插件用**数组形态** `message` 发 `{"type":"face"}` / `{"type":"remove"}` 时，Rust 把段原样拼成 `[CQ:face,id=14]` 这类字面随正文发进群——QQ 官方 api-v2 的群 / 单聊消息只认 text / markdown / ark / embed / media / keyboard，`face` 等并不在其中；Go 的段分发表同样没有这些 `case`，落 `default:` 只 `mylog.Printf("Unhandled segment type: %s")`、不追加正文（`handlers/message_parser.go:1252-1254`），数组形 `remove` 也命中该 `default`（`grep '"remove"' handlers/message_parser.go` 零命中）。

- **修改**：`OutboundPlan::from_segments`（`crates/gensokyo-protocol/src/outbound.rs`）——数组段路径（段无 `raw` 原文切片）的未知段类型改为 `warn!` + 丢弃；数组形 `remove` 不执行撤回、也不发字面量（同样 `warn!`）。**字符串 CQ 路径不变**（Go legacy 仍按 `tok.Raw` 留字面）；markdown / keyboard / card / stream 等**已知类型但载荷不合法**仍按字面保留（既有刻意选择，已登记）
- **影响**：数组段里的 `face` / `node` / `poke` / `group_info` / `json` / 未知码不再出现在群 / 单聊正文；字符串 `[CQ:remove,…]` 撤回照旧真执行；混合数组只丢未知段、已消费段照常（`hi` + `face` → `hi`）
- **测试**：新增 `array_segment_unknown_type_is_dropped`、`array_remove_segment_is_dropped`、`array_unknown_segment_between_text_is_dropped`；既有 `from_segments_keeps_reply_and_media_order` / `admin_segment_forms` 的断言按 Go `handlers/message_parser.go:1252-1254` 改为「丢弃」

### `send_msg` 缺 `message_type` 与 markdown 双层包装载荷（2026-11-20）

插件（nonebot/qbind）发 `send_msg` 时既**不带 `message_type`**，markdown 段又是 go-cqhttp 的双层包装
`{"type":"markdown","data":{"data":{"markdown":{"content":…}}}}`。Rust 先命中「缺少 message_type」的 `1400`；即使手动补上，
也会因只读顶层 `content` 再 `1400`。Go 侧则用参数形状推断目标、并按 `markdown.content` 解包，发送成功（`retcode:0`）。

- **修改**：`plan_send_msg`（`crates/gensokyo-actions/src/send.rs`）在 `message_type` 缺失/未知时按 Go `handlers/send_msg.go` 口径推断——有非空 `group_id` → 群、否则有非空 `user_id` → 私聊、两者都有 → 群，都推不出才 `1400`；markdown 载荷按 `content` → `markdown.content` → `data.markdown.content` → `data.data.markdown.content` 逐层解包（对齐 Go `parseMDData` + `unwrapNestedMarkdown`，`crates/gensokyo-runtime/src/deferred.rs`）
- **影响**：go-cqhttp 风格的数组段 markdown 不再 `1400`；缺 `message_type` 的 `send_msg` 与 Go 行为一致；WebUI/日志无变化
- **测试**：`send_msg_infers_target_without_message_type`、`send_msg_requires_message_type_or_ids`、`field_markdown_object_payload_keeps_content` 及 runtime 端到端用例；`cargo test -p gensokyo-actions` 178 / `-p gensokyo-protocol` 150 / `-p gensokyo-runtime --lib` 350 全绿

### `cargo xtask build --all` 缺工具链目标改为 `[skip]`（2026-11-18）

Windows-gnu 宿主上，矩阵中的 macOS / MSVC / Android 目标此前会先 `rustup target add`（下载 rust-std）再硬编失败（ring 的 C 代码被 mingw gcc 拒绝 `-arch`），且失败计入整体结果（exit -1）：

- **修改**：构建循环前增加工具链可用性闸门（纯函数 `target_skip_reason`，`xtask/src/build.rs`）——宿主非 macOS 跳过 darwin、找不到 `link.exe` 跳过 MSVC、无 NDK 跳过 Android；linux 目标继续走 zig 自动引导（`.tools/` 命中即零下载）
- **影响**：`--all` 输出 `N 个产物 / M 个跳过 / 0 个失败`；显式 `--target <triple>` 不受闸门影响（仍会真实尝试）
- **测试**：xtask 32 条单测全绿（含两条新增闸门判定用例）

### `--all` 的 MSVC 误判与 zig 引导失败连带失败（2026-11-18）

Windows-gnu 宿主（PATH 带 `C:\msys64\usr\bin`）上 `cargo xtask build --all` 把 MSVC 目标当成可构建目标硬编失败（ring 0.17.14 的 `error occurred in cc-rs: failed to find tool "cl.exe"`）；zig 引导失败（本机实测 4 个镜像源均不可达）时 4 个 linux 目标被记 `[fail]`，整条 `--all` 非零退出：

- **修改**：MSVC 闸门判据由 `link.exe` 改为 `cl.exe`——MSYS2 coreutils 在 `usr/bin/link.exe` 提供同名 Unix `link`（硬链接工具），按 `link.exe` 探测会误判「MSVC 可用」，而 `cl.exe` 不在 PATH 才正确反映无 MSVC（`xtask/src/build.rs` 的 `target_skip_reason`）；同时 zig/cargo-zigbuild 引导失败降级为跳过：打印一条 `[warn]`（保留完整错误链）、linux 目标全部 `[skip]`，退出码仍如实反映「真实构建失败」与「一个产物都没有」两种情形（全跳过时以非零退出并给出指引）。下载侧同步加护栏（`xtask/src/bootstrap.rs` 的 `download`）：`--retry 3` → `--retry 1`、单次 `--max-time` 由固定 900s 改为缺省 600s（`GENSOKYO_DOWNLOAD_MAX_TIME=<秒>` 可收紧，非法值回退缺省并打提示）、并加「<1 KB/s 持续 120s 即中止」的低速护栏——断网或镜像停摆时在个位分钟级落到 `[skip]`，不再干等约 45 分钟。Windows 下载另加 `--ssl-revoke-best-effort`（schannel 软吊销：CRL/OCSP 不可达时不再以 `curl: (35) CRYPT_E_NO_REVOCATION_CHECK (0x80092012)` 硬失败，仍做证书链校验、不使用 `--insecure`；openssl/gnutls 后端不加该参数）；引导失败的 `[warn]` 补 `.tools` 手工兜底指引（把工具解压到 `.tools\zig-0.16.0\` / `.tools\cargo-zigbuild-0.24.1\` 后重试即零网络）；`--ssl-revoke-best-effort` 改为按 `curl --version` 探测（**≥ 7.70 才加**、解析不了按不支持、结果进程内缓存），避免老机器（Win10 1803~1909 自带 curl 7.55.x）因未知开关 `option … is unknown` 直接硬失败
- **影响**：裸机（无 MSVC、无网络）上 9 个交叉目标全部 `[skip]`，命令以非零退出码结束并给出「没有产出任何产物」的明确指引——不再有 cc-rs 找不到 `cl.exe` 的崩溃噪音，0 产物也不被当成成功；有 MSVC 工具链的机器上两个 windows 目标照常构建并退出 0（本机无 MSVC，未实测）；引导成功后某个 zig 目标真正编译失败仍记 `[fail]`，不被吞
- **测试**：`cargo test -p xtask` 全绿（闸门两条用例改按 `cl.exe` 口径）；`cargo clippy -p xtask --all-targets -- -D warnings`、`cargo fmt --all -- --check` 零告警；`GENSOKYO_ZIG_VERSION=0.0.0-does-not-exist cargo xtask build --all` → 9 个目标全 `[skip]`（msvc 理由为 `cl.exe` 不在 PATH）、`0 个产物 / 9 个跳过 / 0 个失败`、非零退出码；`GENSOKYO_DOWNLOAD_MAX_TIME=45 cargo xtask build --linux-only`（真实 zig 0.16.0 版本号、本机 ~12 KB/s）整条 **131s** 结束（xtask 重建 + 2 次下载尝试各 45003/45008 ms，`curl: (28)` 超时），4 个 linux 目标 `[skip]`、退出码 1，`.tools/` 无残留；`GENSOKYO_DOWNLOAD_MAX_TIME=abc` 打出 `[warn] … 非法（需正整数秒），回退缺省 600` 并照常以 600s 启动下载；schannel 修复实测（Windows 10/11 自带 curl 8.22.0 Schannel）：把仓库副本置于「`.tools` 只有 `zig-0.16.0`、无 cargo-zigbuild」的用户现场状态，`cargo xtask build --no-upx --target x86_64-unknown-linux-gnu` 自动下载 cargo-zigbuild + `.sha256`（`sha256 校验通过`）并完成 zigbuild，`curl: (35)` 未再出现；随后 4 个 linux 目标全部实建成功：`gensokyo-linux-amd64` 13.4 MB、`-386` 12.8 MB、`-arm` 11.2 MB、`-arm64` 11.5 MB，逐个退出码 0

### 私聊 keyboard 按钮权限全类型归一化（`permission.type` 非 2 一律转 2）

QQ 官方 C2C 单聊只支持按钮 `permission.type=2`（所有人）：`type=0`（指定用户）/ `1`（仅管理者）/ `3`（指定身份组）下点击都会被服务端拒绝并提示"无权限操作"。此前只有「`type=0` 且带非空 `specify_user_ids`」的内嵌键盘路径被改写，私聊其余类型与空名单仍原样下发。

- **参照**：Go 主仓 `255af02`——`handlers/message_parser.go` 新增 `NormalizeC2CKeyboardPermission`（遍历 `content.rows[].buttons[].action.permission`，非 2 全归 2，`specify_user_ids` / `specify_role_ids` 等其余字段原样保留），`ResolvePlaceholderUserIDs` 只管 `__USER_ID__` → 对方 OpenID、不再顺带改权限类型；调用点收敛在私聊路径（`send_private_msg.go:516/545/642`、`outbound_bridge.go:976/992`、`send_group_msg.go:2263-2264` 的 `generatePrivateMessage`，该函数只构造 C2C 消息）
- **修改（Rust）**：`crates/gensokyo-runtime/src/deferred.rs` 的 `compose_keyboard` 在挂载前对 `SendTarget::Private` 目标做同样归一（缺 `permission` 的按钮跳过，非对象载荷沿用既有 `1400`）；同时补齐 markdown 内嵌键盘的取用——markdown 载荷里的 `keyboard` 优先、独立 `[CQ:keyboard]` 兜底（此前内嵌键盘只读 `content`，实际上被丢弃），群聊目标不做归一
- **影响**：私聊内嵌 keyboard 与独立 `[CQ:keyboard]` 一律按 `type=2` 下发，按钮不再被服务端拒绝；群聊保留插件原始 `permission.type`、也不解析 `__USER_ID__`（与 Go 群侧现状一致，属已登记差异）
- **测试**：Go `handlers/keyboard_permission_test.go`；Rust `cargo test -p gensokyo-runtime --lib`（348 passed）与 `--test deferred`（22 passed）——覆盖 type 0（带名单 / 空名单）、1、2、3 五组，内嵌与独立两形态，群聊保持 `type=0` 且 `__USER_ID__` 不解析，`__USER_ID__` 在 C2C 换成对方 OpenID

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

### `[CQ:card]` 校验收紧：本地路径 / `http` 一律 1400

- **修改**：`handlers/card_payload.go` 的 `buildTuwenCard` 成为唯一构造与校验点；四字段必填——`title` 1~50 字符、`description` 1~200 字符、`url`/`pic_url` 必须 `https://` 且 ≤1024 字节（Rust `crates/gensokyo-qq/src/dto/message.rs` 同口径）
- **影响**：旧的「非法卡片跳过、继续发文本」与「本地路径自动 OSS 上传」移除；现在整条消息回 `retcode=1400`，不再静默降级；卡片与 @全员 同现（markdown 未优先）也回 `1400`

### `[CQ:music]` 不再发送空 Markdown

- **修改**：`handlers/send_group_msg.go` 的 `[CQ:music]` 分支改为普通文本（`msg_type=0`，正文「分享一首 QQ 音乐」）+ 键盘按钮承载播放链接
- **影响**：不再发出只有模板 `params` / 空 `content` 的 `msg_type=2` 无效载荷

### `native_md` 不再改变 Markdown 载荷形态

- **修改**：`Processor/Processor.go` 两条分支统一构造原生 `markdown.content`；`native_md=false` 不再产出 `{"params":…}`
- **影响**：模板 `params` 形态随 `custom_template_id` 一并废弃；`native_md` 仅作兼容配置保留，不影响载荷

### `custom_template_id` / `template_id` 自动注入移除

- **修改**：`Processor/Processor.go`、`handlers/message_parser.go`、`handlers/send_group_msg.go` 不再按配置自动注入模板字段；模板消息改走原生 `markdown.content`
- **影响**：字段仍可反序列化，但不再生效；`template_id` / `custom_template_id` 出站不构造（Rust 新链同口径）

### 禁言时长超过 30 天按 30 天截断

- **修改**：`handlers/set_group_helpers.go` 新增 `maxMuteDurationSeconds`（30 天）与 `clampMuteDuration`，`set_group_ban` / `[CQ:set_group,action=ban]` 共用；Rust 新链 `crates/gensokyo-actions/src/group.rs` 同步 `MAX_BAN_SECONDS`
- **影响**：超长 `duration` 不再被上游拒绝，按 30 天提交

### @全员限流默认开启

- **修改**：`structs/structs.go` 新增 `enable_at_everyone_limit`（`*bool`，缺省 `true`）；`config.GetEnableAtEveryoneLimit` 仅显式 `false` 关闭
- **影响**：同群同日第 2 次 @全员、单聊带 @全员 都回 `1400`；计数落 idmap config 表 `at_everyone`，跨重启保留

### Rust 新链补齐事件 / 分页 / 上传上限 / 出站类型

- **事件**：新增 `INTERACTION_CREATE` → `notice.interaction`（`sub_type=create`，互动对象整体落在 `data`）
- **分页**：`get_group_member_list` 支持 `cursor`（后改为 handler 内部翻页拉全量、响应不带游标，见上条「`get_group_member_list` 内部翻页拉全量」）
- **上传上限**：直传统一 5,000,000 字节（十进制 5MB），与 Go `file_data` 口径一致
- **出站**：支持 markdown（`msg_type=2`，只带 `content`）与卡片（`msg_type=8`，群聊 / 单聊）

### Rust lotus gRPC 客户端模式（P55-P1~P4，2026-11-13）

Rust 版补齐 lotus 客户端模式 parity：`lotus=true` 时 idmap 17 个操作走远端（gRPC 客户端或 HTTP 回退），对齐 Go `main_grpc.go` / `idmap/service.go` 的三分支语义；远程命中时不写本地库（本地 redb 仍开，供无远程分支的函数使用）：

- **gRPC 客户端**：`RemoteBridge` 专职桥线程（独立 current_thread runtime，RPC 30s 超时 + 5s 建连上限），`IdMapStore::with_remote` 17 方法分支 + `idmap_remote_mode` 模式判定（P55-P1 `4fbfd5d` / P55-P2 `30c0c69`）
- **HTTP 回退**：`lotus_grpc=false` 且 `lotus_without_idmaps=false` 时自打远端 `/getid`（type 1~17 全表，type=11 走 `/getrealvalues`；30s 超时 + 4MiB 响应上限；port=443/force_ssl → https；不带 token/timestamp，照抄 Go 存量缺陷——设 `lotus_password` 会 401，修复另立任务）（P55-P3 `5ca58f3`）
- **boot 装配**：gRPC 服务端方向修正为 `!lotus && lotus_grpc`（P28 原方向相反）；`/getid` 按实际状态隐藏（服务端真起或远程真挂；降级保留挂载，仍带鉴权中间件）；`lotus=true` 主 listener（正向 WS）改用 `backup_port`（P55-P4）
- **与 Go 差异（登记）**：建连失败 warn 降级 vs Go panic；gRPC 30s 超时 vs Go 无超时；服务端模式不阻塞主线程（与 HTTP 共存）；`lotus+lotus_grpc` 不查 `lotus_without_idmaps`（Go 怪癖照抄 + warn）；`delete_config` 远程恒 true / `store_cache_with_ttl` 远程忽略 TTL / `update_virtual_value_pro` 远程单次 RPC 等语义微差——完整清单与配置矩阵见 `docs/rust-m0.md` R4l 条目
- **测试**：41 suites / 1286 passed（P55-P4 时点口径）；双实例集成（服务端实例 + 客户端实例，虚拟 ID 跨实例一致，`crates/gensokyo-runtime/tests/lotus.rs`）

### Rust 裸媒体码不再发出空 file_info（P59，2026-11-14）

`[CQ:image]` / `[CQ:video]` / `[CQ:file]` / `[CQ:record]` / `[CQ:voice]` 不带 `file`/`url` 参数时，Rust 出站链曾产出 `MediaRef::Unknown("")` 并按「已拿到 `file_info`」透传臂**向 QQ 上游发出 `"media":{"file_info":""}`**（空引用必被上游拒绝；Go 真行为是原位保留字面文本，`handlers_media.go:56-59`、`:88-91`，属触及外部 JSON 的硬约束不一致）。修复后裸码与 Go 同口径：字面进正文、不占媒体槽：

- **修改**：`crates/gensokyo-protocol/src/outbound.rs` 的 `from_segments` 媒体归一点新增 `is_bare_media_segment`（五段族且 `file`、`url` 两值皆空）→ 段原文经 `Segment::raw_or_cq()` 放回 `plan.text`，不再进任何媒体桶；单点实现，五臂无特判分叉
- **影响**：裸码原位保留逐字节字面（含方括号与两侧空白，同 Go `Replacement: tok.Raw`），不触发上传、不拆 `msg_type=7` 条；`file=` 有值 / `url=` 有值 / `file=,url=` 有值三类既有行为零变化，`[CQ:file,url=x]` 的 file 臂不读 url 仍按 P57-② 钉死口径
- **测试**：新增 protocol 2 例（五族裸码字面保留与桶为空、混排空白）+ runtime 1 例（经 `MessageOut::from_content` 生产入口断言上游 `MessageToCreate` 不带 media 槽、正文含五族字面）；`from_segments_voice_alias_matches_record_every_shape` 的裸码形态由「与裸 record 逐字段等价」改为「各自保留自身字面 + 都不进桶」——字面保留后两码文本天然不同，属被本修复纠正

### `array_value` 手打 CQ 折回 text（Rust 侧 P72，2026-11-14）

P69 登记的那项入站口径差异（`inbound` 位比 Go 实际产出集超集）经 P71 逐码取证后由 **P72 裁决并实施**：Go `ConvertToSegmentedMessage`（`handlers/message_parser.go:2253-2334` + `:2340` 的段类型集合）对正文**只做整段切片 + `<@OpenID>` 正则、完全不解析用户手打的 CQ** ⇒ `array_value` 上报里用户手打的 `[CQ:…]` 除 `at` 外一律按原文留在 text 段，与 Go 逐字节一致。

- **修改**：`crates/gensokyo-protocol/src/cqcode.rs` 改写 `CodeSpec::inbound` 的文档位语义——这一位从此只回答「`array_value` 上报时用户手打的 `[CQ:x,…]` 要不要落成类型化段」（真→落段、假→按原文逐字节折回 text；折回臂取码段原文，段名/键名归一只发生在落段臂），它**既不是**「Go 是否支持该码」、也**不是**「入站是否会产出该段」；据此把 **18 条码位 true→false**（reply / image / record / video / file / face / markdown / card / keyboard / stream / input_notify / music / active / wakeup / node / poke / ark / json），每条行内注明依据；`at` **保持 `inbound: true`** 并注明为结构性唯一例外（真实 @ 由 `revert_mentions` 合成 `[CQ:at,qq=…]` 后再靠这一位回解，收假会让全部真实 mention 退化成文本，彻底对齐另立 P73）；`crates/gensokyo-runtime/src/lib.rs` 的模块文档按代码事实改写（image / record / ark / reply 段已按 `attachments` 与 `msg_elements` 经 `image_segments` / `message_event_segments` 产出，只有**视频附件**仍不产段——Go M2-B 的 `image/` 前缀过滤、语音走 record 段），旧表述（称 image / record / video / ark 四类段尚未产出）作废
- **影响**：`code_spec(…).inbound` 的**生产读取点只有** `crates/gensokyo-runtime/src/inbound.rs` 的 `reported_message` 一处，故外部可见变化仅限「`array_value=true` 时手打码的段数组由类型化段改为 text 段原文」；`image_segments` / `message_event_segments` 在其之后 `extend`，**结构化附件与卡片上报不受收位影响**（混排用例实证：真图片附件照旧落 image 段，同一条正文里手打的 `[CQ:image,…]` 留 text，字符串路径追加的媒体码与之互不影响）。零回归面：`cqcode.rs` 码表往返与 verbatim 用例、actions `messages.rs` 的 face 断言均未改动且通过；`golden_inbound_matches_go` 不受影响（其 config 无 `array:`，走 Text 分支）。**遗留（勿当已全部收口）**：`poke` 的收位依据在 **P72 本轮未复扫**（沿用 P71 取证：全 Go 树大小写不敏感零命中 + 无 `REACTION_*` 处理器），随后于 2026-11-14 的复扫波次中**源码级确证**（`.go` 零命中、`Processor/` 对 `REACTION` / `GROUP_POKED` 等零命中），但复扫同时暴露一项覆盖不对称——**Go 侧没有「手打 CQ 在数组上报留 text」的入站用例**能钉住该结论，已另立 **P75①** 欠账（该项已于 2026-11-14 由主仓提交 `39a4d5b` 补齐：`handlers/inbound_handtyped_cq_test.go`）；`at` 的结构改造 = **P73**（**已于同日完成**，见下方 P73 子节；本条「`at` 保持 `inbound: true`」为 P72 时点记录）
- **测试**：`crates/gensokyo-runtime/tests/inbound.rs`（改动 137 行：+133 / −4）纠正旧用例——原 `array_value_parses_at_and_multiple_segments`（现名 `array_value_keeps_at_segment_and_folds_typed_face_code`）由 5 段（含 face 段）改为 3 段，手打 `[CQ:face,id=1]` 留在相邻 text，`at` 断言未动、只是相邻合并；新增 `array_report` 三视图助手（段数组 JSON / `raw_message` / `message` 纯文本）、`array_value_folds_every_inbound_false_code_verbatim`（20 例，混入码名大写、段名内空格、大写键名、`&#44;` 转义、JSON 值与 stream 冒号语法；先断言确被 parse 成码段再断言整段折回，避免顺从「解析失败本来就留 text」这条与本位无关的路径，并断言 `message` 文本与 `raw_message` 逐字节一致）、`array_value_keeps_structured_image_segment_while_folding_typed_image_code`（真附件与手打码混排）。**门禁**：`cargo test --workspace` 43 suites / 1373 passed / 0 failed（P72 时点实测，lead 独立复核通过；当前基线见下方 P73 子节），`cargo clippy` 零告警、`cargo build --release` Finished、`cargo fmt --all -- --check` 退出码 0

### at 段占位符直出（Rust 侧 P73，2026-11-14）

P72 之后最后一个手打码位偏差收口。Go 的 `ConvertToSegmentedMessage`（`handlers/message_parser.go:2182-2336`）从不解析正文里的 CQ：`at` 段只由 `<@!?([0-9A-Fa-f]+)>` 正则（`:49` 的 `incomingAtRe`）对占位符展开产出，`<@all>` 等非 hex 形态原样留 text。Rust 原先靠 `revert_mentions` 合成 `[CQ:at,qq=…]`、再由 `CodeSpec::inbound` 回解成段——既让手打 `[CQ:at,…]` 被类型化（与 Go 不一致），又使该位无法收假。

- **修改**：`crates/gensokyo-runtime/src/inbound.rs` 的数组形态改为「Go 同款 NUL 哨兵 `"\x00ATPH%d\x00"` 正文 + at 槽位表」，`reported_message` 数组臂删除 `cqcode::parse`，由 `expand_at_placeholders` 在「条件 trim → 去首个 `/`」之后按哨兵切片直出 text/at 段（顺序与「找不到哨兵就跳过、不中断」照 Go `:2269-2323`）；at 段用 `Segment::new("at").param("qq", …)` 构造，不用 `Segment::at`（规避 id 规范化风险）；`push_text_segment` 随最后一个 `cqcode::parse` 调用点删除（全仓 grep 零调用）；码表 `at.inbound` 由 true 改 false，`CodeSpec::inbound` 自此**无生产读取点**，与 `params`/`outbound` 同属描述性元数据；`crates/gensokyo-runtime/src/lib.rs` 模块文档按新口径改写。
- **影响**：手打的 `[CQ:at,qq=…]` 与 Go 一致按原文留 text；真实 `@` 仍由 `<@!hex>` 占位符产 at 段；非 hex（如 `<@all>`）原样留 text（`MentionEveryone` 全 Go 仓零消费）；出站 `at_everyone.rs` 未动，`raw_message` 字节不变。
- **测试**：Go `handlers/zzzz_inbound_at_order_test.go`（`TestInboundAtOrderPreserved`）的 8 组 `wantStr`/`wantArr` 逐字移植 + 交叉不变式（段数组拼回 == `raw_message`）+ 非 hex / 前导零 / 超 i64 / 哨兵丢失 / 连续多 `@` 五类边界；门禁 `cargo test --workspace` **43 suites / 1379 passed / 0 failed**、`cargo clippy` 零告警、`cargo build --release` Finished、`cargo fmt --all -- --check` 退出码 0。**说明**：按磁盘与并发纪律未运行 Go 侧 `go test`，Go 依据为源码与测试期望值的静态核对。

### 出站 avatar / music 不可消费时保留字面（Rust 侧 P79，2026-11-14）

P74 双仓核实发现：Go 默认 `cq_parse_mode=legacy` 下，`[CQ:music]` 只消费 `type=qq,id=<数字>`（`handlers/cqcode.go:38`），其余形态原样留正文；`[CQ:avatar]` 字符串路径替换为 `[CQ:image,file=…]` 字面、段路径反查 openid 后发真图。Rust 此前：`music` 只查 id 不看 type（`type=163` 被误当 QQ 音乐发送），`avatar` 无对应臂 ⇒ `extra → unsupported_segment → Error::Protocol →` 整条 **1400**。

- **修复**：`from_segments` 新增 `avatar` 臂按原文写回 `plan.text`；`music` 臂收紧为「`type` 恰为 `qq`（小写敏感）且 `id` 非空纯 ASCII 数字」才进 `qqmusic`，其余按原文写回 ⇒ 两类不可消费形态不再拒发整条消息；同步改实注释（music 出站为 `msg_type=2` markdown+键盘；avatar 的 Go 消费点与 P80 待办）。
- **影响**：含 `[CQ:avatar,…]` 或裸/非 qq `[CQ:music,…]` 的消息由「整条 1400 拒发」变为「按原文字面发出」；`[CQ:music,type=qq,id=123]` 行为不变。**已知差异**：avatar 可见形态与 Go 不同（Rust 保留原文，Go 为 image 字面/真图）⇒ P80；music 乱序/多余参数 Rust 消费而 Go 不识别；其余不可消费码仍会 1400 ⇒ P81。
- **测试**：协议 2 例（六形态字面 + 反面 `type=qq,id=123` 仍消费）、runtime 1 例（含 avatar/music 的消息按字面发出、不 1400）；全量 **43 suites / 1382 passed / 0 failed**、clippy 零告警、release Finished、`fmt --all -- --check` 退出码 0。

### 环回假红测试加固（P82，2026-11-14）

- **修改**：`gensokyo-media` 的 `ssrf hop`、`gensokyo-runtime` 的 `lotus_variant_replaces_via_transform` / `lotus_mint_omits_token_when_password_empty`、`gensokyo-storage` 的 `http_remote` 17-op 用例，改用「stub 侧是否收到完整请求」为判据，重试上限 3、轮间退避、每轮清空 stub 记录；断言强度不降（hop 逐跳全序、lotus 恰好一次 `POST /url`、17 条逐字等价）。
- **影响**：仅测试代码；本机安全软件间歇掐环回导致的假红不再污染门禁。
- **测试**：三处各五连跑全绿；三 crate 822 passed；workspace **43 suites / 1382 passed / 0 failed**。

### 出站不可消费段三分界与纯码空消息回执（Rust 侧 P81，2026-11-15）

P81 逐码核实 Go 三条出站路径后定界：字面保留 / 静默丢弃 / 仍 1400。

- **修复**：`from_segments` 改为三分界——`face`/`node`/`poke`/`group_info`/`json`/未知码按原文写回正文；`ark` 与空 `[CQ:card]` 静默丢弃；空或非法 `markdown`/`keyboard`/`stream`/`input_notify` 按字面保留（payload 接受形态收紧为 `base64://` 或 JSON，对齐 Go 正则）；`member`/`set_group`/`remove` 仍 1400（Go 真执行、Rust 未实现，P83）。`execute_inner` 新增纯码空消息出口：正文空且无内容时直接成功回执、不调 QQ API（对齐 `handlers/send_group_msg.go:557-562`）。
- **影响**：含 `[CQ:face]` 等无消费点码的消息不再整条 1400，改为按字面发出；空 `[CQ:card]`/`[CQ:ark]` 静默消失；纯码消息回执的 `message_id` 类型修正另见 P86。
- **测试**：协议 5 例 + runtime 3 例（含 `literal_segments_round_trip_byte_for_byte` 幂等、`empty_pure_code_message_acks_without_sending`）；全量 **43 suites / 1391 passed / 0 failed**、clippy 零告警、release Finished、fmt 退出码 0。

### 构建版本串注入（P85，2026-11-15）

- **修改**：`cargo xtask build` 给 cargo 子进程注入 `GENSOKYO_BUILD_VERSION`——git 时 `git-<7 位短 hash>`、否则 `dev-<epoch>.<ms>`，同名环境变量优先；`version.rs` 文档与 `build.ps1` 注释同步。
- **影响**：发布产物 `--version` 与 Go `Get-BuildLdflags` 同形态、可按短 hash 追溯；直接 `cargo build` 仍走 `dev-…` 回退。
- **测试**：xtask 30 passed、bin 28+4 passed、clippy 0、fmt 0；`cargo xtask build` 产物输出 `gensokyo git-c8206f2`。

### avatar 出站发 qlogo 真图（Rust 侧 P80，2026-11-15）

- **修复**：带 `qq` 的 `[CQ:avatar]` 出站按 Go 段路径/cqparse 口径补齐——协议层把 `qq` 搬进 `OutboundPlan.avatars`（握手经 `to_segments`/`from_segments` 往返），runtime 在 `execute_inner` 判空之前反查 openid 并生成 `https://q.qlogo.cn/qqapp/{app_id}/{openid}/640`，走既有图片链上传；反查失败或 `app_id` 未配置只吞段 + `warn`，不再 1400、不再把 CQ 码字面发给用户。
- **影响**：avatar 出站可见形态与 Go 段路径一致；无 `qq` 的形态仍按字面保留。已知差异（登记）：`app_id` 未配置时 Go 发坏 URL 换 1500、Rust 吞段；idmap 仅一维反查（Go Pro 二维）；头像图追加在 `plan.images` 末尾、多图混排次序不严格同序。
- **测试**：protocol 3 例 + runtime 6 例（含真 serde 握手往返 `avatar_becomes_qlogo_image_after_handshake`、反查失败吞段、`app_id` 未配置吞段、与真实图片混排）；全量 **43 suites / 1399 passed / 0 failed**、clippy 零告警、release Finished、fmt 退出码 0。

### action 回执 message_id 类型对齐（Rust 侧 P86a，2026-11-15）

P86 审计（parity §F）确认的 D1/D3/D4：Go numeric 模式（默认 `string_ob11=false`）的 `data.message_id` 是 int（echo 派生虚拟行号），string 模式是上游原串，空回执两种模式都是数字 `0`；Rust 此前恒回字符串。

- **修复**：`dispatcher::message_id_value` 按三分支渲染——空回执 → 数字 `0`；`string_ob11=true` → 原 ID 字符串；numeric → `EchoStore::store_cache_in_memory` 派生数字（失败退回 `parse::<i64>`，再失败保留字符串 + `warn`，**不伪造 0**）。`EMPTY_MESSAGE_ID` 提为 `pub(crate)`。SSE/forward 分支未动（P86b/P86c）。
- **影响**：默认 numeric 配置下，发送类 action 回执的 `data.message_id` 由字符串改为数字；string 模式保持原串（且不登记 echo）。回执与 Go 的 numeric/string 双模式一致。
- **测试**：5 个 dispatcher 单测（numeric/string/空回执/派生失败两条）+ 2 个集成用例；并做反向验证（还原旧实现 → 5 例红）。全量 **43 suites / 1406 passed / 0 failed**、clippy 零告警、release Finished、fmt 退出码 0。

### action 响应外层信封按 Go 对齐（Rust 侧 P86b，2026-11-15）

- **修复**：`ActionResponse` 按 Go 的 map 字母序声明字段（`data, echo, group_id, message, next_cursor, retcode, status, user_id`）；`group_id`/`user_id` 为 omitempty（反查失败不写、不伪造 0）；`message` None→`""`、`echo` None→`null` 恒序列化；`wording` 不上线（grep 零消费者）；`next_cursor` 保留为 Gensokyo 扩展。dispatcher 在 `finalize` 按 deferred target 填顶层 ID——群 string 用请求原串、群 numeric 用 idmap 虚拟号、私聊两模式恒数字（照抄 Go `SendC2CResponse`），空消息回执同样带 ID。
- **影响**：发送类 action 顶层 JSON 与 Go 对齐；已知差异登记：失败回执不填 ID、numeric 下原数字串不重复分配、`channel_id`/`guild_id` 未覆盖。群管理/查询类的 per-action 信封另立 **P86c**。
- **测试**：协议 4 例 + runtime 6 例整串断言（群/私聊 × numeric/string + 空回执 × 群/私聊）；全量 **43 suites / 1416 passed / 0 failed**、clippy 零告警、release Finished、fmt 退出码 0。

### per-action 响应信封（管理类 Compact，Rust 侧 P86c，2026-11-15）

- **修复**：新增 `EnvelopeKind`（`Full`/`Compact(CompactData::{Omit,AsIs,EmptyObject,Null})`）与 `envelope_for(action)` 表（B 类 23 个管理 action 逐条对 Go 源码映射）；`ActionResponse` 改为手写 `Serialize`（两套键集、恒按字母序）。Compact 下：`data` 按子形态（4 省略 / 15 有值 / 2 `{}` / 2 `null`；**失败一律不写 data**）、`echo` 缺省省略、`message` 恒在、不写 `group_id`/`user_id`；B 类失败 `retcode 100 / failed`。dispatcher 在唯一出口按 action 设置信封，**未改任何 handler 业务逻辑**。
- **影响**：群管理类响应不再多出 `data`/`echo` 字段；A 发送类与 C 查询类保持 Full；A 类失败仍 1400/1500（D7）。剩余见 P86d（`next_cursor` 进 data、`mark_msg_as_read` 的 `msg`/`wording`、C 类个别字段）。
- **测试**：协议 8 例 + runtime 3 例（Compact 七条 golden 整串）；全量 **44 suites / 1427 passed / 0 failed**、clippy 零告警、release Finished、fmt 退出码 0。

### 响应信封收尾（Rust 侧 P86d-1，2026-11-15）

- **修复**：删除 `ActionResponse` 顶层 `next_cursor`（Go 无此字段）——分页 action 的游标按 Go 放在 `data` 内（join request list / panel list / blacklist / media list），`get_group_member_list` 不再输出游标；`mark_msg_as_read` 顶层改为 Go 的 `msg` + `wording`（空串、无 omitempty，`msg` 顶替 `message`）；`dispatcher::encode` 兜底按信封模式输出（Full 带 `data/echo/message`，Compact 带 `message/retcode 100`）。
- **影响**：分页与已读回执的键集/位置与 Go 对齐。已知差异（登记）：`mark_msg_as_read` 的 `data`——Go 是 `gjson.Result` 零值结构体转储、Rust 按语义回 `null`；成员列表 Rust 仍只回一页（Go 内部拉全量）另项。
- **测试**：新增/改写 4 条整串断言（join request / panel list / member list / mark read / encode 兜底）；全量 **43 suites / 1429 passed / 0 failed**、clippy 零告警、release Finished、fmt 退出码 0。

### SSE / 合并转发回执补齐（Rust 侧 P86d-2，2026-11-18）

P86 审计遗留的两条回执路径（`send_private_msg_sse` 与 `send_*_forward_msg`）此前既不按 `string_ob11` 渲染 `message_id`，也不带顶层收件人 ID——Go 的两条回执都从 `message.Params` 取 `user_id` / `group_id`。

- **修复（SSE）**：`dispatcher::run_deferred` 的 `message_sse` 分支把 `message_id` 改经 `message_id_value` 双模式渲染（空回执恒数字 `0` / string 模式原串 / numeric 模式 echo 派生），`target` 由 `None` 改为计划收件人（`SseRequest.target` 恒私聊），经 `receipt_ids` 补顶层 `user_id`（数字虚拟号，与 Go `SendC2CResponse` 一致）
- **修复（合并转发）**：`message_ids` 元素逐个过 `message_id_value`（此前是上游原串）；顶层补 `group_id`——**恒数字**，因为 Go 的转发 handler 直接调 `SendResponse`（`handlers/send_group_forward_msg.go:75`），不经 `send_group_msg.go:1020` 的 `string_ob11` 分派；`failed` 清单（含原 node 序号）保留
- **影响**：默认 numeric 配置下 `send_private_msg_sse` 的 `data.message_id` 与 `message_ids` 元素由字符串改为数字，回执多出 `user_id` / `group_id`。已知边界：整次转发没有任何可发 node 时计划里拿不到收件人（`ForwardRequest` 无顶层 `target`），`group_id` / `user_id` 留空而非伪造
- **分侧登记（刻意差异）**：① SSE 的 `message_id`——Go `SendC2CResponse` 无 string 分支、恒派生数字，Rust `string_ob11=true` 时回上游原串（超集，照 P86a 双模式口径）；② `message_ids` / `failed` 是 Rust 独有增强，Go 转发只回 `message_id: 0`；③ `send_private_forward_msg` 在 Go 未注册（Rust 扩展 action），其回执 ID 按私聊口径给数字虚拟号
- **测试**：`crates/gensokyo-runtime/tests/deferred.rs` 新增 3 例（SSE string 模式整串断言、转发两模式 `message_ids` 元素类型 + `group_id` 恒数字 + `failed` 保留、全空 node 转发回 `message_ids: []` 且顶层 ID 留空），并按新口径改写 3 例旧断言（SSE 整串、群转发 `message_ids` + `group_id`、私聊转发 `message_ids` + `user_id`）；`deferred` 目标 **25 passed / 0 failed**

### Action 响应信封审计（P86，2026-11-15，审计项）

- **发现**：Go 的 numeric 模式（默认 `string_ob11=false`）响应里 `data.message_id` 是 **int**、`group_id` 为 int64；string 模式为原串 + OpenID；Rust dispatcher 恒回字符串且缺 `group_id`/`user_id`；空回执 `0` vs `"0"`；顶层 `message`/`echo` 缺省省略；键序不同。D1–D9 详见 `reports/refactor/17-go-rust-parity.md` §F。
- **裁决**：D1–D4、D6、D9 是正常路径硬性差异，必须修（P86a→P86b）；D5（私聊空消息 Go 无回执）/D7（Go 失败恒 ok）/D8（Go 参数错无回执）保留 Rust 更安全行为，登记为刻意分侧。
- **状态**：修复待 P80 落地后开始（同改 `dispatcher`/`deferred`）。

---

## 📝 工程与文档变更

### 矩阵对齐 Go 五平台 + 产物改名 gensokyo-rust-*（2026-11-19）

`--all` 的平台集合改回 Go 主仓 `build.ps1` 的 5 个平台（linux×3 + windows×2），产物统一加 `rust` 标识；`--workers 2` 的缓存从 3 份压回 2 份。

- **修改**（`xtask/src/build.rs`）：`MATRIX` 改为 `x86_64/i686/aarch64-unknown-linux-gnu` + `x86_64/i686-pc-windows-gnu`（顺序照 Go），双版本共 10 个产物；darwin / msvc / android / armv7 移出矩阵，仍可 `--target <triple>` 显式构建，对应 `[skip]` 判定原样保留；`artifact_name` 改为 `gensokyo-rust-{os}-{arch}[-noWebui][.exe]`；新增 `windows_gnu_toolchain` / `target_env_overrides`（纯映射 `windows_gnu_env` 便于单测）：`*-pc-windows-gnu` 按位宽探测 mingw gcc（x86_64：`x86_64-w64-mingw32-gcc` → `C:\msys64\mingw64\bin\gcc.exe` → PATH 的 `gcc`；i686：`i686-w64-mingw32-gcc` → `C:\msys64\mingw32\bin\gcc.exe`），命中后给 cargo 子进程注入 `CARGO_TARGET_<TRIPLE>_LINKER`、`CC_<target>`（连字符 / 下划线两种拼写）并把该 gcc 的 `bin` 目录前置到 PATH——实测只设 `CC`/linker 时，`C:\msys64\mingw32\bin` 不在 PATH 会让 32 位 gcc 的子进程加载不到同目录 DLL，`gcc -E` 静默退出码 1（`ring` 编译器家族探测失败），前置 PATH 后 i686 构建通过；矩阵模式下找不到 mingw 则 `[skip] <triple>: 需要 mingw-w64 工具链（<arch> gcc 不在 PATH）`、不计失败；`worker_target_dir` 改为 worker 1 用 `<base>` 本体、i≥2 才派生 `-w<i>`
- **影响**：`--all` 与 Go `-All` 产物一一对应（10 个）；windows 产物由 msvc 换成 gnu（链接器与 C 编译器都是 mingw gcc，不再需要 `cl.exe`）；`--workers 2` 只占 `base` + `base-w2` 两份缓存；新文件名与旧 `gensokyo-*` 并存时不会互相覆盖，旧产物需手工清理
- **文档**：`docs/rust-build.md`（矩阵表、产物命名、并行缓存预算、windows-gnu 的 mingw 说明与 `[skip]` 语义、`--linux-only` = 3 目标）、`AGENTS.md`（产物命名 / `--all` 矩阵 / 缓存预算）、`readme.md` 同步；启动横幅 `TITLE` 与欢迎语改为 `Gensokyo-NewQQ-Rust`（`crates/gensokyo-bin/src/banner.rs`，`LOGO` 与其它文案未动）
- **测试**：`cargo test -p xtask` 40 passed / 0 failed（新增 `all_plans_the_go_matrix_in_order`、`first_worker_reuses_the_base_cache`、`windows_gnu_gcc_names_follow_the_architecture`、`windows_gnu_env_sets_linker_and_cc`、`target_env_overrides_only_covers_windows_gnu`）；`cargo clippy -p xtask --all-targets -- -D warnings` 零告警；`cargo fmt --all -- --check` 退出码 0

### `--all` 多目标并行构建（2026-11-19）

`cargo xtask build --all` / `--linux-only` 从逐目标串行改为多 worker 并行；产物命名、`[skip]` 语义、汇总行与退出码零变化。

- **修改**（`xtask/src/build.rs`）：矩阵模式默认 2 个 worker，`--workers <1..=4>` 与 `GENSOKYO_BUILD_WORKERS` 双入口（命令行优先，非法值打印 `[warn]` 回退默认，上限 4，且不超过实际 job 数）；每个 worker 用派生的 `<CARGO_TARGET_DIR>-wN` 缓存目录（cargo 对整个 target 目录加独占文件锁，共用目录只会 `Blocking waiting for file lock` 排队），`--workers 1` 仍用 `<base>` 本体；`rustup target add` 补装改为并行前**串行**装齐（并发补装会撞 rustup 的 `could not rename … .partial (os error 2)`）；每个 worker 的 cargo job 数 = 总 job 数（`CARGO_BUILD_JOBS`，缺省按 CPU 数）÷ worker 数、下限 1；各 worker 的 cargo 输出先捕获，全部结束后按矩阵顺序回放（单 worker 仍是实时输出）；UPX 压缩同样按 worker 数并行；worker 线程 panic 转成该 job 的 `[fail]`、不中断进程；新增纯函数 `shard`（连续切分）与单测
- **影响**：矩阵构建的墙上时间缩短——release 为 `lto="fat"` + `codegen-units=1`，单个目标的最后 LTO + 链接是单线程，跨目标并行才吃得满 CPU；代价是并行模式多 1 份缓存目录（缓存预算 2 份，上限 4），**首次在新目录里要重编依赖、比串行更慢**，第二次起吃到缓存才快；单目标构建（`--target` / 宿主）默认仍 1 worker、缓存目录与输出行为不变
- **测试**：`cargo test -p xtask`、`cargo clippy -p xtask --all-targets -- -D warnings`、`cargo fmt --all -- --check` 全绿（含 `shard` 新用例：`len=0/1/9`、`workers=1/2/3/4`、`workers>len`）

### 恢复 `build.ps1` 为 xtask 薄包装（2026-11-18）

纯 Rust 化清理（`63cd9b0`）把 `build.ps1` 误当 Go 遗留删除；该文件自 `9d9e254` 起已是 Rust 入口——只做 Go 命名参数 → Rust 三元组翻译与 Windows 工具链自动选择（缺 MSVC 回退 GNU），构建逻辑全在 xtask，UPX 默认 7。

- **修改**：按 `9d9e254` 后的版本恢复 `build.ps1`（内容零改动，行尾 `i/lf`）；`docs/rust-build.md` 撤回「本分支没有 `build.ps1`」的失效口径，命令对照表改为「Go 版 `build.ps1`（`main`）/ 本分支 `build.ps1`」双列；`docs/rust-handover.md`、`docs/rust-migrate.md`、`docs/本版新增功能.md`、`readme.md`、`AGENTS.md` 同步补回该入口
- **影响**：Windows 上 `.\build.ps1` / `.\build.ps1 -All` 与 `cargo xtask build` / `--all` 等价，不再需要手敲参数翻译；薄包装不参与构建逻辑，与 xtask 行为严格一致
- **测试**：`.\build.ps1 -NoUPX` 宿主构建成功（`release/gensokyo-windows-amd64.exe`，12.5 MB，退出码 0）；`.\build.ps1 -All` 在假 zig 版本下正确透传 `--all`，输出 9 个 `[skip]` 后按「无产物」非零退出

### 文档同步（群管理三接口 + 入站新字段）

- `docs/api/扩展api/扩展api-get_group_bot_state.md`：字段表与示例改为官方字段 + 兼容别名
- `docs/api/扩展api/扩展api-get_group_join_request_list.md`：请求/响应改为 `cursor`/`list`/`next_cursor`，示例改为游标分页循环
- `docs/cq码/扩展CQ码/扩展cq码-cq-set_group.md`：补 `whole_ban` 固定失败说明（`set_group_ban` 增量语义）
- `docs/cq码/标准CQ码/标准cq码-cq-file.md`：补 5MB 直传上限与自动分片说明
- `docs/本版新增功能.md`：`API` 表三行更新、新增「入站事件新增字段（2026-09-16）」「富媒体上传：5MB 直传上限与自动分片」小节、`事件字段` 补 `GROUP_JOIN_REQUEST`、错误码补码说明

### 文档同步（素材管理 / @全员 / 单聊卡片）

- `docs/api/扩展api/扩展api-media_images.md`：新增三个 action 的完整参数、返回、错误码与示例
- `docs/api/api介绍.md`：扩展 API 表补 `upload_media_image` / `get_media_image_list` / `delete_media_image`
- `docs/本版新增功能.md`：`API` 表补三行，新增「素材管理」「@全员」小节，`[CQ:card]` 改为群聊/单聊口径
- `docs/cq码/扩展CQ码/扩展cq码-cq-card.md`：补单聊支持、四字段必填与字段边界、`https` 限制、整条 1400 行为（含与 @全员 同现）
- `docs/cq码/扩展CQ码/扩展cq码-cq-at.md`：补 `qq=all` / 字面标签、每群每自然日 1 次、时区与限流开关
- `AGENTS.md`：`foundItems` 的 `card` 行、`msg_type=8` 群聊/单聊、`qqbot-media://` 透传、@全员 限流
- `template/config_template.go` / `crates/gensokyo-config/templates/config.yml`：新增 `enable_at_everyone_limit: true` 中文注释；`custom_template_id` 标注已废弃
- `readme.md` / `docs/文档-markdown消息.md`：`custom_template_id` 可用描述改为「已废弃，不再生效」
- `docs/文档-markdown消息.md` / 两个 config 模板：补 `native_md` 不再改变载荷形态、`params` 形态废弃；`docs/本版新增功能.md` 补 `native_md` 与 `[CQ:music]` 小节

### 文档同步（lotus 客户端模式 P55-P5）

- `reports/refactor/17-go-rust-parity.md`：lotus gRPC idmap 服务行、idmap 解绑/重绑定行的「客户端模式未实现 / lotus 远程分支未移植」缺口改为已移植（P55-P1~P4，注明 commit）；`/getid` 挂载判定表述更新为按实际状态隐藏
- `docs/rust-m0.md`：追加 R4l 波次条目（能力概述 / 配置矩阵六行 / 与 Go 差异清单 / 测试证据），里程碑时点更新至 2026-11-13
- `docs/rust-build.md`：「从 Go 版升级时的数据注意」补 lotus 运维注意（建连失败 warn 降级本地、无运行时自动切回、桥线程 panic 在 panic=abort 下退出进程需 supervisor、backup_port 端口语义映射）

### 文档同步（出站 URL 管线 P56-T4 收口）

- `reports/refactor/17-go-rust-parity.md`：§A「QR + 出站 URL 转换管线」行缺失 → **已对齐（P56-②）**，写明四层链路（xurls 扫描 / Q 级容错无静区 QR / mint 两分支 / 发送序与响应丢弃）；「SSRF 防护（通用下载/出站入口）」行 部分 → **已对齐（P56-① `53b81d9`）**；「视频处理」关闭行与 `crates/gensokyo-media/src/lib.rs` 未做项清单口径对齐（Go 死代码不移植、视频**发送链**已覆盖）
- `docs/rust-m0.md`：追加 R4m 波次条目（能力概述 / 与 Go 差异清单 / 测试证据）
- `crates/gensokyo-media/src/lib.rs`：未做项清单「视频处理」补「Go 死代码不移植」措辞（与 parity §A 关闭行同口径）；「QR 码生成」确认后本不在未做项清单（P56-T2 已交付）

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

### CQ 码表元数据与口径校准（Rust 侧 P69，2026-11-14）

只动 `crates/gensokyo-protocol/src/cqcode.rs`，**P69 本波零行为变化**（真正改动上报行为的是后续 P72，见「🐛 修复」段同名子节）；完整口径见 `docs/rust-m0.md` R4q 与 parity 报告 B4。

- **修改**：`ark.params` 由 `["data"]` 改为**入站三键** `["ark_type", "ark_name", "prompt"]`——Go 入站的 CQ 文本形态只有这三键（`handlers/message_parser.go:2496-2498`），`fields` 只进段 JSON 的 `data`（`arkSegmentData`，`:2432-2444`），而出站对 ark **整段剥离、一个参数都不读**（`handlers/cqcode.go:198`、`:203-206` + `handlers/cqparse/handlers_control.go:26` 的 `inboundOnlyHandler`）⇒ 旧值在两个方向都不成立，不是「出站口径」；`CodeSpec` 三字段的消费者写实——`params` 是**纯描述性元数据**（全仓唯一读取点是同文件测试模块遍历 `CODES` 的往返用例，读 `spec.params`）、`inbound` **是行为开关**（唯一生产消费者 `crates/gensokyo-runtime/src/inbound.rs:1622` 的 `reported_message`，决定 `array_value` 上报时用户手打的 `[CQ:x,…]` 落成类型化段还是折回 text 段）、`outbound` **无生产消费者**（只有测试模块 `code_spec(…).outbound` 那组断言）；`face` **当时**维持 `inbound: true` 并在码表 `face` 条目的注释里写明「这一位管的是用户手打要不要类型化，与 Go 是否产出 `CQ:face` 无关」——**该结论已被 P72 推翻并收假**（见「🐛 修复」段的 P72 子节）。`cqcode.rs` 的行号随码表与用例增删浮动，上述位置一律**按符号定位**
- **影响**：**P69 本波零行为变化**（改的是「按元数据推断行为」的误导面）；由它牵出的入站口径差异当时登记为待裁决项，**现已由 P71 取证、P72 裁决并实施**——终态与影响面见「🐛 修复」段的「`array_value` 手打 CQ 折回 text（P72）」子节，本条只保留取证轨迹，不再作结论
- **测试**：新增两条行为断言 `ark_inbound_cq_form_carries_three_keys`（钉住 ark 入站三键形态）、`face_cq_text_is_preserved_verbatim`（face 段原文逐字节保真）；protocol 用例只增不改，数字以 `cargo test --workspace` 输出为准

### lotus stub 用例加固（Rust 侧 P70，2026-11-14）

- **修改**：`crates/gensokyo-runtime/tests/url_pipeline.rs` 的 `lotus_stub_replaces_with_remote_link_inplace`（`:478`）按仓库既有「仅连接层中止才重试、上限 3 次」口径加固（同 `tests/lotus.rs` 与 storage `http_remote.rs` 的 10053 处理）——每轮开头清空 stub 请求记录与 api 发送记录（`stub.clear()` / `api.clear_sent()`），否则上一轮会污染计数断言；判定指纹从「错误文本」下移到「stub 侧未收到完整 `POST /url`」（`conn_aborted`），因 `LotusMint::mint` 按 Go 口径把发送失败吞成空串、用例观测不到错误；并先要求「已有记录」才算被掐断（`all()` 对空集合恒真，否则 mint 根本没接线时会白跑满 4 轮才在最外层断言暴露）；业务失败（非 200 / JSON 坏 / 缺 `shortURL`）不参与重试判定、照常上抛
- **影响**：仅测试代码，产品链路零变化；本机安全软件间歇掐环回造成的偶发失败不再靠重跑整个套件规避

---

## ✅ 提交记录

```
2d9fa6e  docs: P79/P82 文档收口与已知差异登记
56cf231  test: 加固环回依赖用例的间歇假红（P82）
ca59e2b  fix: 出站不可消费的 avatar/music 保留字面，不再整条 1400（P79）
58de264  docs: P75 闭环回写、P78 注释修复登记与 readme 戳一戳对齐主仓
a78574f  docs: 修正 card 别名的三处陈旧注释（P78）
27a661d  docs: P73 收口与 P77 分侧登记，CHANGELOG 提交记录回填全量
791734b  docs: P69/P70/P71/P72 文档收口与取证轨迹留档
15b1a4e  fix: array_value 手打 CQ 按原文折回 text（P72，收 18 码位）
ebe26c3  feat: user_info 写侧闭环 + 码表元数据口径修正（P64b / P69① / P70）
3095302  docs: 交接文档 + P59/P64a/P65 文档收口
970f60d  fix: 裸媒体码按臂判定 + get_friend_list 读侧（P59/P59b/P64a）
cc7e479  docs: 新增换机迁移手册 docs/rust-migrate.md
ff72776  docs: 补「迁移到新机器」与构建缓存位置约定
a4a1312  feat: 出站 URL 转换管线 + QR 侧发送 + 重试/voice 别名收口（P56-T3/T4 · P57）
53b81d9  feat: P56 前段——SSRF 守卫 + xurls 移植 + QR 生成（①/T1/T0+T2）
aeabe8c  feat: lotus 客户端模式 boot 装配 + 文档 + 评审修复（P55-P4/P5）
5ca58f3  feat: lotus HTTP 回退桥 + type 全表（P55-P3）
032a40d  feat: 构建链自动引导 zig/cargo-zigbuild（Linux 交叉免安装）
30c0c69  feat: IdMapStore 远程分支 + lotus 模式判定（P55-P2）
4fbfd5d  feat: lotus gRPC 客户端 RemoteBridge 专职桥线程（P55-P1）
857817e  docs: 纠正 parity 过期状态（union_webhook P17 / AppID-BotID P15 已实现）
65a77fb  feat: 段 JSON 键序对齐 Go + 判等顺序无关（P53/P54）
594c9d4  feat: ark fields 真对象化（P52）
662cbd5  feat: 入站媒体段 + 评审收口（P50/P51）
b10aaca  feat: /getid type 14 对齐 + gRPC RPC 14（P49）
c54d968  feat: /getid type 5/12 重绑定 + gRPC RPC 5/6（P47/P48）
65b625e  feat: union_id 入站切换 + 配置清洗 + /uploadrecord（P42/P43/P44/P45/P46）
caec0be  docs: 补端到端复测 §4.4（Rust 最终 release vs 最新 Go，2026-11-10）
2805da2  perf: 统一共享 HTTP 传输（P41）
7b3309a  docs: 里程碑补 R4h 波次（P33/P36/P38/P39/P40）
36fb95b  perf: mimalloc 分配器 + 短链/配置自愈接线 + unioncache（P33/P36/P38/P39/P40）
6122182  feat: parity 余项与运行效率波次（P29b/P30/P31/P34/P35/P37）
328189f  docs: parity 表与里程碑回写 R4f 波次（P20/P21/P26/P27/P28/P29a）
1a5dd7f  feat: 入站 mention/审核/端点/分片媒体补齐波次（P20/P21/P26/P27/P28/P29a）
b3ce791  feat: 连接/入站/端点/传输补齐波次（P15/P17/P18/P19/P22/P24/P25）
890ee0e  feat: /uploadpic 系列 + /getid 端点与 WebUI 日志流（P14 + P16）
0fb10d4  feat: oss 三后端（0~3）与 botstats 生产接线（P13 + P12b）
f85a9e4  feat: 图床接线（P11）与 /metrics + botstats（P12）
6b8a7cb  feat: parity 审计后修复波次——连接/入站/出站/WebUI/迁移全面对齐
2c7f058  feat: idmap legacy-export 导出器（G1）
5cca076  docs: #70 网关收尾文档 + parity 审计状态更新
2fcb05e  chore: Cargo.lock 补 gensokyo-actions 的 tokio 依赖边
9e71c31  feat: 下游分发接线——反向 WS action 派发 + 正向 WS 事件下发（P4）
e1f822c  feat: get_group_list 逐群节流（P2）
3fcb426  feat: 多媒体段与卡片+媒体拆分发送（P1）
dfee81b  feat: QQ 上游事件 WebSocket 网关客户端（#70）
feec3c8  docs: Go-Rust 全链路 parity 审计总表（A/B/C 三分域，含缺口与优先级）
8c69129  feat: SSM 22009 补发（#65）
34cc131  test: 运行时测试注入临时 idmap，消除 cwd redb 污染（#69 修复）
83fd328  feat: 出站懒消息 / "2000" 主动推送 / msg_seq 解析（#66b）
d94704e  docs: rust-m0 里程碑回写 R4b 波次（MSRV/金样/热重载/markdown素材链/WS缓冲/拆分/echo+idmap接线）
b3612c9  chore: gitignore 兜底忽略 idmap redb 测试库
ead507c  feat: idmap 生产接线（#69）
90abe64  feat: EchoStore 生产接线与入站喂池（#66a）
ad7cd10  feat: markdown 与 CQ 媒体同条时按 Go 口径拆分发送（#64）
5fe3e86  feat: 反向 WS 写失败缓冲（对齐 Go sendFailures/processFailedMessages）
5d1d19f  test: 入站金样回放（15 类事件 Go↔Rust 对齐）与 hash_id vUIN 支持
0f42d6e  feat: markdown 正文本地图经官方素材链替换（#62，对齐 Go ResolveMarkdownImages）
1d87aba  feat: 接线配置热重载（ConfigWatcher）到运行时启动链
89befb6  test: 黄金入站样本生成器（goldengen build tag）与 15 类 fixture（time 归一化、复跑 SHA256 一致）
74bd69c  ci: 新增 MSRV job 钉 1.90；reqwest default-features 上提根 manifest（成员覆写在 Cargo <=1.98 拒绝解析）
18f6894  docs: actions/lib.rs 媒体上传与 echo 回写表述按 deferred 现状更正
19669c9  docs: rust-m0 未建清单按代码现状对账（热重载/idmap/echo/四连接/媒体/WebUI 已建，余 MSRV job 与黄金样本回放/M4）
4f3836f  docs: R4a 与素材 DTO 修复回写里程碑与审计缺口表述
da3fe3c  feat: wakeup 发送终局回推 c2c_wakeup_resp notice（R4a）
ab79a50  fix: 素材管理 DTO 数值字段兼容官方字符串下发
fbe28a7  docs: R3 里程碑收尾——rust-m0 回写 + 官方 API 审计接线对账
c6b44a2  feat: 出站发送链接线——raw/wakeup/sse/合并转发（R3b）
bbee6d6  feat: get_group_list 真实化——idmap 枚举 + 逐群信息聚合（R3a）
a60c4ed  feat: Rust 端点补齐与出站段类型放开（M3）
8e7daa1  fix: 短签名图床外链不得进入出站与对外接口，回退本地公网 URL
4cd83d9  fix: xtask 跨目标 skip 提示带 --toolchain，0 产物给出可操作指引
9d9e254  fix: build.ps1 自动选择 Rust 工具链，无 MSVC 时回退 GNU
5692f51  fix: ChatGLM 短签名图床自取转传，修复 QQ 图片不显示
6decc56  fix: 仓库自带 webui/dist 占位文件，clean clone 可直接构建 Go 与 Rust
26dd81b  docs: 图床实测状态标注（2026-11-07）
0c48237  docs: 站点审计回改——wakeup/cq-active/card 口径与代码对齐
dc838ae  feat: CQ 码解析全路径最大兼容统一
f9eb538  feat: CQ 码参数空白容错全量适配
6bb967e  docs: 审计报告补记 CQ at 参数顺序的已知差异
67f6522  fix: 统一 CQ at 段的空白容错与非法段口径
0a95b52  docs: 入库 QQ 官方能力全量复核报告
d00a5ff  feat: 对齐 QQ 官方 2026-10 三条新能力并完成双侧全量复核
5391096  docs: 性能对比改为 10 轮均值与标准差
f2c882c  test: action 数量断言改为动态
789575a  feat: deferred 媒体上传/reply/echo 与 POST / 修复（M1r）
c0fdf21  feat: 剩余 action 与面板/互动占位（M1q）
9992a9b  feat: markdown/card/stream 出站模型（M1p）
2c3e0a6  docs: 补充 Rust/Go 同负载性能对比
f0c6537  ci: 占位 index 含 WebUI 字样，匹配运行期断言
982768f  feat: bin 装配 QQ 客户端与 47 actions
eb6bf3f  ci: 构建前创建 webui/dist 占位；补性能优化记录
8199231  feat: gRPC IDMapService（tonic/prost）
d6e225a  feat: action 全量注册与 deferred 发送执行（M1o）
6200e66  feat: 入站 12 类事件与 text_intent（M1n）
38ca39d  feat: action 批次二与 async 变体（M1m）
b0714e9  feat: silk 编解码外部进程封装（M2d）
3366503  feat: 再实现 12 个常用 action（M1l）
e5247cf  perf: 事件反序列化手写实现（-40%）
56d6de4  build: 构建链 Rust 化（xtask build + build.ps1 薄包装）
074708c  fix: 健康检查改用 /health
75640c6  feat: WebUI 挂载与 /health（M3 集成）
b5cf811  feat: 6 个查询/管理 action 与 QqApi 抽象（M1k）
700d689  feat: 图片压缩与 image 特性收窄（M2c）
f4fafa4  perf: CQ 解析热路径优化（约 2x）
63bfc7e  feat: WebUI 静态资源与 cookie 登录（M3）
69421a6  refactor: Handler 异步化并移除 spawn_blocking
a696bb6  perf: 发布配置与热路径基线
a753da2  feat: 二进制装配与 CLI（M1j）
83143bd  feat: 反向 WS/webhook/入站事件转换（M1h-b）
8e8f0c7  feat: 图片尺寸与图床抽象（M2b）
28e19cb  feat: 文本类出站 action 规划层（M1i）
28d9e79  feat: 运行时正向 WS/HTTP 与 dispatcher（M1h-a）
3e71afd  feat: 媒体基础能力 M2a（下载/解析/格式识别）
09946d2  feat: QQ OpenAPI 客户端（M1g）
39749cc  feat: OneBot V11 协议层（M1f）
68c6054  fix: echo 清理保留懒消息表并解除引用环
fb92211  fix: 修复 macOS 下配置监听路径不匹配
ec73c74  feat: echo 内存层与消息行号算法（M1e）
8cb5d2c  feat: redb 存储层与 idmap 本地核心（M1d）
7b44650  feat: 配置热重载与模板生成（M1c）
6c5f247  feat: 配置访问器与宽容解析补齐（M1b）
bd14e36  feat: 配置层全量对齐 Go Settings（M1a）
4b64df6  feat: 引入 Rust 重写 M0 脚手架
68efccd  chore: 升级漏洞依赖与工具链
4977720  fix: 修复 wsclient Close/Reconnect 竞态（send on closed channel）
86d9415  docs: CHANGELOG_v016 补本轮提交记录
eca1843  fix: 官方口径再对齐（token 域名 / 5M 直传阈值 / dto.User 字段）
8f3cfdb  docs: 修复 3 处失效相对链接（语法参考 cq-set_group / API介绍 本版新增功能 / 入群申请列表 readme）
c1c6470  feat: 对齐官方 v2 现文档（群管理契约 / 入站事件字段 / 5MB 直传 / 错误码）
bd73b1f  feat: 生产主链切换到 internal/+adapter 分层架构，新增 arch_mode 三态开关
1e2691e  fix: 私聊 markdown 内嵌 keyboard 补齐 permission.type=0→2 兜底
b4519d3  docs: readme 顶部小图标的引用仓库改为本仓库
d719aef  docs: AGENTS.md 补充发版与 CHANGELOG 惯例及 Windows 提交编码提示
```

# QQ 官方能力全量复核（2026-10 快照）

> **⚠️ 时效声明（P65 · 2026-11-14 追加）**：下文「裁决汇总」「未到里程碑 / 永久占位」两表的计数是 `d00a5ff` 时点（2026-11-08 订正节回写之前）的快照，原文保留仅为留痕，**现状以「R3 接线回写」一节 + 当前代码为准**。
> 本会话逐项实测：原「未到里程碑 **28**」中 27 项已接线（M3 22 项 + R3b 5 项），唯一未移植的 `get_qq_profile` 已由本文真缺口表的 `get_qq_profile` 行裁决「明确不做」⇒ **真缺（注册集口径）= 0**；原「发送链未接线 **5**」5 项全部已接。依据见各表行内标注。
> 唯一行为不对等项为 `get_friend_list`（Go `handlers/get_friend_list.go:29-47` 本地 idmap 活实现回 `retcode=0`；Rust 此前回 `1500`），被本快照误归入「官方无端点 16 项」。**定稿（P64a，2026-11-14）**：读侧已对齐——Rust 现照 Go 枚举 idmap `user_info` 表（key 字节序升序、滤空 `user_id`、恒 `retcode=0 status="ok"`、空表/出错/未装配为 `"data":null` 而非 `[]`），归类改判「本地 idmap 活实现」；**写侧 `StoreUserInfo`（Go 在入站 C2C 处理时写该桶，`ProcessC2CMessage.go:231-241`）属 P64b 待做**——Rust 的 `user_info` 表当前只有 M1e 导入器数据，全新 Rust 部署在导入前该接口恒回 `data:null`（是实现缺口，不是功能缺陷）。详见 `docs/rust-m0.md` R4o。

## 元信息

| 项 | 值 |
|---|---|
| 复核日期 | 2026-10-31 ~ 2026-11-01 |
| 分支 | `Refactor` |
| 对应提交 | `d00a5ff`（`feat: 对齐 QQ 官方 2026-10 三条新能力并完成双侧全量复核`） |
| 接线回写 | `a60c4ed`（M3）/ `bbee6d6`（R3a）/ `c6b44a2`（R3b），2026-11-07；见「R3 接线回写」节。快照本体（元信息 ~ 口径不一致 7 条）仍按 `d00a5ff` 时点陈述，不改写 |
| 官方文档快照 | `https://bot.q.qq.com/wiki/develop/api-v2/`，变更记录页末次更新 **10/28/2026** |
| 复核对象 | Go 现役链（`handlers/`、`botgo/`、`Processor/`）与 Rust 新链（`crates/`） |
| 性质 | 只读复核 + 差异裁决；本文档不改代码，交叉引用已落地的实现 |
| 留存 | 同轮的四份黑板文件路径见「证据索引」。黑板有 5 天 TTL，本报告是正式留存 |

## 范围与方法

复核的官方文档面：

- **服务端接口**：`/autogen/api/*`（2026-10-28 索引）+ 未收录进索引的正文页（素材管理、频道消息）。
- **事件**：`/autogen/event/*`（22 页）+ `/autogen/index.html` 事件表。
- **Intents**：`/dev-prepare/interface-framework/event-emit.html`（11 个位与订阅权限规则）。
- **错误码**：`/dev-prepare/api-call-guide.html`（9/17/2026）+ 各接口页「错误码」章节 + 频道区间族。
- **变更记录**：`/changelog.html`（10/28/2026），覆盖 20260916 ~ 20261028 五条新条目与窗口外背景条目。

取证方式与两个关键陷阱：

- 接口页 URL 规律为 `/autogen/api/<路径下划线化>.<method>.html`，事件页为 `/autogen/event/<事件名小写>.html`；**正确做法是先取 `/wiki/sitemap.xml`（423 条 loc，api-v2 下 192 条）再逐页抓**，靠 slug 猜测会漏面。
- 任何猜错的路径都会 HTTP 200 返回约 836 字的「启动接入」占位页；据此判定「URL 无效」而非「无权限」。首轮 41 次无效路径探测即由此产生，并导致频道消息页、Intents 总表、素材管理三处被误判为缺失，第二轮已订正。
- 只读代码取证：`git grep` 全仓 + 逐文件读；两侧能力清单由 Go `callapi.RegisterHandler` 与 Rust `KNOWN_ACTIONS` / `IMPLEMENTED_ACTIONS` 反查。

置信度分级（全文沿用）：

| 级别 | 定义 |
|---|---|
| **High** | 官方正文原文直引（含完整字段表或错误码表） |
| **Medium** | 官方页摘要、索引级条目，或多页交叉引用但无完整字段表 |
| **Low** | 推断，不得据以判定实现缺失 |

统计口径订正：首轮记「33 端点 / 24 事件」是 `/autogen/index.html` 的**表格行数**，不是页面数；sitemap 实测为 **47 端点 / 22 事件**，本报告一律用后者。

## 覆盖面统计

| 维度 | 官方规模 | 取证级别 |
|---|---|---|
| 服务端接口 | **47** 页（sitemap `autogen/api/*`），另有 2 个不在 autogen 下的正文页 | 矩阵共 37 行：High **28** 行、Medium **8** 行、混合 1 行（频道详情，路径 High / 字段 Medium）；素材管理与频道消息另补 6 页 High |
| 事件 | **22** 页（`autogen/event/*`） | High 4（`GROUP_AT_MESSAGE_CREATE`、`GROUP_MESSAGE_CREATE`、`C2C_MESSAGE_CREATE`、`GROUP_JOIN_REQUEST`），其余 18 条 Medium |
| Intents | **11** 个位 | event-emit 页 High；`GROUP_MEMBER_EVENT(1<<24)` 不在总表内，按事件页取用，记 Medium |
| 错误码 | **40+** 具体码 + 7 个区间族（频道/发消息/编辑消息等） | api-call-guide 9/17/2026 High |
| 变更记录 | 5 条（20260916 ~ 20261028） | `/changelog.html` 10/28/2026 High |

覆盖到的官方新条目：20261021 素材管理、20261026 群聊 @全员、20261028 图文卡片、20260916 分享链接字段修正与 `ark_type=picture`。背景条目（20260810 群管理/禁言/入群申请、20260812 菜单与指令面板、20260903 群成员管理 5 接口）一并核对。

## 裁决汇总

| 裁决 | 数量 | 处置 | 落地状态（`d00a5ff`） |
|---|---|---|---|
| 真缺口 | **4** | 3 修 + 1 明确不做 | 素材管理 / @全员 / 单聊图文卡片**已修**；`GET /users/@me` 明确不做 |
| 口径不一致 | **7** | 全修 | 7 条**全部已修**，逐条见「口径不一致 7 条」 |
| 未到里程碑（Rust 端点未移植/未接线） | **28** | 不动 | **未修**：`gensokyo-qq` 尚无对应端点是本轮范围外 **【快照失效，见下文『未到里程碑 / 永久占位』节首订正与『R3 接线回写』节】28 项中 27 项已接线**：M3 22 项 + R3b 5 项（落点 `panels.rs:5-21`、`interaction.rs:6-25`、`group_admin.rs:10`、`messages.rs:7`）；余 1 项 `get_qq_profile` 属上文真缺口表「明确不做」⇒ 真缺 0 |
| 官方无端点永久占位 | **16** | 文档登记 | **未修**：官方无对应能力，两侧行为一致 **【快照部分失效，见下文『R3 接线回写』节 R3a 行】** 群列表已由 R3a 改判「已实现聚合」；`get_friend_list` 的「两侧一致」原不成立（Go 活实现 vs Rust 曾 `1500` 占位），**P64a 读侧已对齐并移出本集合**（现照 Go 枚举 idmap `user_info`，空态 `"data":null`/恒 `retcode=0`，见 `docs/rust-m0.md` R4o；写侧 `StoreUserInfo` 属 P64b）⇒ 现值 16→**15**（纯官方无端点） |
| OneBot 兼容空实现 | **5** | 不动 | **未修**：纯兼容回执，与 Go 语义一致 |
| 附：固定 `retcode=100` | 2 实现 / 3 名 | 不动 | **未修**：`set_group_whole_ban`/`get_group_whole_ban`、`set_group_card` 官方无写接口 |

四条真缺口的逐项处置：

| 真缺口 | 官方出处 | 裁决 | 状态 |
|---|---|---|---|
| 素材管理端点族（5 端点） | `material-management.html` 10/21/2026 **High** | 两侧全缺，按完整契约补齐 | 已修 |
| 群聊 @全员 | `text-chain.html` 10/26/2026 **High**（规则）/ **Low**（错误码） | 两侧 0 命中，按标签 + 自然日限流补齐 | 已修 |
| 单聊图文卡片 | `v2_users_user_openid_messages.post.html` 10/28/2026 **High** | Go 仅群聊、Rust 无出站，补单聊 | 已修 |
| `GET /users/@me` | `users_me.get.html` **High**（字段索引级） | 影响面小，`get_login_info` 已由配置覆盖 | **未修**（明确不做） |

## 真缺口与已修项明细

### 素材管理（官方 2026-10-21）

- **官方口径**：`POST /v2/media-images/prepare` → `PUT <presigned_url>`（直传 COS，不带 `Authorization`，`Content-Type` 须与 prepare 一致）→ `POST /v2/media-images/complete`；另有 `GET /v2/media-images`、`DELETE /v2/media-images/{image_id}/delete`。限制：单图 ≤10MB、png/jpeg/gif/webp、单机器人 100 张、90 天滑动过期、`upload_id` 4 小时、错误码 40035001/2/3/6/7/8/9/10。出处 `/server-inter/message/material-management.html`（页脚 10/21/2026 19:35:14）｜**High**。
- **Go 落点**：`handlers/media_images.go:26-29`（三个 action 注册）、`:34`（10MB 十进制上限）、`:417`（裸 PUT）、`:408`（ETag 去引号）、`:442`（400350xx 原文透传）；DTO 与客户端 `botgo/dto/media_image.go`、`botgo/openapi/v2/media_image.go`。`qqbot-media://` 在 Markdown 中透传，不再进本地路径分支：`handlers/message_parser.go:2814-2816`。
- **Rust 落点**：`crates/gensokyo-qq/src/media_image.rs`（prepare/complete/list/delete + `put_presigned`）、`crates/gensokyo-actions/src/media_image.rs`、注册见 `crates/gensokyo-actions/src/register.rs:86`、`:152`、`:279`。
- **验收证据**：`handlers/media_images_test.go:104`（prepare 字段）、`:178`（10MB 边界）、`:215`（格式/扩展名）、`:265`（400350xx 映射）、`:369`（Markdown 透传）、`:384`（ETag 去引号）、`:424`（shadow 不碰上游）；Rust `crates/gensokyo-actions/src/media_image.rs:613`（prepare→PUT→complete 顺序）、`:688`（非法文件形态）、`:779`（空列表不 panic）。

### 群聊 @全员（官方 2026-10-26）

- **官方口径**：标签 `<qqbot-at-everyone />`（自闭合），**仅群聊**，文本（`msg_type=0`）与 Markdown（`msg_type=2`）两路支持；机器人须为群管理员，**每群每自然日 1 次**。出处 `/server-inter/message/trans/text-chain.html`（页脚 10/26/2026 17:41:55）｜规则 **High**；错误码官方未给，记 **Low**（推断，不得据以判定）。
- **Go 落点**：`handlers/at_everyone.go:23-35`（标签与 CQ 正则，`qq=all|everyone`，大小写不敏感）、`:50`（`NormalizeAtEveryone`）、`:171`（`GuardAtEveryone` 占位 + 失败回滚）；归一挂在 CQ 管线 `handlers/cqcode_pipeline.go:30`、`:34`，文本与 Markdown 同源；开关 `config/config.go:2314`、`structs/structs.go:169`、`template/config_template.go:210`（默认 `true`）。
- **Rust 落点**：`crates/gensokyo-runtime/src/at_everyone.rs:46`（标签）、`:71`（`normalize`）、`:130-133`（`at_mention_id`）；出站侧卡片与 @全员同现回 `1400`（`crates/gensokyo-runtime/src/deferred.rs:362-366`）。
- **验收证据**：`handlers/at_everyone_test.go:17`、`:40`、`:122`（同日第二次 1400）、`:140`（跨日放行）、`:152`（回滚释放额度）、`:168`（开关关闭不拦）、`:184`（shadow 不计数）、`:204`（固定 +08:00 切日）、`:322`（单聊拒绝）、`:364`（发送失败回滚）；Rust `crates/gensokyo-runtime/src/at_everyone.rs:373`~`:572`（归一、时区、并发占位、回滚、脏数据容错）。

### 单聊图文卡片（官方 2026-10-28）

- **官方口径**：`msg_type=8` + `card: {type:"tuwen", content:{...}}`，群聊与单聊同构；四字段必填：`title` 1–50 字符、`description` 1–200 字符、`url` 与 `pic_url` 须 https 且 ≤1024 字节。出处 `/autogen/api/v2_groups_group_openid_messages.post.html`、`.../v2_users_user_openid_messages.post.html`（索引 10/28/2026）｜**High**。
- **Go 落点**：`handlers/card_payload.go:29`（`buildTuwenCard` 单一校验实现）、`:132`（`applyCardToMessage`）；单聊新增 `handlers/send_private_msg.go:57-72`（`applyPrivateCard`，`MsgType=8` 且清空正文），调用点 `:591`。
- **Rust 落点**：`crates/gensokyo-runtime/src/deferred.rs:241-250`（markdown 胜 card、card 独占正文）、`:358-379`（`compose_card` 四字段校验）；DTO `crates/gensokyo-qq/src/dto/message.rs`。
- **验收证据**：`handlers/card_payload_test.go:74`（50/51 边界）、`:88`（200/201）、`:101`（非 https）、`:125`、`:170`（1024 字节边界）、`:139`（四字段缺一）、`:201`（群聊 `msg_type=8`）、`:235`（**单聊** `msg_type=8`）、`:359`（与 @全员同现 1400）、`:391`（markdown 优先）；Rust `crates/gensokyo-runtime/src/deferred.rs:1208`（群与私聊均出 `msg_type=8`）、`:1147`（markdown 只带 content）。

### 既有缺口修复（随本轮一并落地）

| 事项 | 官方出处与置信度 | 修前 | 落点与验收 |
|---|---|---|---|
| Rust 缺 `INTERACTION_CREATE` 入站 | `/autogen/index.html` 事件表 10/28/2026 **Medium**（索引级） | Go 有、Rust 14 类事件无此条 | `crates/gensokyo-runtime/src/inbound.rs:299`、`:788-813`（转 `notice.interaction`）；测试 `:1021` |
| Rust 群成员列表无分页 | `v2_groups_group_openid_members.get.html` 9/3/2026 **High**（每页 ≤30、顶层 `next_cursor`） | 单页、无 `cursor` 入参 | `crates/gensokyo-actions/src/group.rs:290-321`（透传 cursor、回顶层 `next_cursor`）；测试 `:977` |
| 禁言未截断到 30 天 | `...restrict_chat_setting.post.html` 9/3/2026 **High**（最长 30 天） | Go 直接透传上限值；Rust 同 | Go `handlers/set_group_helpers.go:50-57`；Rust `crates/gensokyo-actions/src/group.rs:74-76`、`:335`；测试 `handlers/at_everyone_test.go:347` |
| Rust 直传阈值二进制口径 | `/autogen/api/v2_groups_group_openid_files.post.html` **High**（5MB）/ 进制 **Medium** | `5*1024*1024`，5,000,001~5,242,880 区间被上游拒且无分片退路 | `crates/gensokyo-runtime/src/media.rs:33`（`5_000_000`）；测试 `:203` |

### 连带修复（非官方差异项）

- **删除 `custom_template_id` 自动注入**：消息接口页标注模板字段「【已废弃】」（`/autogen/api/v2_groups_group_openid_messages.post.html`，裁决见 02 §5，**High**）。修前 Go 按配置自动注入模板 ID；修后只构造 `markdown.content`，字段仅保留反序列化兼容（`handlers/message_parser.go:2610`、`:2907`）。
- **shadow dry-run 不产生副作用**：`arch_mode=shadow` 时新链只做 diff，不再触发额度占用与素材上传（`handlers/outbound_bridge.go:179-182`）；验收 `handlers/media_images_test.go:424`、`handlers/at_everyone_test.go:184`。

## 口径不一致 7 条

| # | 项 | 官方口径（出处 / 置信度） | 修前 | 修后 |
|---|---|---|---|---|
| a | `file_data` 直传 5MB 进制 | 接口页只写 5M，未定义进制（**High** 上限 / **Medium** 进制） | Go 十进制正确；Rust 二进制 `5*1024*1024` 且超限直接 1400 | 两侧统一 `5_000_000`（`crates/gensokyo-runtime/src/media.rs:33`） |
| b | 出站 @ 用户 | 群聊 `<qqbot-at-user id="" />` **仅对 Markdown 生效**，文本不支持（`text-chain.html` 10/26/2026 **High**） | Rust 一律旧式 `<@openid>`（官方标注即将弃用） | Rust Markdown 产出 `qqbot-at-user`（`deferred.rs:313-346`）；文本保留旧式，属唯一允许的分叉 |
| c | deferred 明拒段 | `msg_type` 0/2/7/8 均可发（消息接口页 **High**） | Rust 对 markdown/keyboard/card/music/stream/input_notify 一律 1400 | 放开 markdown 与 card（`deferred.rs:16-18`）；keyboard/music/stream/input_notify 仍 1400（`:20`、`:587-590`） |
| d | `INTERACTION_CREATE` | 事件表 10/28/2026 **Medium**（索引级，可作 `event_id` 被动回复） | Rust 无入站 | `inbound.rs:299` 转 `notice.interaction` |
| e | 成员列表分页 | 每次最多 30 条 + `next_cursor`（9/3/2026 **High**） | Rust 单页、无 cursor 入参 | `group.rs:290-321` 透传 cursor 并回 `next_cursor` |
| f | `template_id` / `custom_template_id` | 消息接口页标注「【已废弃】」（02 §5 **High**） | Go 按 config 自动注入模板 ID | 只构造 `content`，字段仅解析兼容（`message_parser.go:2610`） |
| g | 卡片四字段校验 | `title` 1–50 / `description` 1–200 / `url`、`pic_url` https ≤1024（10/28/2026 **High**） | Go 三处只判 `pic`/`url` 非空；Rust 无发送路径 | Go 收敛到 `buildTuwenCard`（`card_payload.go:29`）+ 单聊路径；Rust `compose_card`（`deferred.rs:358-379`） |

## 未到里程碑 / 永久占位

Rust 侧 44 项硬占位（`retcode=1500`）的拆分，**28 项属未到里程碑，16 项属官方无端点永久占位**，两者都不是本轮要补的缺口：

> **2026-11-08 订正**：本节为 `d00a5ff` 时点口径。M3（`a60c4ed`）已接线原「端点未移植」中的 22 项、R3a（`bbee6d6`）已聚合实现群列表、R3b（`c6b44a2`）已接线原「发送链未接线」5 项；现状清单以「R3 接线回写（2026-11-08）」一节为准。

| 分组 | 数量 | 代表项 | 为什么不算缺口 |
|---|---|---|---|
| 端点未移植 | **23** | `set_group_kick`、`delete_msg`/`delete_group_msg`、`put_interaction`、`get_robot_share_link`、`set_group_add_request`、黑名单两枚、`get_qq_profile`、面板 6、菜单 2、审批策略 6 | `gensokyo-qq` 未暴露对应端点（`crates/gensokyo-actions/src/qqapi.rs`、`panels.rs`、`interaction.rs` 自述），属新链移植进度。**【快照失效，见下文『R3 接线回写』节 M3 行】** 本行 23 项中 22 项已由 M3（`a60c4ed`）接线（`panels.rs:5-21` 面板 6 + 菜单 2 + 审批策略 6、`interaction.rs:6-25`、`group_admin.rs:10` 踢人/黑名单/入群申请、`messages.rs:7` 撤回）；余 `get_qq_profile` 属上文真缺口表「明确不做」 |
| 发送链未接线 | **5** | `send_group_forward_msg`、`send_private_forward_msg`、`send_group_msg_raw`、`send_private_msg_sse`、`send_private_msg_wakeup` | 端点已在 Go 侧验证可用，Rust 仅缺 deferred 链分支。**【快照失效，见下文『R3 接线回写』节 R3b 行】5 项全部已接线（R3b `c6b44a2`）**：`send_group/private_forward_msg` → `dispatcher.rs:231` → `deferred.rs:773 execute_forward`；`send_private_msg_sse` → `dispatcher.rs:245` → `deferred.rs:728`；`send_group_msg_raw` → `interaction.rs:19`；`send_private_msg_wakeup` → `deferred.rs:263-304` + R4a `publish_wakeup_notice`（`deferred.rs:674`，见下文『已知缺口与刻意差异』第 1 条） |
| 官方无端点（**不在这 28 项内**） | **16** | 退群、群列表、`get_msg`、合并转发查询、好友列表、陌生人资料、点赞、群荣誉、匿名禁言、@all 剩余次数、群名/管理员/头衔/备注、机器人资料写、群系统消息 | 官方无对应能力，永久占位；Go 侧同样降级或固定回执。**【快照部分失效，见下文『R3 接线回写』节 R3a 行】** 代表项「群列表」已由 R3a 移出本集合；「好友列表」亦已由 P64a 读侧对齐移出（Rust 现照 Go 枚举 idmap `user_info`，空表/出错/未装配恒 `retcode=0` + `"data":null`；写侧 `StoreUserInfo` 属 P64b，见 `docs/rust-m0.md` R4o）——现维持 1500 / 固定回执的清单见下文『官方无端点 · 维持 1500 / 固定回执（现值）』表，**P64a 后现值 15** |
| 固定 `retcode=100` | 2 实现 / 3 名 | `set_group_whole_ban`/`get_group_whole_ban`、`set_group_card` | 官方 `restrict_chat_setting` 只能按成员写，无全员禁言与群名片写接口 |
| OneBot 兼容空实现 | **5** | `set_restart`、`clean_cache`、`get_cookies`、`get_csrf_token`、`get_credentials` | 纯兼容回执，与 Go 一致，不产生上游调用 |

计数订正：`d00a5ff` 新增的三个素材管理 action 全部有真实上游，**不占**上述 44 项，且该提交未从占位集合中移除任何一项（改动文件不含 `unsupported.rs` / `panels.rs` / `group_admin.rs` / `interaction.rs`）。双链注册表因此为 Go `56` 个 action 名（复核时 53 + 3）、Rust `KNOWN_ACTIONS` 56、`IMPLEMENTED_ACTIONS` 75（72 + 3）。

## R3 接线回写（2026-11-08 · `a60c4ed` / `bbee6d6` / `c6b44a2`）

本节按代码现状登记接线进度，不改写上文快照（快照口径见「未到里程碑 / 永久占位」节首订正说明）。计数与形态均以 `crates/` 源码逐条核对。

### 官方有端点 · 已接线

| 批次 | 数量 | action（逐一核过的接线形态） | 落点 |
|---|---|---|---|
| M3（`a60c4ed`） | **22** | 面板 6（`create_panel` / `set_panel` / `delete_panel` / `get_panel` / `get_panel_list` / `set_panel_target` → `/v2/panels*`）+ 自定义菜单 2（`set_custom_menu` / `get_custom_menu` → PUT/GET `/v2/menu`）+ 入群自动审批策略 6（`join_approval_strategy_*` → `/v2/groups/join_approval_strategy*`）+ 群黑名单 2 + `set_group_add_request` + `delete_group_msg` + `put_interaction` + `get_robot_share_link` + `set_group_kick` + `delete_msg` | `panels.rs:5-21`、`interaction.rs:6-25`、`group_admin.rs:10`、`messages.rs:7` 模块表；判据 = a60c4ed diff 中被移除的占位调用与 8e7daa1 模块表比对（panels.rs 14 + interaction.rs 6 + 踢人 1 + 撤回 1）。提交标题口径「20 个」与 `qqapi` trait 新增 20 方法计数一致，按 action 逐项核为 22，属口径差非缺实现 |
| R3a（`bbee6d6`） | **1** | `get_group_list`：官方无群列表端点，照 Go 口径实现——idmap `find_config_sections` 枚举 `<上报形态ID>:type=group` + 逐群 `GET /v2/groups/{gid}/info` 聚合；反查 / 信息 / 正查任一失败整群跳过并 warn | `group_admin.rs:12,36-44`、`gensokyo-storage/src/idmap/mod.rs`（`find_config_sections`）、runtime 入站 4 群事件注册点 |
| R3b（`c6b44a2`） | **5** | `send_group_msg_raw`（按 Go 实际=普通解析链，非原生体直传）、`send_private_msg_wakeup`（同 C2C 端点置 `is_wakeup`）、`send_private_msg_sse`（`msg_type=2` + `stream{state,index,id}`）、`send_group_forward_msg` / `send_private_forward_msg`（逐条重发） | `interaction.rs:18-22`、`send.rs`（plan_*）、`runtime/src/deferred.rs`（`execute` / `execute_forward` / `execute_sse` 三分支）、`dispatcher.rs` |

出站段类型随 M3 放开：`keyboard`（加性挂载，不改 `msg_type`）、`input_notify`（`msg_type=6`，仅单聊、独占整条）、`stream`（`msg_type=2` + `stream`，仅单聊）；`music` 降级为文本分享文案（Rust 侧无 QQ 音乐查询与 `music_prefix` 配置）——见 `runtime/src/deferred.rs:21-24`。`HttpMethod` 已补 PUT/PATCH/DELETE（`gensokyo-qq/src/http.rs:29-40`），`gensokyo-qq` 新增 22 个端点方法与 panel DTO。

### 官方无端点 · 维持 1500 / 固定回执（现值）

| 处置 | 数量 | action | 落点 |
|---|---|---|---|
| 参数校验后回 `1500` | **15** | `get_group_at_all_remain`、`set_group_name`、`set_group_admin`、`set_group_special_title`、`set_group_remark`、`set_qq_profile`、`get_group_system_msg`、`get_qq_profile`（`gensokyo-qq` 未暴露机器人资料端点）、`get_stranger_info`、`send_like`、`set_group_leave`、`get_msg`、`get_forward_msg`、`get_group_honor_info`、`set_group_anonymous_ban` | `unsupported.rs:5-16`、`users.rs:8-9`、`messages.rs:8-9`、`group_admin.rs:11`、`interaction.rs:23-24`；`get_friend_list` 已由 P64a 移出本表（读侧直读 idmap，`users.rs` 模块文档） |
| 固定 `retcode=100`（Go 同口径） | 2 实现 / 3 名 | `set_group_whole_ban`（含别名 `get_group_whole_ban`）、`set_group_card` | `group_admin.rs:8-9,18-23` |
| Go 的本地固定回执 | **1** | `get_online_clients` → `{"clients": [], "tiny_id": 0}`（逐字段照抄 Go） | `unsupported.rs:15,30-34` |
| OneBot 兼容固定回执 | **5** | `set_restart`、`clean_cache`（日志 + `{"code": 0}`，不真执行）、`get_cookies`、`get_csrf_token`、`get_credentials`（固定空值） | `system.rs:8-12` |
| 本地可判定（不触上游） | 4 实现 | `get_avatar`（URL 拼接）、`mark_msg_as_read`（空回执）、`can_send_image` / `can_send_record`（固定 `yes: true`） | `interaction.rs:8-9,28-34`、`capability.rs` |

`d00a5ff` 时点的 44 项硬占位中，未到里程碑的 28 项去向如下：M3 接线 22（原「端点未移植 23」中除 `get_qq_profile` 外全部落地，`set_group_kick` / `delete_msg` 在 8e7daa1 时点亦为 1500，同轮转真实）、R3b 接线 5、R3a 把 `get_group_list` 自「官方无端点 16」改判「已实现聚合」（官方本无列表端点）。维持 `1500` 的合计因此为 **16**（官方无端点 15 + 未移植 1），见下表。

### 已知缺口与刻意差异（本轮新增登记）

1. **wakeup 的 `c2c_wakeup_resp` notice**：~~未接线，待 R4 立项~~ **已接线（R4a `da3fe3c`）**——wakeup 终局向 `EventHub` 推 notice，JSON 逐键对齐 Go `sendWakeupNotice`（`handlers/send_private_msg_wakeup.go:343-375`）；残留备案差异：`error_msg` 为 Rust `Error::Display` 文本非 Go 原文；forward 计划带 wakeup 标记按节点各推一次（Go 无该组合，自创口径，生产规划层当前不可达）。
2. **`get_group_list` 不回捞 uin 前缀形态**：`idmap_isolation` 开启且与 Go 共库时，Go 写入的 `"<uin>:<原始串>"` section 既非 32 位也 parse 不成数字，按反查失败整群跳过；Rust 入站链注册的是不带前缀的原串，故该形态只出现在混合部署里。刻意差异同记：**Go 留空字段行、Rust 整群跳过**（`group_admin.rs:36-44`）。
3. **wakeup 空 `message` 收紧为 `1400`**：Go 侧无此前置校验（`handlers/send_private_msg_wakeup.go:63-70` 直接进 `parseMessageContent` → 发送路径），Rust 在参数层直接拒——与 Go 回执**方向相反**的刻意收紧（`send.rs:57`，`c6b44a2` 评审修复轮登记为刻意差异）。
4. **合并转发保留 Go 的 `limit+1` off-by-one**：Go 循环条件 `count > limit` 且仅在形状合格的 node 上计数，上界实际是 `limit+1` 条；行为对齐优先于修它（`send.rs:562-567`）。附带口径：`failed[].index` 用原 node 序号、坏 / 空 node 计入失败清单不静默丢、500ms 间隔经 `DeferredDeps::forward_send_interval` 可注入（`deferred.rs:127-129,202-206,244-256`）。
5. **`forward` / `message_sse` 载荷新握手键全部 `serde(default)`**：旧载荷反序列化零破坏，有回归用例钉住（`c6b44a2` dispatcher 改动）。

### 注册表计数复核（现值，动态断言）

| 项 | 现值 | 判据 |
|---|---|---|
| `KNOWN_ACTIONS` | **56** | `registry.rs:150-207` 数组实测 56 项；`register.rs:384` 保留字面量钉值 `assert_eq!(KNOWN_ACTIONS.len(), 56)` 作全集 tripwire |
| `IMPLEMENTED_ACTIONS` | **75** | `lib.rs:173-249` 实测 75 项；与 KNOWN 交集 50，`EXTENSION_ACTIONS` 25（`lib.rs:31-32`） |
| 动态断言（`f2c882c`「action 数量断言改为动态」） | 在位 | `register.rs:368-379`（`IMPLEMENTED_ACTIONS.len() + ACTION_ALIASES.len()`）、`startup.rs:505`（`+3+3`）、`runtime/tests/deferred.rs:194`、`registry.rs:366`（`KNOWN_ACTIONS.len() - 1`） |
| Go 主仓 `callapi.RegisterHandler(` 实测 | **53**（HEAD `cfeb03b`） | `git grep -h -F 'callapi.RegisterHandler(' -- handlers` 字面名去重 = 53（须 `[Console]::OutputEncoding=UTF8` 后统计：GBK 解码会把含中文注释的行吞并换行、少计 2 名）；另有 2 处 `RegisterHandler(action, …)` 在 `handlers/zzzz*_test.go` 注册测试局部名，非产品 action，不计。相对 KNOWN 缺的正是素材三枚 `upload_media_image` / `get_media_image_list` / `delete_media_image` |
| 与 KNOWN 口径关系 | **53 + 素材 3 = 56 成立** | Refactor 工作树 `handlers/` 实测 56（`media_images.go:27-29` 注册三枚素材 action），与 KNOWN 逐项 diff 双向为空；Go 主仓当前把素材链收敛为 `media_asset.go` 内部流程、未注册 action 名（`handlers/media_asset.go:19` 注释自述「将来若要曝光 upload_media_image，复用 mediaImageUploadFlow 即可」）——属双侧部署形态差异，**注册表不改**，此处只登记口径 |


## 遗留风险与已知限制

1. **带空格的 `[CQ:at, qq = all]` 已统一（2026-11-01）**：Go 的 `atEveryoneCQPattern` 与 `cqAtNumRe` 已改为容忍 `qq`、`=`、取值两侧空白；Rust 的 `at_mention_id` 本就对键值做 `trim`。两侧现均归一为标签并按同日额度计一次（`handlers/at_everyone.go:33`、`handlers/message_parser.go:55`、`crates/gensokyo-runtime/src/at_everyone.rs:134-148`）。
2. **缺 `qq` 的 at 段已统一（2026-11-01）**：契约定为「Markdown 内 `[CQ:at]` / `[CQ:at,foo]` / `[CQ:at,qq=]` → `1400`；普通文本原样透传」，Go 在 `parseMarkdownFromMessageE` 统一校验（`handlers/message_parser.go:3145-3149`），Rust 在 `render_markdown` 报错、`render_text` 透传（`crates/gensokyo-runtime/src/deferred.rs:393-428`）。
   - `qq` 非首参写法（`[CQ:at,name=x,qq=123]`）**已于 2026-11-03 统一**：Go 改按键取值（`cqAtSegRe` + `cqAtSegmentQQ`），与 Rust 同口径正常解析；不再判 malformed。
3. **CQ 参数空白容错已全量适配（2026-11-03）**：全部 CQ 段类型双侧统一——段名后/参数间/`=` 两侧容忍空白；词法切分空白集 = ASCII `{ ,\t,\n,\f,\r}`（同 Go 正则 `\s`），键/值首尾 trim = Unicode `TrimSpace` 语义（Go `cqCodeParam`/`buildParams` 对偶）；值内部空白保留。Rust `Segment` 挂 raw 原文，透传臂逐字节保真（`deferred.rs`/`outbound.rs`）；Rust parse 采用与 Go `scanSegments` 同款花括号配平，含逗号 JSON 不截断、配平失败不吞内嵌合法码（`cursor=start+1`）。详见 `release_log/CHANGELOG_v016.md`「CQ 码空白容错全量适配」。
4. **CQ 码解析全路径最大兼容统一（2026-11-04）**：上条「保留的既有结构差异」两项**已全部消除**——① Go legacy 媒体/markdown/keyboard 由位置锚定正则改为整码匹配 + `cqUnionParams` 按键取值（任意键序、尾键不污染值）；② 段名前空白四路统一识别，`errMarkdownAtMalformed` 下线。同轮统一：段名/键大小写不敏感（`[cq:` 前缀仍区分）、stream 冒号/等号双语法（冒号限 stream）、段名字符集 `[A-Za-z0-9_]`、Rust 裸尾段并入前一键、internal 新链补花括号配平状态机与 stream case、`CARDIGAN` 类前缀不吞段、`acnode` base64 豁免谓词适配宽松形态。详见 CHANGELOG「CQ 码解析全路径最大兼容统一」。
   - 记录的分层语义（非缺陷）：legacy 键值不做实体解码、`cq_parse_mode=new` 才反转义（模式设计本意）；`[CQ:reply,id=…,msg_idx=…]` 同现时整码按 reply 消费、msg_idx 随段移除（用例钉桩）。
   - 记录项：`adapter/identity`、`adapter/state`、`config`、`echo` 四包既有测试非幂等（`-count=2` 隔离复跑必红，`-count=1` 绿），属 TEST_DEFECT 待单独立项清理，与 CQ 改动无关。
5. **段类型现状（2026-11-08 更新，替代本条原「仍回 1400」表述）**：M3（`a60c4ed`）已放开 `keyboard`（加性挂载，不改 `msg_type`）、`input_notify`（`msg_type=6`，仅单聊、必须独占整条）、`stream`（`msg_type=2` + `stream`，仅单聊）；`music` 降级为文本分享文案（Rust 无 QQ 音乐查询与 `music_prefix` 配置，不伪造音乐卡片）。现状表见 `crates/gensokyo-runtime/src/deferred.rs:21-24`；markdown 与 card 的放开仍是 `d00a5ff` 本轮口径。仍与内容同条出现即 `1400` 的约束保留（不静默丢）。
6. **成员列表分页语义差异**：Go 循环拉取最多 100 页后合并返回（`handlers/get_group_member_list.go`），Rust 单页 + `next_cursor` 接力（`crates/gensokyo-actions/src/group.rs:36-39`）。同一 OneBot 调用的返回条数在两侧不同，客户端需按 `next_cursor` 自行翻页。
7. **shadow diff 长期非空**：新链与 legacy 的 `foundItems` 键集合历史差异导致 `[outbound-shadow]` 日志几乎每次都有差异（`handlers/outbound_bridge.go:323-340`），只能当噪声参考，不能作为回归判据。
8. **未做真实上游联调**：全部结论基于官方文档 + 本地单测（Fake 客户端），`40035010`（跨机器人引用）、`11253`（未获接口权限）等权限类错误未在真实机器人上验证。
9. **@全员 管理员身份不本地判定**：官方要求机器人为群管理员，本地不做前置校验（`handlers/at_everyone.go:21`），上游报错原样透传。
10. **时区注释已校正**：`crates/gensokyo-runtime/src/at_everyone.rs:30-32` 原先称 Go 用本地时区，现已改为「Go 固定 `time.FixedZone("CST", 8*3600)`」，与实现一致。
11. **@全员 计数存储不同源**：落在各自 idmap 的 config 表 `at_everyone`（Go bbolt / Rust redb），仅共用同一存储文件时互认（`crates/gensokyo-runtime/src/at_everyone.rs:20-22`）。

## 证据索引

| 命令 | 结果 | 说明 |
|---|---|---|
| `go build ./...` | 退出码 0 | Go 侧编译 |
| `go test ./...` | **24 包 `ok` / 0 `FAIL`**（另 19 个包无测试文件） | 本报告撰写时（2026-11-01）实测 |
| `cargo test --workspace` | 退出码 0，**683 passed / 0 failed**（33 个 suite） | 实测口径：`rustup run 1.99.1-x86_64-pc-windows-gnu cargo test --workspace`（PATH 前置 `C:\msys64\usr\bin`）；默认 MSVC host 缺 `link.exe` 属工具链选择问题，换 GNU host 即可 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 退出码 0，**0 warning** | 同上工具链实测（2026-10-31 ~ 11-01 两轮） |
| `cargo test --workspace`（R3 验收记录） | **33 suites / 770 passed** | M3 / R3a / R3b 合入时点的全量口径（`c6b44a2` 提交正文记 33 suites 全绿）；2026-11-08 文档回写轮只独立复跑 `cargo test -p gensokyo-runtime`（见本轮验收），全量计数以提交记录为准 |
| KNOWN / IMPLEMENTED 计数 | 56 / 75 | `registry.rs:150-207`、`lib.rs:173-249` 逐项脚本枚举（UTF-8 解码），与 Go 主仓 53 的对照见「注册表计数复核」 |

黑板原始文件（`.git/opencode-team/20261031-154523/qq-api-audit/`，**5 天 TTL**，以本报告为正式留存）：

| 文件 | 内容 |
|---|---|
| `01-researcher-official-doc-matrix.md` | 官方 47 端点 / 22 事件 / 错误码矩阵，含出处与置信度 |
| `02-researcher-official-doc-matrix-r2.md` | 素材管理端点族、频道消息、Intents 总表、@everyone 权威页、`template_id` 裁决 |
| `03-Team-local-inventory.md` | Go 53 注册名 / 15 事件；Rust `KNOWN_ACTIONS` 53 / `IMPLEMENTED_ACTIONS` 72 / 44 占位 / 14 事件 |
| `04-architect-architect-audit-plan.md` | 差异裁决矩阵与 6 个修复包（P-G1/P-G2/P-R1/P-R2/P-R3/P-DOC） |

本文档为只读复核结论，不含代码改动；后续如官方文档再有更新，应新开一份复核报告而不是改写本文件。

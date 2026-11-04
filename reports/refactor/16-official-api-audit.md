# QQ 官方能力全量复核（2026-10 快照）

## 元信息

| 项 | 值 |
|---|---|
| 复核日期 | 2026-10-31 ~ 2026-11-01 |
| 分支 | `Refactor` |
| 对应提交 | `d00a5ff`（`feat: 对齐 QQ 官方 2026-10 三条新能力并完成双侧全量复核`） |
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
| 未到里程碑（Rust 端点未移植/未接线） | **28** | 不动 | **未修**：`gensokyo-qq` 尚无对应端点是本轮范围外 |
| 官方无端点永久占位 | **16** | 文档登记 | **未修**：官方无对应能力，两侧行为一致 |
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

| 分组 | 数量 | 代表项 | 为什么不算缺口 |
|---|---|---|---|
| 端点未移植 | **23** | `set_group_kick`、`delete_msg`/`delete_group_msg`、`put_interaction`、`get_robot_share_link`、`set_group_add_request`、黑名单两枚、`get_qq_profile`、面板 6、菜单 2、审批策略 6 | `gensokyo-qq` 未暴露对应端点（`crates/gensokyo-actions/src/qqapi.rs`、`panels.rs`、`interaction.rs` 自述），属新链移植进度 |
| 发送链未接线 | **5** | `send_group_forward_msg`、`send_private_forward_msg`、`send_group_msg_raw`、`send_private_msg_sse`、`send_private_msg_wakeup` | 端点已在 Go 侧验证可用，Rust 仅缺 deferred 链分支 |
| 官方无端点（**不在这 28 项内**） | **16** | 退群、群列表、`get_msg`、合并转发查询、好友列表、陌生人资料、点赞、群荣誉、匿名禁言、@all 剩余次数、群名/管理员/头衔/备注、机器人资料写、群系统消息 | 官方无对应能力，永久占位；Go 侧同样降级或固定回执 |
| 固定 `retcode=100` | 2 实现 / 3 名 | `set_group_whole_ban`/`get_group_whole_ban`、`set_group_card` | 官方 `restrict_chat_setting` 只能按成员写，无全员禁言与群名片写接口 |
| OneBot 兼容空实现 | **5** | `set_restart`、`clean_cache`、`get_cookies`、`get_csrf_token`、`get_credentials` | 纯兼容回执，与 Go 一致，不产生上游调用 |

计数订正：`d00a5ff` 新增的三个素材管理 action 全部有真实上游，**不占**上述 44 项，且该提交未从占位集合中移除任何一项（改动文件不含 `unsupported.rs` / `panels.rs` / `group_admin.rs` / `interaction.rs`）。双链注册表因此为 Go `56` 个 action 名（复核时 53 + 3）、Rust `KNOWN_ACTIONS` 56、`IMPLEMENTED_ACTIONS` 75（72 + 3）。

## 遗留风险与已知限制

1. **带空格的 `[CQ:at, qq = all]` 已统一（2026-11-01）**：Go 的 `atEveryoneCQPattern` 与 `cqAtNumRe` 已改为容忍 `qq`、`=`、取值两侧空白；Rust 的 `at_mention_id` 本就对键值做 `trim`。两侧现均归一为标签并按同日额度计一次（`handlers/at_everyone.go:33`、`handlers/message_parser.go:55`、`crates/gensokyo-runtime/src/at_everyone.rs:134-148`）。
2. **缺 `qq` 的 at 段已统一（2026-11-01）**：契约定为「Markdown 内 `[CQ:at]` / `[CQ:at,foo]` / `[CQ:at,qq=]` → `1400`；普通文本原样透传」，Go 在 `parseMarkdownFromMessageE` 统一校验（`handlers/message_parser.go:3145-3149`），Rust 在 `render_markdown` 报错、`render_text` 透传（`crates/gensokyo-runtime/src/deferred.rs:393-428`）。
   - `qq` 非首参写法（`[CQ:at,name=x,qq=123]`）**已于 2026-11-03 统一**：Go 改按键取值（`cqAtSegRe` + `cqAtSegmentQQ`），与 Rust 同口径正常解析；不再判 malformed。
3. **CQ 参数空白容错已全量适配（2026-11-03）**：全部 CQ 段类型双侧统一——段名后/参数间/`=` 两侧容忍空白；词法切分空白集 = ASCII `{ ,\t,\n,\f,\r}`（同 Go 正则 `\s`），键/值首尾 trim = Unicode `TrimSpace` 语义（Go `cqCodeParam`/`buildParams` 对偶）；值内部空白保留。Rust `Segment` 挂 raw 原文，透传臂逐字节保真（`deferred.rs`/`outbound.rs`）；Rust parse 采用与 Go `scanSegments` 同款花括号配平，含逗号 JSON 不截断、配平失败不吞内嵌合法码（`cursor=start+1`）。详见 `release_log/CHANGELOG_v016.md`「CQ 码空白容错全量适配」。
4. **CQ 码解析全路径最大兼容统一（2026-11-04）**：上条「保留的既有结构差异」两项**已全部消除**——① Go legacy 媒体/markdown/keyboard 由位置锚定正则改为整码匹配 + `cqUnionParams` 按键取值（任意键序、尾键不污染值）；② 段名前空白四路统一识别，`errMarkdownAtMalformed` 下线。同轮统一：段名/键大小写不敏感（`[cq:` 前缀仍区分）、stream 冒号/等号双语法（冒号限 stream）、段名字符集 `[A-Za-z0-9_]`、Rust 裸尾段并入前一键、internal 新链补花括号配平状态机与 stream case、`CARDIGAN` 类前缀不吞段、`acnode` base64 豁免谓词适配宽松形态。详见 CHANGELOG「CQ 码解析全路径最大兼容统一」。
   - 记录的分层语义（非缺陷）：legacy 键值不做实体解码、`cq_parse_mode=new` 才反转义（模式设计本意）；`[CQ:reply,id=…,msg_idx=…]` 同现时整码按 reply 消费、msg_idx 随段移除（用例钉桩）。
   - 记录项：`adapter/identity`、`adapter/state`、`config`、`echo` 四包既有测试非幂等（`-count=2` 隔离复跑必红，`-count=1` 绿），属 TEST_DEFECT 待单独立项清理，与 CQ 改动无关。
5. **Rust keyboard / music / stream / input_notify 仍回 1400**（`deferred.rs:20`、`:587-590`）：本轮只放开 markdown 与 card，其余段类型保持显式拒绝而非静默丢弃。
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

黑板原始文件（`.git/opencode-team/20261031-154523/qq-api-audit/`，**5 天 TTL**，以本报告为正式留存）：

| 文件 | 内容 |
|---|---|
| `01-researcher-official-doc-matrix.md` | 官方 47 端点 / 22 事件 / 错误码矩阵，含出处与置信度 |
| `02-researcher-official-doc-matrix-r2.md` | 素材管理端点族、频道消息、Intents 总表、@everyone 权威页、`template_id` 裁决 |
| `03-Team-local-inventory.md` | Go 53 注册名 / 15 事件；Rust `KNOWN_ACTIONS` 53 / `IMPLEMENTED_ACTIONS` 72 / 44 占位 / 14 事件 |
| `04-architect-architect-audit-plan.md` | 差异裁决矩阵与 6 个修复包（P-G1/P-G2/P-R1/P-R2/P-R3/P-DOC） |

本文档为只读复核结论，不含代码改动；后续如官方文档再有更新，应新开一份复核报告而不是改写本文件。

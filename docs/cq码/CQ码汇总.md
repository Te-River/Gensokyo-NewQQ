# CQ 码汇总

Gensokyo 兼容 [OneBot V11](https://github.com/botuniverse/onebot-11) 标准 CQ 码协议，并在此基础上扩展了多种 CQ 码以适配 QQ Bot API 的特有能力。

> 🧪 **测试插件仓库**：[Gensokyo-NewQQ-Test-plugins](https://github.com/Te-River/Gensokyo-NewQQ-Test-plugins) — 每个 CQ 码 / 扩展 API 均配有对应的 NoneBot2 测试插件，各文档头部已贴出对应插件与触发命令，可直接部署体验。

---

## 标准 CQ 码

OneBot V11 协议定义的标准消息类型。

| CQ 码 | 范围 | 说明 | 文档 |
|-------|------|------|------|
| `[CQ:text]` | `全场景` | 纯文本消息段，最基本的消息类型。 | [查看](./标准CQ码/标准cq码-cq-text.md) |
| `[CQ:face]` | `全场景` | QQ 表情。⚠️ 暂不可用：入站不产出该段、出站按文本透传。 | [查看](./标准CQ码/标准cq码-cq-face.md) |
| `[CQ:image]` | `q群 (Group Chat)` / `C2C (私聊)` | 图片消息，支持 URL / base64 / 本地路径。 | [查看](./标准CQ码/标准cq码-cq-image.md) |
| `[CQ:record]` | `q群 (Group Chat)` / `C2C (私聊)` | 语音消息，支持 URL / base64 / 本地路径，自动转码 silk。`[CQ:voice]` 是其同义别名（键归一为 `*_record`）。 | [查看](./标准CQ码/标准cq码-cq-record.md) |
| `[CQ:video]` | `q群 (Group Chat)` / `C2C (私聊)` | 视频消息，支持 URL / 本地路径。 | [查看](./标准CQ码/标准cq码-cq-video.md) |
| `[CQ:at]` | `q群 (Group Chat)` | @ 标签，用于提及群成员或机器人。 | [查看](./标准CQ码/标准cq码-cq-at.md) |
| `[CQ:share]` | `-` | 分享链接（QQ Bot API 暂不支持发送）。 | [查看](./标准CQ码/标准cq码-cq-share.md) |
| `[CQ:location]` | `-` | 位置信息（QQ Bot API 暂不支持发送）。 | [查看](./标准CQ码/标准cq码-cq-location.md) |
| `[CQ:music]` | `q群 (Group Chat)` / `C2C (私聊)` | 音乐分享，当前仅支持 QQ 音乐。 | [查看](./标准CQ码/标准cq码-cq-music.md) |
| `[CQ:reply]` | `q群 (Group Chat)` / `C2C (私聊)` | 回复指定消息。 | [查看](./标准CQ码/标准cq码-cq-reply.md) |
| `[CQ:forward]` | `-` | 合并转发（QQ Bot API 暂不支持）。 | [查看](./标准CQ码/标准cq码-cq-forward.md) |

---

## 扩展 CQ 码

Gensokyo 为适配 QQ Bot API 特有能力而扩展的 CQ 码类型。

| CQ 码 | 范围 | 说明 | 文档 |
|-------|------|------|------|
| `[CQ:member]` | `q群 (Group Chat)` | 群成员入群/退群通知及回复路由。 | [查看](./扩展CQ码/扩展cq码-cq-member.md) |
| `[CQ:active]` | `-` | 主动消息标记，强制走主动推送通道。 | [查看](./扩展CQ码/扩展cq码-cq-active.md) |
| `[CQ:wakeup]` | `私聊 (C2C)` | C2C 互动召回（唤醒）消息标记，`userid` 指定目标用户（OpenID 或虚拟 ID）。 | [查看](./扩展CQ码/扩展cq码-cq-wakeup.md) |
| `[CQ:remove]` | `q群 (Group Chat)` | 撤回指定群消息（出站单向，需 user_id + msg_id）。 | [查看](./扩展CQ码/扩展cq码-cq-remove.md) |
| `[CQ:at]` | `q群 (Group Chat)` | @ 标签：纯文本出站转为 `@用户名 `，Markdown 出站转为 `<qqbot-at-user>` 标签。图文混合消息（msg_type=7）同样走纯文本转换路径；图文混合走 Markdown 路径（auto_md，msg_type=2）同样走 Markdown 转换路径（2026-08 修复）。 | [查看](./扩展CQ码/扩展cq码-cq-at.md) |
| `[CQ:card]` | `q群 (Group Chat)` | 图文卡片消息（msg_type=8），参数顺序无关；`title` / `desc` 缺失或超长整条消息回 `1400`，`pic` / `url` 缺失时卡片被丢弃、正文照常发送。 | [查看](./扩展CQ码/扩展cq码-cq-card.md) |
| `[CQ:input_notify]` | `私聊 (C2C)` | 输入状态通知，正文发送前先展示"正在输入"。 | [查看](./扩展CQ码/扩展cq码-cq-input_notify.md) |
| `[CQ:stream]` | `私聊 (C2C)` | 流式消息，逐片展示实现打字机效果（start→mid→finish）。 | [查看](./扩展CQ码/扩展cq码-cq-stream.md) |
| `[CQ:keyboard]` | `q群 (Group Chat)` / `C2C (私聊)` | 文本消息附加内嵌键盘（按钮消息）。⚠️ 按钮仅在 Markdown 消息下渲染，建议与 `[CQ:markdown]` 配合。 | [查看](./扩展CQ码/扩展cq码-cq-keyboard.md) |
| `[CQ:set_group]` | `q群 (Group Chat)` | 出站动作统一码：成员禁言/解禁（`action=ban`）、⚠️ 全员禁言（`action=whole_ban` **固定失败**：官方 POST 无全员禁言写入字段）、入群申请审批（`action=add_request`）、审批策略执行/删除（`action=strategy_execute`/`strategy_delete`）、成员踢出（`action=kick`，批量≤20可同步拉黑）、黑名单增删（`action=blacklist_add`/`blacklist_del`）。 | [查看](./扩展CQ码/扩展cq码-cq-set_group.md) |
| `[CQ:group_info]` | `q群 (Group Chat)` / `C2C (私聊)` | 群信息内容展开码：正文展开群名/群简报/成员数或三字段 JSON，支持指定群与失败 fallback，同群多码合并一次取数。⚠️ 需 `cq_parse_mode: new`；legacy/shadow 下该码不被识别会原文发出。 | [查看](./扩展CQ码/扩展cq码-cq-group_info.md) |

---

## 入站新增形态（2026-09）

入站消息事件新增三种上报形态（随消息事件字段降级产生，无独立文档）：

| CQ 码 | 说明 |
|-------|------|
| `[CQ:ark,ark_type=...,ark_name=...,prompt=...]` | 结构化卡片（`message_type=3` / `ark_data`），`ark_type` 为官方枚举；无 `ark_data` 时上报裸 `[CQ:ark]` |
| `[CQ:record,file=...,asr=...]` | 语音附件（`content_type=voice`），`file` 取 `voice_wav_url`，缺失时回退附件 url；`asr` 为空时省略 |
| `[CQ:reply,msg_idx=...]` | 引用消息（`msg_elements`），`msg_idx` 为 REFIDX 索引，与出站 `[CQ:reply,id=...]` 区分 |

> 三种形态均为**入站专用**：出站解析会剥离 `[CQ:ark]`（含参数）与 `[CQ:reply,msg_idx=...]`，`[CQ:record]` 的尾随 `,asr=` 参数亦被排除。

---

## CQ 码解析模式（cq_parse_mode）

消息解析统一为 cqparse 架构（字符串 / 消息段数组 / TRSS map 三输入归一 Token 流），由配置 `cq_parse_mode` 控制：

| 模式 | 行为 |
|------|------|
| `legacy`（默认） | 走旧解析器，零行为变化。 |
| `shadow` | 新旧两套解析并行运行，行为仍走 legacy；结果有差异时输出 `[cqparse-shadow]` 差异日志。 |
| `new` | 全部走新解析器。修复旧解析的若干缺陷（贪婪 JSON 正则跨码吞噬、字符串路径批量 user_ids 只取第一个、动作码私聊泄漏、媒体码附加参数污染 URL、stream 等号语法等）；`[CQ:group_info]` 仅在此模式下可用。 |

配置非法值（大小写敏感）时回退 `legacy` 并输出一次警告。建议先以 `shadow` 观察日志无差异再切换 `new`，可随时改回 `legacy` 回滚。

---

## 无参裸码行为

媒体类 CQ 码**不带任何参数**（裸码）时，行为是**保留原文字面**，而不是被静默移除或产出一个空媒体：

| 形态 | 行为 |
|------|------|
| `[CQ:image]` / `[CQ:record]` / `[CQ:voice]` / `[CQ:video]`（`file` 与 `url` 均为空） | 整码原样保留为正文文本，不产出媒体项 |
| `[CQ:file]`（`file` 为空，无论 `url` 是否有值） | 同上：保留字面，不产出媒体项 |
| `[CQ:card]`（无任何参数） | 码被移除、无产物（`handlers/cqparse/handlers_content.go:122-124`） |
| 控制类裸码（`[CQ:active]` 等） | 不适用本表：控制码识别后从正文移除并写入 `foundItems` 控制字段 |

锚点：`handlers/cqparse/handlers_media.go:56-59`（媒体族 `file`/`url` 皆空 → `Replacement: tok.Raw`）与 `:87-91`（`file` 码只看 `file`）；legacy 与新解析器口径一致（代码注释「裸码 legacy 保留字面，new 对齐不静默移除」）。

> 💡 若上游插件发来空参媒体段，现在的表现是「正文里原样留着这串 `[CQ:...]`」，不会带出内容为空的媒体，需要插件自己补齐参数。

---

## URL 改写与二维码（`transfer_url` / `url_to_qrimage`）

出站文本中的 URL 由 `transformMessageTextUrl` 统一处理：用 `xurls.Relaxed` 找出正文里所有链接，逐个按配置替换或删除（实现见 `handlers/message_parser.go:1749-1790`）。

| 配置项 | 模板默认值 | 行为 |
|--------|------------|------|
| `transfer_url` | `true`（`template/config_template.go:189`） | 开启后，正文中每个 URL 被改写为机器人自身的短链跳转地址 `https://{server_dir}/url/{短码}`（302 跳转） |
| `url_to_qrimage` | `false`（`template/config_template.go:187`） | **opt-in**：需 `transfer_url` 同时为 `true`。命中后把该 URL 生成二维码图片单独作为图片发出，并从正文中删除原 URL |
| `visible_ip` | `false`（`template/config_template.go:186`） | 当 `server_dir` 是 IP 地址且 `visible_ip: false` 时，URL 被替换为空字符串（即从正文中删除） |

> ⚠️ **升级须知（沿用配置模板的实例必读）**
> `transfer_url` 在配置模板中的默认值是 `true`（`template/config_template.go:189`），不特意配置就直接升级，出站文本里的 URL 会被改写：
>
> - `server_dir` 为域名 → 链接替换为 `https://{server_dir}/url/{短码}`；
> - `server_dir` 为 IP 且 `visible_ip: false` → 链接被**删除**，用户看不到原始 URL；
> - 所用域名必须已在 q.qq.com 后台报备并通过 `/url` 与 `/image` 两条路径的校验，否则跳转与图片外链都会被 QQ 拦截；
> - 不希望改写 → 显式设置 `transfer_url: false`（热重载即时生效，可作为一键回退开关）。

---

## 范围说明

| 标记 | 含义 |
|------|------|
| `全场景` | 所有消息场景（群聊、私聊等） |
| `q群 (Group Chat)` | QQ 群消息 |
| `C2C (私聊)` | QQ C2C 单聊消息 |
| `-` | 通用解析；是否能发送取决于调用的 Action 和 QQ API 限制 |

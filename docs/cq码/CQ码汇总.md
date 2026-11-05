# CQ 码汇总

Gensokyo 兼容 [OneBot V11](https://github.com/botuniverse/onebot-11) 标准 CQ 码协议，并在此基础上扩展了多种 CQ 码以适配 QQ Bot API 的特有能力。

> 🧪 **测试插件仓库**：[Gensokyo-NewQQ-Test-plugins](https://github.com/Te-River/Gensokyo-NewQQ-Test-plugins) — 每个 CQ 码 / 扩展 API 均配有对应的 NoneBot2 测试插件，各文档头部已贴出对应插件与触发命令，可直接部署体验。

---

## 标准 CQ 码

OneBot V11 协议定义的标准消息类型。

| CQ 码 | 范围 | 说明 | 文档 |
|-------|------|------|------|
| `[CQ:text]` | `全场景` | 纯文本消息段，最基本的消息类型。 | [查看](./标准CQ码/标准cq码-cq-text.md) |
| `[CQ:face]` | `全场景` | QQ 表情（经典小脸表情）。⚠️ 开发中，暂无法正常使用。 | [查看](./标准CQ码/标准cq码-cq-face.md) |
| `[CQ:image]` | `q群 (Group Chat)` / `C2C (私聊)` | 图片消息，支持 URL / base64 / 本地路径。 | [查看](./标准CQ码/标准cq码-cq-image.md) |
| `[CQ:record]` | `q群 (Group Chat)` / `C2C (私聊)` | 语音消息，支持 URL / base64 / 本地路径，自动转码 silk。 | [查看](./标准CQ码/标准cq码-cq-record.md) |
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
| `[CQ:card]` | `q群 (Group Chat)` / `私聊 (C2C)` | 图文卡片消息（msg_type=8），群聊与单聊载荷同构，参数顺序无关，`title` / `description` / `url` / `pic_url` 四项必填。 | [查看](./扩展CQ码/扩展cq码-cq-card.md) |
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

## 参数空白容错与最大兼容（2026-11）

全部 CQ 码类型在**所有解析路径**（Go legacy / Go cqparse / Go internal 新链 / Rust 新链）统一为最大兼容并集口径（详见 `release_log/CHANGELOG_v016.md`）：

- `[CQ:image , file = https://example.com/a.png]`、`[CQ:at, name=x, qq=123]`（参数间、`=` 两侧空白，任意键序）均可识别；值仅去首尾空白，**内部空格保留**（`[CQ:record,file = my voice.amr]` 的值为 `my voice.amr`）。
- 段名与参数键**大小写不敏感**（`[CQ:Image,File=x]` 等价）；`[cq:` 前缀本身仍区分大小写，小写 `[cq:` 整码透传。
- 段名可带前导空白（`[CQ: image,file=x]`、`[CQ: at,qq=all]`）；段名字符集为 `[A-Za-z0-9_]`（`[CQ:3d,x=1]` 按未知段处理）；含 `!`/`(`/全角空格等非法字符的段名整码按文本透传。
- stream 支持冒号与等号双语法（`[CQ:stream,type:start,qq:1]`、`[CQ:stream,type=start,qq=1]` 及混写）；冒号语法仅 stream 段生效。
- 词法切分的空白集为 ASCII `{空格, \t, \n, \f, \r}`（同正则 `\s`，不含 `\v`/全角空格）；键与值的 trim 为 Unicode 语义。
- 未被识别的畸形段原样保留为文本；识别成功的段透传输出时保持原文字节（不做静默规范化改写）。

---

## 范围说明

| 标记 | 含义 |
|------|------|
| `全场景` | 所有消息场景（群聊、私聊等） |
| `q群 (Group Chat)` | QQ 群消息 |
| `C2C (私聊)` | QQ C2C 单聊消息 |
| `-` | 通用解析；是否能发送取决于调用的 Action 和 QQ API 限制 |

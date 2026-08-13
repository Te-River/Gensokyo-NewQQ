# Changelog — Release012

> 自 Release011 以来的所有变更。本轮合并 main 与 Pr-Edit 两条线：Pr-Edit 侧补齐 QQ 官方 API v2 全部群聊接口（文档全量遍历确认共 17 个）并接入入群申请事件（`GROUP_JOIN_REQUEST`）；main 侧包含语音上传修复、@Bot 剥离改进、分阶段重构基础设施（PR #49）等既有变更。

---

## 🆕 新功能

### 群聊 API 全量补齐（11 个接口，12 个方法）

基于官方文档实现全部群聊管理类接口，其中 **6 个入群自动审批策略接口为本轮新增**，其余 5 个为上一轮开发、本轮随提交落库：

| 接口 | HTTP 路径 | 方法 | 说明 |
|------|-----------|------|------|
| 获取群基本信息 | `/v2/groups/{group_openid}/info` | GET | 群名、简介、分类、标签、成员数 |
| 获取机器人群内状态 | `/v2/groups/{group_openid}/bot_state` | GET | 入群时间、是否可推送、角色 |
| 入群申请列表拉取 | `/v2/groups/{group_openid}/join_request_list` | GET | 分页拉取，需群管理员身份 |
| 入群申请审批 | `/v2/groups/{group_openid}/approval_join_request/{member_openid}` | POST | approve/decline，支持拒绝理由与拉黑 |
| 查询群禁言状态 | `/v2/groups/{group_openid}/restrict_chat_setting` | GET | 全员禁言 + 成员级禁言列表 |
| 设置群成员禁言 | `/v2/groups/{group_openid}/restrict_chat_setting` | POST | 单次最多 10 个成员 |
| 创建入群自动审批策略 | `/v2/groups/join_approval_strategy` | POST | 一个机器人最多 20 个策略 |
| 查询策略列表 | `/v2/groups/join_approval_strategy` | GET | cursor 分页，limit 默认 20 最大 100 |
| 修改策略 | `/v2/groups/join_approval_strategy/{strategy_id}` | PATCH | 启停/过期/增删关联群/备注 |
| 执行策略 | `/v2/groups/join_approval_strategy/{strategy_id}/execute` | POST | 对关联群全量扫描，异步约 10 分钟 |
| 修改策略白名单 | `/v2/groups/join_approval_strategy/{strategy_id}/whitelist_users` | POST | 单次最多 10000 个 QQ 号码 |
| 删除策略 | `/v2/groups/join_approval_strategy/{strategy_id}` | DELETE | — |

既有群聊消息类接口（发送/撤回群消息、富媒体上传/预上传/分片完成）保持不变，至此 17 个群聊接口全部覆盖。

### GROUP_JOIN_REQUEST 入群申请事件

官方文档确认该事件属于 `GROUP_AND_C2C_EVENT`（1<<25）Intent，机器人需为群管理员才可收到：

- **botgo 事件层**：事件常量、intent 映射、`ParseData` 事件 ID 注入、`GroupJoinRequestEventHandler` 注册（含未注册时的告警日志）
- **Processor 接入**：新增 `ProcessGroupJoinRequest`，将 `group_openid`/`member_openid` 映射为虚拟 ID，上报 OneBot `request` 事件（`request_type=group`、`sub_type=add`），`flag` 携带 `join_request_id` 供审批使用，`comment` 携带验证信息
- **配置**：`text_intent` 新增 `GroupJoinRequestEventHandler`（需群管理员身份）

### set_group_add_request 真实化

- `handlers/set_group_add_request.go` 由 MOCK 占位改为真实调用 v2 审批接口：虚拟 ID 经 idmap 反查真实 OpenID，`flag` 作为 `join_request_id`，`approve` 映射为 `op=approve/decline`
- `callapi.ParamsContent` 新增扩展参数：`approve`、`flag`、`reason`（拒绝理由）、`add_to_member_blacklist`（是否拉黑）

### 群聊管理 API 暴露为 OneBot action

全部 11 个群聊管理类接口现已注册为 OneBot action，下游插件可直接调用：

| Action | 对应 botgo API | 说明 |
|--------|---------------|------|
| `get_group_info`（真实化） | `GroupInfo` | 返回真实群名/简介/成员数（原为 MOCK 假数据） |
| `set_group_ban`（真实化） | `SetRestrictChatSetting` | `duration` 秒禁言，0=解除（原为"暂未开放"骨架） |
| `set_group_whole_ban`（真实化） | `SetRestrictChatSetting(AllMute)` | `enable` 开关全员禁言，保留已有成员级禁言 |
| `get_group_join_request_list`（新增） | `JoinRequestList` | 入群申请列表，`next_index` 分页；返回的 `group_id`/`user_id`/`flag` 可直接回传审批 |
| `get_group_bot_state`（新增） | `BotInGroupState` | 机器人群内状态（入群时间/可推送/角色） |
| `join_approval_strategy_create/list/update/execute/whitelist/delete`（新增） | 6 个策略 API | 入群自动审批策略全生命周期管理 |

`set_group_ban`/`set_group_whole_ban` 同时保留旧 action 名 `get_group_ban`/`get_group_whole_ban` 兼容。`callapi.ParamsContent` 同步新增 `next_index`/`cursor`/`limit`/`strategy_id`/`group_openids`/`group_ids`/`is_enable`/`expire_at`/`remark`/`op`/`whitelist_users` 参数。

### 动作型 CQ 码（群聊管理）

复用现有 `[CQ:member]`/`[CQ:remove]` 的"解析-执行-移除"出站模式，新增 4 个动作型 CQ 码（`handlers/message_parser.go` + `send_group_msg.go` 文本路径接入，纯 CQ 码消息不发送只回执）：

| CQ 码 | 动作 |
|-------|------|
| `[CQ:set_group_ban,group_id=...,user_id=...,duration=秒]` | 成员禁言（0=解除） |
| `[CQ:set_group_whole_ban,group_id=...,enable=true/false]` | 全员禁言开关（保留成员级禁言） |
| `[CQ:set_group_add_request,group_id=...,user_id=...,flag=...,approve=true/false]` | 入群申请审批（可带 reason/add_to_member_blacklist） |
| `[CQ:strategy,action=execute/delete,strategy_id=...]` | 审批策略执行/删除 |

`group_id` 支持跨群路由（省略时使用发送目标群）；参数缺失/未知 action 时 CQ 码原样保留不吞掉。数据查询类接口（get_group_info 等）不做 CQ 码。

### CQ 码处理重构（独立文件 + 单次扫描）

- 新增 `handlers/cqcode.go`（546 行）：集中全部 CQ 码处理——包级正则常量（16 个）、`ProcessCQFile`/`ProcessCQActive`（迁移）、出站动作型统一入口
- 新增 `ProcessOutboundCQCodes(text, defaultGroupID, eventID, apiv2) (string, string)`：**单次正则扫描全文**，按类型分发执行动作，未知类型（标准 CQ 码）原样保留
- 出站动作型由 6 次独立全文扫描（member/remove/ban/whole_ban/add_request/strategy 各自 `ReplaceAllStringFunc`）改为 1 次扫描，行为完全兼容（member 的 eventID/跨群路由语义保留，内部函数对默认群 ID 增加反查保持等价）
- `message_parser.go` 相应精简（-540 行），`send_group_msg.go` 6 次调用改为 1 次

### [CQ:keyboard] 独立内嵌键盘 CQ 码

新增扩展 CQ 码 `[CQ:keyboard]`，使文本消息可以直接附加 QQ 官方内嵌键盘（按钮消息），覆盖群聊与单聊：

| 语法 | 说明 |
|------|------|
| `[CQ:keyboard,data=base64://<键盘JSON>]` | base64 编码形式（与 `[CQ:markdown]` 一致） |
| `[CQ:keyboard,data=<键盘JSON>]` | 原始 JSON 形式 |

- **解析**：`handlers/cqcode.go` 新增 `ProcessCQKeyboard`，从出站文本中提取并移除 CQ 码，解码后的键盘 JSON 存入 `foundItems["keyboard"]`；`handlers/message_parser.go` 的 `parseMessageContent` 统一接入
- **发送**：`send_group_msg` / `send_private_msg` / `send_group_msg_raw` 文本路径在无 markdown 时解析键盘（复用 `parseMDData` 结构），附加到 `MessageToCreate.Keyboard`，以 `msg_type=0` 文本 + 键盘发送
- **键盘能力**：支持官方三种形态（模板 `id`、`content.rows`、顶层 `rows`）；`specify_user_ids` 数字虚拟 ID 自动转换为 OpenID（`ResolveKeyboardVirtualIDs`）；私聊 `__USER_ID__` 占位符替换为实际用户 OpenID（`ResolvePlaceholderUserIDs`）；按钮本地图片自动解析（`ResolveKeyboardImages`）
- **优先级**：与 `[CQ:markdown]` 同存时 markdown 优先（其附带键盘生效）

### CQ 码解析统一管道架构（ProcessCQCodePipeline）

- 新增 `handlers/cqcode_pipeline.go`：统一管道 `ProcessCQCodePipeline()` 处理所有出站 CQ 码字符串解析（媒体/控制/动作），输出纯文本 + foundItems
- `message_parser.go` 移除分散的 CQ 码解析逻辑（string 分支内的 markdown/card/input_notify/stream/媒体正则，-116 行），统一调用 `ProcessCQCodePipeline`
- **架构原则**：cqcode_pipeline.go 负责解析、cqcode.go 负责正则常量与动作执行、message_parser.go 只负责消息段格式转换
- **修复**：消息段数组路径（NoneBot 等框架的 segment_type_koishi 格式）下 CQ 码原样发出的问题——此前 string 分支内的解析逻辑在 `[]interface{}` 路径不执行，统一管道确保两种路径解析一致

### QQ 开放平台新能力调研（群聊/C2C 场景）

基于 QQ 开放平台 api-v2 文档（2026-07 更新）全量比对：

- **事件**：群聊/C2C 场景的文档化事件（`GROUP_AND_C2C_EVENT` 1<<25 全量 10 个 + `GROUP_MESSAGE_CREATE` + 探测所得成员/入群申请/内联搜索/互动）本地已全部适配，**无新增可适配事件**
- **表情消息**：官方 api-v2 消息类型新增"表情"页面（2025-07），但公开文档未披露群聊发送接口与 `msg_type` 取值，无法确认适配方式
- **表情表态（reaction）**：官方明确"仅支持在频道内使用"，群聊/C2C 不可用
- **消息审核（MESSAGE_AUDIT）**：官方描述面向主动消息审核，群聊/C2C 场景适用性未获官方确认，暂不接入
- **图文消息**：官方 4 个发送场景（单聊/群聊/文字子频道/频道私信）中的图文能力已由本地富媒体 `msg_type=7` 与卡片 `msg_type=8` 覆盖

---

## 🐛 Bug 修复

### [CQ:keyboard] 顶层 content 形态解析失败

**文件：** `handlers/message_parser.go`

`[CQ:keyboard]` 独立使用时的官方简写形态 `{"content": {"rows": [...]}}`（顶层 `content`）与按钮模板形态 `{"id": "..."}`（顶层 `id`）无法被 `parseMDData` 解析，导致键盘 JSON 解析失败（kb=nil）、CQ 码原样发出。

**修复：** `parseMDData` 的临时结构体新增顶层 `content`/`id` 字段，与既有嵌套 `keyboard` 形态并行支持，不影响 `[CQ:markdown]` 既有行为。

### 消息段数组路径 CQ 码原样发出

**文件：** `handlers/message_parser.go`、`handlers/cqcode_pipeline.go`

`parseMessageContent` 中 `[]interface{}` 路径（NoneBot 等框架发送的消息段数组格式）下，text 段内嵌的 `[CQ:keyboard]` 等 CQ 码未被解析，原样作为文本发送到 QQ。

**修复：** 新建 `cqcode_pipeline.go` 统一管道 `ProcessCQCodePipeline()`，在 switch 之后统一调用，确保 string 与 `[]interface{}` 两种路径均经过同一解析。

### 复检问题修复（P1/P2/P3 全量）

针对复检报告（`reports/refactor-validation/rereview-2026-08-11.md`）中的 6 项问题逐一修复：

**P1-4.1 热路径聚焦测试**：新增 `handlers/cqcode_pipeline_test.go`（24 个用例），覆盖统一管道全部 CQ 码类型的解析、文本剔除、keyboard JSON 解析、card/stream 参数验证、消息段数组路径、幂等性与未知 CQ 码保留。

**P1-4.2 CI 测试门禁**：`cross_compile.yml` 新增 `test` job（`go vet ./...` + `go test ./... -count=1`），`build` job 通过 `needs: test` 依赖，测试失败则编译不执行。

**P2-4.3 前端测试占位符**：`frontend/package.json` 的 test 脚本替换为零依赖语法验证脚本 `scripts/test-syntax.js`（检查 30 个 .ts/.vue 文件的非空、UTF-8 合法性、script 标签闭合），CI 中 ESLint 仍由 quasar build 覆盖。

**P2-4.4 SSM 补发关联标识**：`echo.MessageGroupPair` 新增 `EnqueueTime`/`CorrelationID` 字段；`PushGlobalStack` 自动生成 `ssm-<group>-<seq>` 关联标识并记录入队时间；`send_group_msg.go` 与 `send_group_msg_raw.go` 的 5+4 处入队/补发路径日志统一携带 `[SSM][cid]` 前缀，补发时输出队列停留时长，实现跨入队/补发边界的日志关联。

**P3-4.5 [CQ:face] 文档一致性**：文档已标注"开发中，暂无法正常使用"（上一轮完成），与代码零实现一致。

**P3-4.6 MCP 版本固定**：`.mcp.json` 两个 MCP 服务固定版本（`@modelcontextprotocol/server-github@2025.4.8`、`@upstash/context7-mcp@4.0.1`），并修正 context7 错误包名（原 `@context7/server` 不存在）。

### [CQ:keyboard] 渲染说明补充

实测确认：**QQ 客户端仅在 Markdown 消息（`msg_type=2`）下渲染内嵌键盘按钮**，纯文本消息（`msg_type=0`）附带的 `keyboard` 参数不显示按钮。已在 `扩展cq码-cq-keyboard.md` 新增"渲染说明（重要）"章节，建议与 `[CQ:markdown]` 配合使用；`CQ码汇总.md` 同步标注。测试插件文本同步修正（移除消息文本中的 CQ 码字面量）。

### [CQ:keyboard] 完整构建文档 + AGENTS.md 适用范围

- `扩展cq码-cq-keyboard.md` 新增"完整构建（按钮 JSON 全字段诠释）"章节：Keyboard/KeyboardContent/Row/Button/RenderData/Action/Permission 全字段表格 + 三按钮完整示例（指令+回调+跳转），并附 QQ 开放平台官网链接（发送群聊/单聊消息页面）
- `AGENTS.md` 新增"适用范围（重要）"章节：规范仅在本仓库内拥有最高优先级，其他项目/工作区域以用户指令为准、无需参考本文件；文件随仓库公开上传供所有访问本仓库的 Agent 使用

### 前端构建失败修复（ESLint 11 处错误）

**文件：** `frontend/src/`（5 个 Vue/TS 文件）

`npm run build` 因 11 处 ESLint 错误（10 处 `no-unused-vars` + 2 处 `no-explicit-any`）导致 COMPILATION FAILED，前端产物无法生成，`go:embed` 一直打包旧 WebUI。

**修复：**
- `ChannelList.vue`/`GroupList.vue`：`defineProps` 去掉未使用的 `props` 变量赋值
- `ChannelView.vue`：删除未使用的 `watch`/`GroupList`/`groupList`；`getNextPage`/`getPreviousPage` 删除无用参数并同步调用处
- `GroupView.vue`：删除未使用的 `ChannelList` 导入；`handleRowClick` 删除无用 `index` 参数
- `LoginView.vue`：`catch(err)` 未使用变量加行内忽略
- `ChannelView.vue`/`GroupView.vue`：文件头补充 `no-explicit-any` disable（与既有 `no-unsafe-assignment` 风格一致）

### [CQ:face] 文档标注开发中

**文件：** `docs/cq码/标准CQ码/标准cq码-cq-face.md`、`docs/cq码/CQ码汇总.md`

`[CQ:face]` 官方未公开群聊/C2C 表情发送接口，文档标注"开发中，暂无法正常使用"（标题下新增提示 + 汇总表标注），与代码零实现保持一致。

### CodeQL 警报 #30 修复（js/bad-tag-filter）

**文件：** `frontend/scripts/test-syntax.js`

CodeQL 规则 `js/bad-tag-filter`（high）触发：HTML 标签正则大小写敏感，不匹配 `<SCRIPT>` 等大写标签。

**修复：** `/<script[\s>]/`、`/<template[\s>]/`、`/<\/script>/` 三处正则添加 `i`（大小写不敏感）标志；CodeQL 重新扫描后警报自动关闭（状态 fixed）。

### CI 申请 PR 时构建全平台双版本

**文件：** `.github/workflows/cross_compile.yml`

申请 PR（`pull_request` opened/synchronize）自动触发的构建 action 由单版本改为**全平台双版本**（9 平台 × 完整版 + noWebUI 精简版，共 18 产物）：

- Compile 步骤同时构建完整版与 `-tags=small` 精简版，产物命名 `gensokyo-{os}-{arch}[-noWebui][.exe]`
- UPX 压缩循环处理两个产物（android/darwin 跳过逻辑保留）
- Upload artifacts 上传双产物（多行 path pattern）
- 本地验证：`go build ./...` 与 `go build -tags=small ./...` 均通过，YAML 语法正确

#### Test & Vet 步骤修复（webui embed 占位）

Test job 的 `go vet ./...` 与 `go test ./... -count=1` 步骤此前在 CI 中必然失败：`webui/dist/` 被 `.gitignore` 忽略，checkout 后目录不存在，`go:embed dist/*` 编译报 `pattern dist/*: no matching files found`。

**修复：** 在 Test job 中新增 `Create webui embed placeholders` 步骤，创建与 `build.ps1` 的 `Ensure-WebUIDist` 一致的占位文件（覆盖 dist 及 css/fonts/icons/js 五个 embed pattern）；9 平台编译矩阵与 `contents: read` 最小权限保持不变。

**验证：** 移除 `webui/dist` 后按 workflow 等效命令本地执行，`go vet ./...` 与 `go test ./... -count=1` 均通过（37 包全绿）。

### 语音上传失败修复

**文件：** `handlers/send_group_msg.go`

`url_record`/`url_records` 处理路径调用了 `UploadBase64RecordToServer`（上传到本地服务器），当 `server_dir` 是私有地址时会失败。

**修复：** 改为与 `local_record` 一致，直接调用 `CreateAndUploadMediaMessage` 上传 QQ CDN。

### 媒体消息 `url` 字段兼容修复

**文件：** `handlers/message_parser.go`

部分客户端（如 Koishi）使用 `url` 字段而非 `file` 字段传递媒体路径，导致本地文件路径被当作 `unknown_*` 处理，触发 SSRF 阻止。

**修复：** 在 `image`、`voice/record`、`video` 的解析中，当 `file` 字段为空时，回退读取 `url` 字段。

### lazy_message_id 多段回复偶发 40054005 msgseq 去重（Issue #19）

**文件：** `echo/messageidmap.go`

`lazy_message_id=true` 下一条命令触发多段独立回复时，偶发某段发送失败（QQ API 返回 `40054005 消息被去重`）。

**根因：** `GetLazyMessagesId`/`GetLazyMessagesIdv2` 选中一条 record 后执行 `usageCount++`，导致同一命令的多段回复每次选中不同 record，各段拿到不同 `msg_id`。QQ API 视两 `msg_id` 为同一回复链，要求 `msg_seq` 连续递增，但不同 `msg_id` 各自独立计数导致 seq 冲突。

**修复：** 移除选中后的 `usageCount++`，让同一回复链的多段回复复用同一 `msg_id`，配合 `GetMappingSeq`/`AddMappingSeq` 连续递增 `msg_seq`。

---

### 私聊 reply 跨场景 msg_id 越权（40034024）

**文件：** `handlers/send_private_msg.go`、`handlers/send_group_msg.go`

私聊 reply 时，虚拟 ID 反查的真实 ID 格式校验存在缺陷（`UserID MessageID` 格式不匹配），导致群聊 `msg_id` 被引用到私聊场景，触发 QQ API `40034024` 越权错误。

**修复：** `applyPrivateReply` 改用反查前 `RetrieveRowByIDv2` 校验虚拟 ID 归属是否为当前私聊目标 UserID，不一致则跳过 reply 避免引用群聊 msg_id 越权。富媒体（msg_type=7）/纯文本/markdown 三处 reply 段统一改用 `applyPrivateReply` 辅助函数。

---

### 群聊 Markdown nil pointer panic

**文件：** `handlers/send_group_msg.go`

`parseMarkdownFromMessage` 解析失败返回 nil 时，`md.Content = messageText + md.Content` 触发空指针 panic。

**修复：** 补 `if md != nil` 校验，nil 时跳过文本合并，走纯文本路径。

---

### 图文消息 CQ:at 未转换（PR #16）

**文件：** `handlers/message_parser.go`

图文消息走 Markdown 路径（`auto_md`）时，`[CQ:at,qq=...]` 未被正确转换为 QQ @ 语法，导致原文显示。

**修复：** 在 Markdown 消息处理路径中补充 CQ 码 @ 转换逻辑。

---

### botgo 层修复

**文件：** `botgo/dto/group.go`、`botgo/openapi/v2/group.go`

- `GroupJoinRequestEvent` 补 `ID`/`EventID` 字段（修复编译错误），`ApplyAt` 改为 `interface{}` 兼容不同类型时间戳
- `ApprovalJoinRequest` 请求体字段名修正（`action` → `op`），并支持 `join_request_id`/`reject_reason`/`add_to_member_blacklist`

---

## 🔧 @Bot 剥离改进

### 全量群消息 @Bot 剥离修复

**背景：** QQ 平台在不同场景使用不同 ID 格式（全局 OpenID vs 群特定 OpenID），导致 `handlers.BotID`（来自 `/users/@me`）与群消息 Mentions 中的 ID 可能不一致，`IsSelfAtID` 无法识别为自身 @。

**修复（`2a6315d`）：** 全量群消息（`DisableErrorChan=true`）时 `RevertTransformedText` 不会被调用，`<@BotOpenID>` 原样上报。在 `ProcessGroupNormalMessage` 中添加独立的 @Bot 剥离步骤，无论 `DisableErrorChan` 如何设置均执行。导出 `handlers.IsSelfAtID` 供 Processor 包调用。

**修复（`4bbaf17`）：** 在 `resolveIncomingAtID` 解析出 atID 后，额外检查 atID 是否等于 bot 的 UIN 或 AppID，若是则认定为 @Bot 自身并剥离。同时修复 `RevertTransformedText`（string 模式）和 `ConvertToSegmentedMessage`（array 模式）两处。

---

### @bot 移除逻辑简化

**文件：** `handlers/message_parser.go`

用正则直接匹配 AppID 简化 @bot 移除逻辑，不再依赖复杂的 ID 格式判断。`transformMessageTextAt` 根据 `remove_bot_at_group` 配置移除 @bot。

---

### 非全量群消息 @用户重复修复

**文件：** `handlers/message_parser.go`、`handlers/send_group_msg.go`、`handlers/send_group_msg_raw.go`

#### 非全量群重复 @ 修复（`86888fb`）

未开启全量群消息（仅订阅 `GROUP_AT_MESSAGE_CREATE` 被动消息）的群中，`add_at_group` 配置会添加 `[CQ:at,qq=AppID]`，但被动消息本身已包含对 bot 的 @，导致出站消息出现重复 @。

**修复：** 被动消息（`GROUP_AT_MESSAGE_CREATE`）中直接移除 `add_at_group` 添加 `[CQ:at,qq=AppID]` 的逻辑。`add_at_group` 仅在全量群消息（`GROUP_MESSAGE_CREATE`）中生效。

#### 非全量消息 @用户重复修复（PR #47）

非全量群消息中，出站回复时出现 @用户重复。进一步修复了非全量消息场景下 @用户重复的问题，并同步更新了 `send_group_msg` 和 `send_group_msg_raw` 的处理逻辑。

---

## 🔧 稳定性修复

### HTTP 资源边界与消息序列原子化（`9240987`）

**涉及：** `echo/`、`handlers/`、`idmap/`、`imagehosting/`、`server/`、`Processor/` 等 26 个文件

- HTTP 客户端增加超时和连接资源边界控制，防止资源泄漏
- 消息序列号生成原子化，避免并发冲突
- idmap HTTP 客户端增加安全边界
- imagehosting 各后端统一资源管理
- server 端 HTTP/webhook 增加请求边界控制
- echo 系统并发安全改进

### 投递重试分类统一

- `b93b01d`：集中投递重试分类逻辑
- `21d0015`：handler 错误路由到重试分类器

---

## 🆕 新增接口

### idmap 批量身份映射导出（`/getid?type=19`）

**文件：** `idmap/service.go`、`server/getIDHandler.go`

新增 `GET /getid?type=19` 接口，一次性导出所有身份映射（OpenID <-> 虚拟 ID），方便迁移调试和数据核查。

**新增结构体：**
- `IdentityMapping`：单条映射记录（`real_id`、`virtual_id`、`username`）
- `IdentitySnapshot`：快照响应（`mappings`、`count`、`timestamp`）

**新增函数：**
- `ListAllIdentities() (*IdentitySnapshot, error)`：遍历 identityDB 导出所有正向映射

**快照机制：**
- 使用 bbolt MVCC `View` 事务，在请求到达时生成一致性快照
- 即使遍历期间有其他 goroutine 写入 identityDB，也不影响本次导出结果
- `timestamp` 字段记录快照生成时间（Unix 时间戳，秒）

**注意事项：**
- type=19 不走 lotus/gRPC 远程调用，始终查询本地 identityDB
- 受 `IDMapAuthMiddleware` 保护（lotus password 或本地回环限制）

详见 [docs/idmap.md](./docs/idmap.md#批量导出接口)。

---

## 🏗 分阶段重构基础设施（PR #49）

**说明：** 本轮重构在旧代码旁新增 `internal/` 分层架构包（双轨并存，未接入生产），生产路径（handlers / Processor / callapi / idmap / echo）行为完全不变。

### 配置基础设施：Immutable Snapshot 管线（P2）

**新增 `internal/infrastructure/config/`：**

- `loader.go`：读取/解析 `config.yml` → `ConfigDTO`
- `migrator.go`：`Migrator` 接口（基于 `yaml.Node`，v0→v1 自动补 `version`）
- `validator.go`：`ValidateSchema` + `ValidateSemantic`（错误带具体路径）
- `runtime.go`：`ConfigDTO → RuntimeConfig`（slice 深拷贝）
- `snapshot.go`：不可变 `Snapshot` + `Manager`（重载失败保留旧快照）
- `writer.go`：`AtomicWrite`（tmp + fsync + `.bak` 备份 + rename）
- `watcher.go`：fsnotify + debounce
- `errors.go`：分类错误（Parse/Migration/Validation/IO）+ 字段路径

### Identity 类型化（P3）

**新增 `internal/domain/identity/`：**

- `OpenID` / `OpenGroupID` / `VirtualUserID` / `VirtualGroupID` / `UIN` / `AppID` 类型 + 转换
- `IsOpenID` / `IsVirtualID`（长度启发式唯一收敛点）
- `IdentityResolver` 接口 + `UserRef`/`GroupRef`/`ResolvedUser`/`ResolvedGroup`
- 新增 `adapter/identity/`：基于 idmap 的真实 Resolver 实现

**长度启发式收敛：** 全仓 `len(id)==32 / !=32` 身份判断归零，替换为 `identity.IsOpenID(...)`。

### 消息解析类型化（P4）

**新增 `internal/domain/message/`：**

- `MessagePart` 类型系统 + `ParsedMessage`（Parts/Reply/DeliveryMode）
- `ParseOneBotString`（String）与 `ParseOneBotSegments`（Array）输出同一模型
- CQ 码扫描器（转义感知参数拆分、malformed 容错）
- `MediaSource`（LocalFile/RemoteURL/Base64/Bytes）
- compat bridge：`Canonicalize` + `ToLegacyFoundItems`（仅迁移期）

### 媒体管线统一（P5）

**新增 `internal/application/media/`：**

- `MediaService.Prepare(ctx, source, policy)`：Local/URL/Base64/Bytes 统一入口
- `SafeHTTPFetcher`：timeout / max bytes / 重定向限制 / SSRF 检查
- Base64 decode 前长度限制（防 OOM）；图片尺寸/像素校验（防解压炸弹）
- 本地文件：AllowedDirs + 扩展名 + regular + 大小校验
- `PreparedMedia.Close()` 临时文件生命周期（幂等）

### 出站消息模型（P6）

**新增 `internal/application/outbound/`：**

- `OutboundService.Send(ctx, OutboundCommand)`：唯一发送主链
- `OutboundMessage` / `OutboundCommand` / `DeliveryPolicy`
- `QQSender` 接口（Application 不 import botgo DTO）
- `RetryPolicy` + `ErrorClassifier`

### 队列与 WebSocket 生命周期（P7）

**新增 `internal/application/queue/`：**

- 有界队列：容量均分分区、session hash 分区保证顺序
- 背压显式选择：Block / Drop / Reject
- 重试走 delay scheduler（不占 worker Sleep）
- Close/Wait 可预测 shutdown；Metrics

### Processor 分层（P8）

**新增 `internal/domain/event/` + `internal/application/inbound/` + `adapter/onebot/`：**

- `DomainEvent` + `EventSource`
- `EventNormalizer` / `EventPublisher` 接口
- `IsSelfMention` / `NormalizeMentions`：@Bot 唯一 canonical 实现
- OneBot serializer：`SerializeString` / `SerializeArray`

### HTTP/WS/callapi Adapter（P9）

**新增 `internal/application/action/`：**

- `Registry` 显式注册表 + `Dispatcher`（HTTP/WS 共用）
- `Envelope` / `Handler` / typed `SendMessageAction`（int/string ID 兼容）

### idmap/echo 仓储化（P10）

**新增 `internal/application/state/`：**

- `SequenceRepository`：msgseq 只允许原子 Next
- `MessageContextRepository`：owner+key 隔离 + TTL 过期校验
- `IdentityRepository` 复用 P3 的 IdentityResolver

### 全局配置依赖收口（P11）

- 验证 `internal/` + `adapter/` 全部无 `config` import
- outbound 增加 `OutboundConfig` 构造注入

### SDK / Generated 边界隔离（P12）

- 新增 `adapter/qq/`（botgo → typed identity 转换边界）
- 验证 domain/application 无 botgo/go-silk
- 新增 fork inventory：`docs/forks/{botgo,go-silk}.md`
- 新增 `scripts/generate.{ps1,sh}`

### P13 — 条件未满足，破坏性删除 BLOCKED

- 生产路径尚未切换到新架构，无真实联调稳定周期 → **不执行破坏性删除**
- 已完成非破坏收尾：新架构无 foundItems（仅 compat bridge）、无 config/botgo/go-silk

---

## 🔒 Nature 图床凭据说明

`imagehosting/nature.go` 内置的腾讯 COS 凭据（`oss_type=10` Nature 免费图床）为**公开共享凭据**，并非用户私有密钥，无泄露风险，维持"开箱即用"行为。

- 曾一度将其误判为需移除的私有云凭据并改为配置注入（提交 `87d34f8`），经确认属于误判，已回滚（`d030501`）。
- 当前 `oss_type=10`（Nature）保持内置凭据、开箱即用；如需自定义 COS 请使用 `cos.go`（需配置）。

---

## 📦 构建与工程

| 文件 | 变更 |
|------|------|
| `.github/dependabot.yml` | 曾新增 Dependabot 配置（覆盖 Go/npm/GitHub Actions 依赖），后随 `6033728` 删除停用 |
| `.gitignore` | 新增 `.qoder/` 忽略项 |

### Dependabot 停用与依赖回退

- 提交 `6033728 chore: 停用 Dependabot 自动依赖更新`：删除 `.github/dependabot.yml`，停用 Go/npm/GitHub Actions 依赖的自动更新 PR，避免持续污染 PR 列表
- 此前 Dependabot 合并的依赖更新（PR #20-#44：golang.org/x/net、x/crypto、x/image、grpc、form-data、@types/node、setup-go/setup-node 等）已**全部回退**，当前依赖保持原版本（`google.golang.org/grpc v1.65.0`、`golang.org/x/crypto v0.23.0`、`golang.org/x/net v0.25.0` 等）
- GitHub Actions 保持 `actions/setup-go@v6`、`actions/setup-node@v6`

---

## 📝 文档同步

| 文件 | 变更 |
|------|------|
| `release_log/CHANGELOG_v012.md` | 本文档（新建，融合 main 与 Pr-Edit 两份变更记录） |
| `template/config_template.go` | `text_intent` 注释新增 `GroupJoinRequestEventHandler` |
| `readme.md` | 已实现 Intent 列表新增 `GroupJoinRequestEventHandler`（入群申请）；API 表新增 8 个群聊管理 action 并标注真实化 |
| `docs/api/api介绍.md` | 标准/扩展 API 表新增 8 个 action 及参数说明 |
| `docs/api/扩展API文档.md` | 扩展 API 索引新增 8 个 action |
| `docs/api/扩展api/` | 新增 `get_group_join_request_list`、`get_group_bot_state`、`join_approval_strategy`（6 action）详细文档 |
| `docs/cq码/` | 新增 4 个动作型 CQ 码文档（set_group_ban/set_group_whole_ban/set_group_add_request/strategy），`CQ码汇总.md` 索引同步 |
| `readme.md` | 拓展 CQ 码表新增 4 个动作型 CQ 码 |
| `docs/本版新增功能.md` | 事件表新增 `GroupJoinRequestEventHandler`；API 表 8 个 action 加链接；`set_group_add_request` 过时 MOCK 描述修正 |
| `AGENTS.md` | botgo Fork 描述补充群聊管理 API（GroupAPI）与入群申请事件 |
| `docs/cq码/` | 标准 CQ 码文档完善（12 个标准 CQ 码文档 + 1 个统一汇总，同步 `docs/更多文档.md` 索引） |
| `docs/forks/` | fork inventory：`botgo.md`、`go-silk.md`（P12） |
| `docs/cq码/` | 新增 `扩展cq码-cq-keyboard.md`（[CQ:keyboard] 独立内嵌键盘），`CQ码汇总.md` 扩展表新增 keyboard 行 |
| `docs/本版新增功能.md` | 消息与 CQ 行为章节新增 `[CQ:keyboard]` |
| `release_log/CHANGELOG_v012.md` | 新增 [CQ:keyboard] 与官方新能力调研结论两个小节 |
| `docs/cq码/` | `标准cq码-cq-face.md` 标题下新增"开发中"提示；`CQ码汇总.md` face 行标注开发中 |
| `docs/cq码/` | `扩展cq码-cq-keyboard.md` 新增"渲染说明（重要）"与"完整构建（按钮 JSON 全字段诠释）"章节；`CQ码汇总.md` keyboard 行同步标注 |
| `AGENTS.md` | 新增"适用范围（重要）"章节，限定规范仅在本仓库内生效 |
| `frontend/scripts/test-syntax.js` | 新增零依赖前端语法验证脚本（30 个 .ts/.vue 文件） |
| `.github/workflows/cross_compile.yml` | 新增 `test` job（go vet + go test）；申请 PR 时构建全平台双版本（完整版 + noWebUI） |
| `.mcp.json` | 两个 MCP 服务固定版本（github@2025.4.8、context7-mcp@4.0.1），修正 context7 错误包名 |
| `reports/refactor-validation/rereview-2026-08-11.md` | 新增复检与测试报告（全流程测试结果、热路径走查、F1-F4 复核、6 项逻辑问题汇总） |

---

## 🧪 验证

| 命令 | 结果 |
|------|------|
| `go build ./...` | ✅ 通过 |
| `go vet ./...` | ✅ 通过 |
| `go test ./handlers/` | ✅ 通过 |
| `go test ./Processor/` | ✅ 通过 |
| `go test ./imagehosting/ ./config/ ./structs/ ./template/` | ✅ 通过 |
| `go test ./internal/infrastructure/config/` | ✅ 通过（24 用例） |
| `go test ./internal/domain/identity/ ./adapter/identity/` | ✅ 通过（12 用例） |
| `go test ./internal/domain/message/` | ✅ 通过（96.9% coverage） |
| `go test ./internal/application/media/` | ✅ 通过（69.6% coverage） |
| `go test ./internal/application/outbound/` | ✅ 通过（90.9% coverage） |
| `go test ./internal/application/queue/` | ✅ 通过（86.1% coverage） |
| `go test ./internal/application/inbound/ ./adapter/onebot/` | ✅ 通过 |
| `go test ./internal/application/action/` | ✅ 通过（87.5% coverage） |
| `go test ./internal/application/state/` | ✅ 通过（97.5% coverage） |
| `go test ./...` | ✅ 通过 |
| `go test ./handlers/ -run TestProcessCQCodePipeline` | ✅ 通过（24 用例，统一管道全覆盖） |
| `npm test`（frontend） | ✅ 通过（30 个 .ts/.vue 文件语法验证） |
| `npm run build`（frontend） | ✅ 通过（仅 2 个非阻塞 warning） |
| `go build -tags=small ./...` | ✅ 通过（noWebUI 精简版编译验证） |

---

## ✅ 提交记录

```
ecff462  feat: 合并 PR #49 — 分阶段重构基础设施（P2-P12）
a926dda  refactor(legacy): document P13 deletion gating（P13）
a305fea  refactor(adapter): isolate botgo and media SDKs（P12）
8cb1c6b  refactor(config): remove global config reads from new architecture（P11）
9e58ce9  refactor(storage): repository-ize idmap and echo state（P10）
514be8c  refactor(callapi): add typed actions and explicit registry（P9）
444ae1f  refactor(processor): introduce inbound event pipeline（P8）
fe4a4a7  refactor(queue): add bounded queue with session ordering（P7）
2f1cbe6  refactor(outbound): introduce unified outbound service（P6）
762ab11  refactor(media): unify media pipeline with safe fetcher（P5）
b751e65  refactor(message): introduce typed parsed message model（P4）
cb75b38  refactor(identity): add typed identity resolver（P3）
15cef7c  refactor(config): introduce immutable runtime snapshots（P2）
21d0015  refactor: route handler errors through retry classifier
b93b01d  refactor: centralize delivery retry classification
9240987  fix: bound HTTP resources and atomize message sequences
bcba13d  修复图文消息cqat未转换（PR #16）
5a9c36d  fix: 修复私聊reply越权/群聊Markdown panic/多段回复msgseq去重
e72e04f  fix: 修复私聊富媒体reply跨场景msg_id越权(40034024)
1ed26a5  fix: lazy_message_id 多段回复偶发 40054005 msgseq 去重（Issue #19）
4bbaf17  fix: 全量群消息 @Bot 剥离失败 — IsSelfAtID 因 ID 格式不匹配返回 false
2a6315d  fix: 全量群消息(DisableErrorChan=true)时 @Bot 未被剥离
6421bcd  移除非全量消息@用户重复（PR #47）
961daf4  refactor: 正则直接匹配 AppID，简化 @bot 移除逻辑
bef1750  fix: transformMessageTextAt 根据 remove_bot_at_group 移除 @bot
158c33c  fix: 被动消息中移除 add_at_group 添加 @ 逻辑
86888fb  fix: 非全量群重复 @ 修复（add_at_group 增加 remove_at 判断）
d030501  revert: restore nature public image hosting credentials
87d34f8  security: remove embedded cloud credentials from Nature provider
f535855  docs: 为新增群聊管理扩展 API 编写详细文档
ce9791d  feat: 群聊管理 API 全部暴露为 OneBot action
43b816e  feat: 补齐群聊 API 并接入入群申请事件
0b4ee93  ci: 新增 Dependabot 配置覆盖 Go/npm/GitHub Actions 依赖
c4300c8  ci: 申请 PR 时构建全平台双版本（完整版 + noWebUI）
70933ec  fix: CodeQL 警报 #30 - test-syntax.js HTML 标签正则大小写不敏感
1519d29  docs: AGENTS.md 限定仓库适用范围 + [CQ:keyboard] 完整构建文档
675b874  fix: 复检 P1/P2/P3 全量修复 + [CQ:keyboard] 渲染说明
ccdb4a0  docs: 补充统一管道重构记录并输出复检与测试报告
f229318  refactor: CQ码解析统一管道架构（ProcessCQCodePipeline）
6a404d4  fix: [CQ:keyboard] 顶层 content/id 形态解析失败导致 CQ 码原样发出
123d249  docs: 标注 [CQ:face] 为开发中状态
2a842e2  fix: 修复前端构建失败（ESLint 未使用变量与 any 类型错误）
56ef577  feat: 新增 [CQ:keyboard] 独立内嵌键盘 CQ 码并完成官方能力调研
```

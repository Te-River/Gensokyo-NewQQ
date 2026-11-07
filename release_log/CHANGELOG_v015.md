# Changelog — Release015

> 自 Release014 以来的所有变更。

---

## Added

- 为 QQ CDN 富媒体上传与统一图床上传添加重试机制：上传失败后最多重试 2 次（线性退避 1s/2s）；CDN 仅对超时类错误重试，图床对所有错误重试。
- 群成员管理（QQ 官方 v2 接口）：
  - `get_group_member_list` 升级为真实成员列表（游标分页全量拉取，单页≤30、安全上限 100 页；`join_time`/`role`/`nickname` 为真实数据）；官方接口不可用时自动回退本地 idmap-pro 缓存路径，中途页失败返回已收集数据。
  - `get_group_member_info` 升级为真实单成员信息；接口失败时回退中性值兜底。
  - 新增 `set_group_kick`：批量移出群成员（单批≤20，`user_id`/`user_ids` 二选一可合并去重，`add_blacklist` 移出同时拉黑）。
  - 新增 `get_group_member_blacklist` / `set_group_member_blacklist`：群黑名单列表查询与增删。
- C2C 自定义菜单与指令面板：新增 `get_custom_menu` / `set_custom_menu` / `get_panel_list` / `create_panel` / `get_panel` / `set_panel` / `delete_panel` / `set_panel_target` 共 8 个扩展 action（菜单仅 C2C 全局生效；面板支持 c2c/group/channel/dm；`create_panel`/`set_panel_target` 关联对象列表任一反查失败整体报错）。
- `[CQ:set_group]` 新增 3 个子动作：`action=kick`（批量踢人，`user_ids` 逗号分隔、≤20 截断警告、`add_blacklist` 同步拉黑）、`action=blacklist_add` / `action=blacklist_del`（黑名单增删）；详见 [扩展 CQ:set_group](../docs/cq码/扩展CQ码/扩展cq码-cq-set_group.md)。
- 全局域名统一：`api.sgroup.qq.com` → `api.bot.qq.com`（botgo v1/v2 与图床直连地址）。官方"统一请求地址"仅记载 `api.bot.qq.com`，sandbox 域名官方文档已不再单列；`sandbox_mode` 配置项保留，行为收敛到同一域名。
- 登录排障提示（UX）：登录失败时按官方错误码输出针对性指引（`11245`=固定 Token 已禁用，提示使用 appid+client_secret 动态鉴权；`11243`=Token 错误，提示核对 appid/client_secret）；启动时输出当前 API 域名日志，便于确认新旧构建。
- Markdown 消息新增配置项 `force_verify_image_resource`（默认 false）：开启后官方校验图片转存结果，转存失败返回错误（40034004）且消息不发送；注入点收敛在 `parseMDData()`，群聊与单聊 Markdown 共用。
- **行为变化（假数据诚实化）**：
  - 发送失败/官方审核中场景的回执 `message_id` 由固定假值 `123` 改为 `0`（无真实 message_id 不再伪造；`retcode 0` 语义保持不变）。
  - `set_group_card` 由假成功改为明确失败回执（`retcode 100`，QQ 官方 API 未提供设置群名片接口）。
  - `get_group_list` 的 `group_create_time` 由当前时间戳改为 `0`（官方无群创建时间字段），并逐群拉取真实群名/群简介/成员数（受 `get_g_list_delay` 毫秒节流防超 QPM，群量大时响应变慢属预期）。
  - `get_status` 的包收发/断线/丢包统计由假数据改为 `0`（官方无统计接口且 wsclient 无导出连接计数），`online`/`good` 保持 true（进程存活即在线）；消息收发统计仍为 botstats 真实数据。
  - `mark_msg_as_read` 清除假字面量，保持空实现回执（QQ 官方无已读上报接口）。
  - `get_group_member_list` 弱字段（`level`/`area`/`title`）由固定 `"0"` 改为空字符串（真实接口无对应字段，不再伪造；下游按数字解析 `level` 需注意，本地缓存回退路径仍为 `"0"`）。
- **CQ 码统一解析架构（cqparse）与新扩展码**：
  - 新增 `handlers/cqparse` 统一解析包：字符串 CQ 码 / 消息段数组 / TRSS map 三输入归一为 Token 流，转义编解码唯一实现，替换值单遍拼接永不回炉重扫；22 个既有码全部迁入，新增码只需注册一次（根治"修一个漏一片"）。
  - 新增扩展码 `[CQ:group_info,field=name/memo/member_count/all,group_id=...,fallback=...]`：正文展开群名/群简报/成员数或三字段 JSON；同消息同群多码合并一次取数（30 QPM 保护）；失败分级（参数错保留原文，取数失败替换 fallback 默认空串）；详见 [扩展 CQ:group_info](../docs/cq码/扩展CQ码/扩展cq码-cq-group_info.md)。
  - 新增配置项 `cq_parse_mode: legacy|shadow|new`（默认 `legacy`，合并后零行为变化）：`shadow` 并行跑新旧解析、差异仅日志上报（`[cqparse-shadow]`）行为仍走 legacy；`new` 全走统一解析器；可随时改回 `legacy` 回滚。**建议先 shadow 观察一版无意外 diff 再切 new**。
  - **`cq_parse_mode: new` 下的行为修复**（legacy 模式维持旧行为）：
    - 修复贪婪 JSON 正则跨码吞噬：`[CQ:markdown,...]普通文本[CQ:keyboard,...]` 相邻时正文与 keyboard 不再丢失。
    - 修复字符串路径批量 `user_ids=1,2,3` 只取第一个的问题，与消息段/TRSS 路径等价。
    - 修复 `[CQ:remove]`/`[CQ:set_group]` 失败路径把 CQ 码原文留在正文发出的问题（统一"无论成败移除+日志"）。
    - 修复动作码在私聊/转发路径原样泄漏为聊天文本的问题（现拦截：移除+日志，不执行不发送）。
    - 修复媒体码附加参数污染 URL（`file=xx,subType=0,url=...`）、`xxx.image` 文件名整码泄漏、`file` 值含逗号被截断、未知前缀泄漏等问题（统一入 `unknown_*` 或按参数正确解析）。
    - 修复 `[CQ:cardboard]` 被 `[CQ:card]` 前缀误吞、`[CQ:stream,type=start,qq=1]` 等号语法不识别、`[CQ:reply,id=字母数字]` 不解析、消息段数字 qq/group_id 产 `[CQ:at,qq=]` 空参等问题。
    - `[CQ:avatar]` 多码不再共用第一个 URL；产物直接入 `url_images`，不再以 `[CQ:image,...]` 形式回写正文；反查失败时丢弃该码并记录日志（不再产出破损 URL）。
  - **入站修复（独立于解析模式，立即生效）**：
    - `array=true` 全量群消息（GROUP_MESSAGE_CREATE）现在与字符串路径一致剥离 @bot（此前 array 模式会向下游上报"有人@了机器人"，可能触发自我应答）。
    - `string_ob11=true` 时 C2C 私聊消息与群消息对齐：`message_id`/`user_id`/`sender.user_id` 上报真实 string（此前私聊无 StringOb11 分支，ID 体系割裂）。
    - `array=true` 时附件按 ContentType 过滤，视频/语音附件不再误标为 image 段。
    - 平台事件伪装消息（c2c_switch / 群消息拒收）的 `message_id` 由固定假值 `123` 改为保留值 `-1`（杜绝与真实虚拟 msg_id 撞号导致 reply/撤回串扰；该消息不可 reply）。
    - 修复 user_id 含正则元字符时出站 @ 处理触发 `regexp.MustCompile` panic 可致进程崩溃的问题（改为精确字符串匹配）。
    - 入站热路径正则改为包级预编译（每条群消息节省 ≥2 次正则编译）。

## Fixed

- CodeQL 整数转换告警：Processor 包 9 处消息 ID 的 `int64→int` 无界转换统一收敛到 `safeMessageID` 边界检查（统一按 int32 边界收紧；越界记日志回退 `-1` 哨兵，杜绝截断产生错误 ID）。
- 修复纯 markdown/keyboard 段消息（无文本段）在群聊与私聊被静默丢弃的问题（存量：发送门槛只看文本段），现在 md-only/kb-only 消息正常发送。
- 修复消息段嵌套 `data.data` 双层包装（`{"type":"markdown","data":{"data":{"markdown":{"content":...}}}}`，keyboard 同构）在 `cq_parse_mode: shadow/new` 下内容丢失的问题：cqparse 段路径未按 legacy 取法解包 `segment["data"]["data"]`，内层 map 被 `coerceString` 归空后 markdown/keyboard 静默丢弃（`new` 模式发送门槛不开 → 日志停在"群组发信息使用messageID"、群里无消息、无报错；`shadow` 模式持续上报 `[cqparse-shadow]` diff）。现 normalize 层对齐 legacy 解包（仅一层，markdown marshal+base64 / keyboard marshal 原样 JSON），三模式 `foundItems` payload 逐字节一致；内层 JSON 字符串与 `base64://` 形态行为不变。
- 修复入站 @ 转换"前移/丢失"问题（真实环境：`@bot at检测 @同道中人 @备用 @晓山瑞希` 上报成 `[at:qq=..]at检测  <@备用裸文本>`，at 被拼到消息最前、部分 mention 丢失）：
  - **@前移**：`array=true` 时 `ConvertToSegmentedMessage` 产出的所有 at 段被 `sortMessageSegments` 统一排到消息最前，与原文位置脱钩。现改为按 `<@OpenID>` 原位逐段扫描：at 段在原位置展开（text/at 交错保序），反查失败的 `<@OpenID>` 原样保留在原位文本中，不前移、不重排、不丢失；图片段保持"内容段之后"的既有语义。字符串路径本就原位替换，两路径行为对齐（新增一致性断言）。
  - **@bot 识别（mentions 联合判定）**：`GROUP_AT_MESSAGE_CREATE`（AT 路径）此前不注册 mentions 中 `is_you/bot` 标记的 OpenID，`<@bot>` 判定落入 idmap 反查，失败时裸文本残留或被误转换；全量群消息路径的注册循环带 `break`，首个误标项会挤掉真正的 @bot。现两条路径均全量注册，且 `RevertTransformedText`/`ConvertToSegmentedMessage` 改为"事件 mentions(is_you/bot) ∪ 全局注册"联合判定。
  - **is_you 误标发送者导致 @ 丢失**：`is_you` 在多实例等场景可能误标到发送者上，发送者的 @ 会被当作 @bot 剥离。现判定时排除消息作者（发送者自身绝不视为 @bot），性能模式（`disable_error_chan=true`）的 @bot 剥离同步排除作者；@bot 自身的解析结果仍按 `use_uin` 映射 UIN/AppID，不会映射成发送者 ID。
  - 附带加固：`BotID` 为空（Ready 事件异常未设置）时跳过 `BotID→AppID` 全文替换，防止空串替换把消息逐字符穿插污染。
  - 新增回归网 `handlers/zzzz_inbound_at_order_test.go`：8 个场景（@bot+多他人 / @bot 在中间 / 仅 @bot / 反查失败他人 / convertOtherAt=false / removeAt=true·false / is_you 误标发送者 / 全量群消息），逐场景断言 string 输出与 array 段序列完全保位，并校验两路径一致。
- 修复 legacy 模式下嵌套 `data.data.markdown` 双层包装识别失败导致 md-only 段消息发送空消息（40034030）的问题：`parseMDData` 新增 `unwrapNestedMarkdown`（gjson 安全导航），支持标准 `{"markdown":{"content":...}}`、双包装 `{"markdown":{"markdown":{...}}}`、顶层 `{"content":"..."}` 三种 JSON 形态；群聊 Markdown CQ 码字符串路径 keyboard 类型不匹配一并修复；D2 ForceVerifyImageResource 注入点不受影响。
- 对齐官方 20260916 字段修正：`/v2/generate_url_link` 请求体字段名由 camelCase `callbackData` 改为官方文档的 `callback_data`（botgo/dto `GenerateURLLinkToCreate` json tag）；此前 callback 透传数据可能静默丢失（链接仍可生成，加好友事件 scene_param 回传断链）。响应侧 `data.url` 本就一致，未改动。注：服务端对旧 camelCase 的容忍度未实测（无法离线验证），以官方文档为权威对齐。
- 修复 ChatGLM 图床（`oss_type=7`）图片在 QQ 不显示：图床外链带两层短签名（约 30 分钟 + 302 跳转后再叠 15 分钟），直传给 QQ 官方 API 时服务端转存拉取失败、或用户查看时签名已过期。现对 `chatglm.cn` 域（含 `t1.`、`pfiles.` 等子域，按 host 边界匹配，`evil-chatglm.cn.example.com` 这类拼接不会误判）外链改为仓库内自取转传：
  - **富媒体**（`url_image`/`url_images`，群聊与单聊）：自取字节后走既有本地字节上传链（`images.CreateAndUploadMediaMessage`/`CreateAndUploadMediaMessagePrivate`，≤5,000,000 `file_data` 直传、超限自动分片），不再把短签名外链交给 QQ。
  - **Markdown / keyboard** 外链：自取字节后复用本地图片那条既有上传流程（`images.UploadBase64ImageToServer`）换稳定外链；当前图床本身就是短签名域时，换到的是重新签发的新链接（原链接已更接近过期，记 warn 后仍替换）。
  - **SSRF 防护**：入口先过 `isPrivateOrLoopback`，首跳与每一跳重定向都复检（被污染的图床域名可把 302 指向 `169.254.169.254` 等内网地址，响应字节还会被上传成可见图片）；重定向显式限 10 跳、总超时 30s、响应上限 20MB、带浏览器 UA。
  - **降级语义**：自取或上传任一步失败，或响应体魔数不是 png/jpeg/gif/webp → 记 warn 后维持原外链直传（不比现状差，不阻断发送）。legacy 媒体链路中自取返回的 `MessageToCreate` 由既有分支接住，图文混合仍是一条消息、一次上传。
  - 新增 `imagehosting/selffetch.go`（自取 + 域名判定 + SSRF 判据注入）与 `handlers/shortlived_image.go`（三处消费点入口，含格式嗅探与降级），回归网见 `imagehosting/selffetch_test.go`、`handlers/shortlived_image_test.go`。
  - 附注：同期实测 ukaka、星野图床同样出现外链失效，属图床侧现状，未纳入本次修复。
- 修复 ChatGLM 短签名外链仍被直接写进出站字段的问题（QQ 不跟随短链的 302，图片不显示）：`images/upload_api.go` 在返回图床外链前先用 `imagehosting.ResolveRedirectURL` 把 302 解析为 OSS 预签名直链（只读响应头、`Range: bytes=0-0`、首跳与每一跳复检 SSRF），解析失败再回退 `oss_type=0` 的本地公网 URL；`handlers/shortlived_image.go` 的 Markdown/keyboard 消费点优先走同一条直链解析，自取转传降级路径若仍拿到短签名外链则再解析一次，成功即采纳直链、失败才拒绝替换并维持原外链；旧行为是把短链（含重新签发的新短链）直接写进 Markdown/卡片等出站字段，即"直接使用短链"的问题（2026-11-07）。
- **Markdown 图片改走官方素材管理 API（最终方案）**：实测即使把 chatglm 外链解析为 pfiles 直链、并把请求体里的 `&` 以原始形式发出，QQ 服务端转存仍然拉取失败（图片不显示）。按官方文档「Markdown 消息 > 图片」改用素材管理：本地图片字节先 `POST /v2/media-images/prepare`（file_size/md5/sha1/content_type，≤10MB、png/jpeg/gif/webp）→ `PUT` 预签名（不带 Authorization）→ `POST /v2/media-images/complete`（md5 取 PUT 的 ETag，去引号并归一大小写），得到平台素材引用 `qqbot-media://image/v1/<token>`（平台托管的可复用引用，滑动有效期 90 天，每次被消息引用自动顺延；发送后不删除，同一张图可被多条消息复用）后直接写入 Markdown/keyboard，平台侧不再需要拉取任何网络资源。oss_type=7 即「官方素材 API 模式」：Markdown/keyboard 本地图只走素材链，不再先传 ChatGLM 图床（双上传消除），图床仅作素材链失败后的兜底，两条链不并存；`qqbot-media://` 只适用于 Markdown/keyboard，卡片 pic_url、`/uploadpic`、`/uploadpicv2` 等必须公网 URL 的路径维持现状。错误码 40035001~40035010 映射中文说明并保留 trace_id；素材链失败或 40035007 配额超限按「素材链 → 直链解析 → 外链上传」降级，不阻断发送；40035007 会先做一次配额清理——`GET /v2/media-images` 拉全量列表按 expires_at 升序删 20 张（最久未被引用的先删：图片为滑动过期，每次被消息成功引用后顺延至 90 天，刚发送过的图自然沉底不被优先删，无需创建时间保护窗；官方说明删除只影响后续新引用、已发送消息不受影响，平台已转存副本）后重试一次，仍失败才降级（2026-11-07；同日修复：原实现按 created_at 最旧 + 跳过 24 小时内新建，100 张全部为 24 小时内新建时一张都删不掉，被迫降级外链导致图片不显示，改为 expires_at 升序后该场景可正常清理重试；同日加固：并列按 created_at 升序、expires_at 缺失/为 0 视为「尚未被引用」排在最后（刚上传未引用的图不争先删）、40035008 视为配额已释放并计入重试条件）。同日 DTO 修复：官方列表数值字段（total/created_at/expires_at）实发为字符串形态（线上日志 `json: cannot unmarshal string into Go struct field MediaImageList.total of type int`），导致拉列表失败、清理永不重试、白白降级外链；fork 的 `botgo/dto/media_image.go` 对 `MediaImage`/`MediaImageList`/`MediaImageCompleteResponse` 增加宽容解析（shadow struct + `flexInt64` 同收 number 与数字串、null/缺省回零值、其余非法形态仍报错），字段 Go 类型与调用点零改动，序列化仍输出 number（Issue #56）。
- 出站请求体的 JSON HTML 转义关闭：resty 客户端 `SetJSONEscapeHTML(false)`（`[OPENAPI]` 日志行的 req 字段同步改为不转义打印，便于核对带签名 URL 的原文）


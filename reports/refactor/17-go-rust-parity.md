# Go↔Rust 全链路 parity 审计（进行中）

> 起因：用户实测发现 Rust 缺 QQ 上游事件 WS 客户端（#70 补）。用户指令：所有流程逐项过一遍，不许有盲区。
> 本文由三域并行审计汇总：A 连接与入站 / B 出站与消息链 / C 基础设施。逐项 file:line、状态=已对齐/部分/缺失。

## A. 连接与入站

| 级别 | 缺口 | Go 证据 | Rust 现状 | 状态 | 规模 |
|---|---|---|---|---|---|
| 主干 | QQ 上游事件 WS 网关 | main.go:315,375-400；botgo/websocket/client/client.go:64-138,155-185,254-349 | gateway.rs（950 行）+ boot 装配 + 7 条 mock 用例 | **已对齐（#70 `dfee81b`）** | — |
| 主干 | `post_url` 上报 | Processor/Processor.go:402-452 | reporter.rs（223 行）+ boot 装配，6 用例 | **已对齐（P5，待提交）** | — |
| 功能 | 正向 WS 不下发事件 | server/wsserver.go:111-112 + Processor.go:292-330 | ws_server.rs：lifecycle/hub 订阅/心跳/回执并行 | **已对齐（P4 `9e71c31`）** | — |
| 功能 | 反向 WS 收到 action 不 dispatch | wsclient/ws.go:249-261 | ws_client.rs：dispatch_json + mpsc 回执写回 | **已对齐（P4 `9e71c31`）** | — |
| 安全 | webhook ed25519 签名 / op13 | server/webhook.go:94,115-120,163-193 | webhook.rs：Go 同款 seed 派生/待签/401；op13 {plain_token,signature} | **已对齐（W1，待提交）** | — |
| 功能 | union_webhook 扇出 | main.go:530,1046-1090 | 无 | 缺失 | 80-120 行 |
| 功能 | 反向 WS TLS/代理 | ws.go:352-355 | wss 随 rustls feature；代理走环境变量 + CONNECT 隧道（P22） | **已对齐** | — |
| 功能 | `array_value` 消息段 | ProcessGroupMessage.go:189-190 | inbound 三事件段数组（P19；媒体入站段仍缺） | **已对齐（文本/at）** | — |
| 功能 | `remove_at`/`convert_other_at` | message_parser.go:1679-1721,1877,1895 | revert_mentions：自身@剥离/还原、只查不分配、trim、去 /（P20） | **已对齐** | — |
| 功能 | `enable_change_word` | main.go:765-770,787-792 | inbound/send 均接 acnode 开关（P8；审计时表项已过期） | **已对齐** | — |
| 功能 | `add_at_group` 消费者 | ProcessGroupNormalMessage.go:148-150（**入站上报**变换，非出站） | inbound.rs reported_group_content（全量群消息加 `[CQ:at,qq=AppID]`，与 Go 同点） | **已对齐（W2，待提交）** | — |
| 功能 | AppID→BotID | message_parser.go:1669-1683（需 `/users/@me` 的 BotID 注入） | 缺 BotID 来源（Go main.go:254/304），暂缓 | 缺失 | 中 |
| 功能 | `op_user_id_type` | Processor.go:245-270（调用点 :293,311,334） | gateway.rs `apply_op_userid_type`（网关 dispatch / webhook 发布前改写；vuin/ruin/其余三态 + sender 跟随） | **已对齐（P24）** | — |
| 功能 | `http_only_bot` | Processor.go:374-399（全 WS 失败时补 downtime 回复） | downtime.rs：无下游订阅者时按 Go 回群 AT/C2C（P26） | **已对齐** | — |
| 边角 | 401/403 语义 | wsserver.go:85-94；httpapi.go:23-31 | auth.rs：WS 缺 token 401 `Missing token` / 错 token 403 `Incorrect token`；HTTP API 一律 403 `鉴权失败` | **已对齐（P24）** | — |
| — | 15 类 QQ 事件转换 | main.go:746-887,889-930 | inbound.rs:282-328 | **已对齐** | — |
| — | intents 位表 | botgo/event/register.go:100-172 | intent.rs:77-95 | **已对齐**（待 #70 消费） | — |

## C. 基础设施

| 级别 | 缺口 | Go 证据 | Rust 现状 | 状态 |
|---|---|---|---|---|
| P0 | bbolt→redb 导入器（含 UserInfo 表） | map_service.go:124；new_service.go:96；main.go:371 | G1 `legacy-export`（`2c7f058`）+ R1 importer（JSONL v1、幂等/冲突/校验/自动导入） | **已对齐** |
| P1 | 三后端审核客户端（COS CI/阿里 Green/百度 AIP） | oss/ 的 UploadAndAudit* | oss/audit.rs 真实实现 + AuditRejected（P21）；图片已对齐。语音：Go 的 `images.UploadBase64RecordToServer` → `oss.UploadAndAuditRecord*` **无调用方**（死代码，全仓 grep 仅定义处），P44 的 `/uploadrecord` 走本地 `channel_temp`，与 Go handler 一致 | **已对齐（语音不移植死代码）** |
| P1 | `/getid` type 18/19 | getIDHandler.go:351-373；service.go:2394,2488 | storage list_identities/force_unbind + upload_http 18/19（P27；Go 自身缺陷在文档） | **已对齐** |
| P1 | lotus gRPC idmap 服务 | main.go:507；main_grpc.go | boot 起 IdMapService 服务端 + 优雅关闭；仅服务端起才隐藏 /getid（P28）；客户端模式未实现（缺口：Go 的 `UpdateVirtualValuev2` 等 lotus 远程分支因此不走 Rust，P47 登记） | **服务端已对齐** |
| P0 | WebUI 登录路径兼容 `/api/check-login-status` | webui/api.go:47 | 双路径并存同 handler + login isLoggedIn 契约 | **已对齐（P6a）** |
| P0 | `/uploadpic`、`/uploadpicv2`、`/uploadrecord`、`/getid` HTTP 端点 | main.go:516-525；server/uploadpic.go:37,64,186；server/getIDHandler.go:104,203-267 | upload_http.rs（15 用例；`/uploadpic*` 10 用例；`/uploadrecord` 5 用例：`channel_temp/<md5>.silk` + `server_dir:port`（不套 frp/stun）、403/400/413/429/500 同码同形、`oss_type` 不参与；getid 1~4/6/7/8~11/13/15~19 同码同形，5/12 走 idmap 重绑定（P47，与 gRPC RPC 5/6 同一套语义），14 依赖退役 bbolt `ids` 桶回 501；**请求体上限差异（登记）**：`DefaultBodyLimit` 硬限 32 MiB，超限显式回 413 `{"error":"request body too large"}`（P45 前该提取器错误被吞成空字段 → 写 0 字节 `.silk` 回 200），Go 的 `MaxMultipartMemory` 只是内存阈值，超限照样落盘解析） | **已对齐（P14/P24/P27/P44；gRPC 装配前 /getid 恒挂载）** |
| P1 | oss 阿里/百度/腾讯后端 | oss/{aliyun,baidu,tencent}.go | media/oss/（0~3 直传 + deferred 分派；审核开关 fail-closed） | **已对齐（P13）** |
| P1 | botstats + Prometheus `/metrics` | botstats/botstats.go；main.go:522,1140-1175 | botstats.rs redb + GET /metrics + sent/received 打点（P12/P12b） | **已对齐** |
| P1 | WebUI logs WS | webui/api.go:52-70；mylog WsHandler | logs.rs 环形缓冲 + WS（快照+实时+登录 401） | **已对齐（P16）** |
| P0 | idmap 解绑/重绑定 UpdateVirtualValue*/ForceUnbindID | service.go:1405,1654,2036,2394 | storage `update_virtual_value` / `update_virtual_value_pro` / `force_unbind` + gRPC RPC 5/6 + `/getid` type 5/12（P27/P47）：三段反查、exists（独立变体 `StorageError::AlreadyExists` → gRPC `AlreadyExists`）、`new=0` 扫删重复反查条目、提交后 bump+精确失效两个方向缓存；隔离模式 alias 按 redb 表结构映射为 `raw:<new>`→BE(old)。已登记差异：Go 的旧库双写 / `idmap_legacy_compat` 无对应（Rust 单库）；Go 的 lotus 远程分支（远端 gRPC 或自打 `/getid?type=5`）未移植 | **已对齐（P47+P48；P48：`new=0` 失效全部被删重复行的 head 缓存、`new!=0` 迁 `<old>:` 全家族（1:N）、exists 检查移入写事务、`/getid` type=5 文案 `不存在:<old>`；lotus 远程分支登记为缺口）** |
| P1 | WebUI 余 9 路由（config 写/accounts/status/logs WS/cookie 持久化） | webui/api.go:36-134；cookie.go:15 | P6a+P6b+P16：config 三方法/accounts/status/process/status/cookie.redb/logs WS（环形缓冲+实时推送） | **已对齐（P16）** |
| P1 | 图床后端 4/5/6/9/10 | imagehosting/hosting.go:45-63 | 4~10 全量 backend（P7）+ oss_type 分派接线（P11） | **已对齐** |
| P1 | acnode AC 自动机 | acnode/acnode.go:17-111 | core/acnode.rs（手写 AC、IN/OUT 双接入、词表回写） | **已对齐（P8）** |
| P1 | messagequeue（发送队列/限速） | messagequeue/queue.go:96-122 | queue.rs（令牌桶+重试，限速可注入；Go 队列死代码） | **已对齐（P10/P10b）** |
| P1 | botstats + Prometheus `/metrics` | botstats/botstats.go；main.go:522,1140-1175 | builtin.rs 6 指标 + `/metrics` + redb 统计 + received 打点（P12/P12b） | **已对齐** |
| P2 | unioncache / url 短链 / sys 全量 | unioncache/*；url/shorturl.go；sys/* | union_cache.rs 1:1 双向缓存（P38 起入站写 + markdown @ 反查已接，P42 补齐 `union_id` 入站切换与 inline search 读点）；short_url.rs redb 库 + 校验/301/恶意判定/自指 token 规则 + `/url` 两条路由（P36）；sys executable_name/SystemInfo → WebUI `/api/status` 扩展字段（P36；重启类不可移植，已文档化） | **已对齐（P30 库 / P36 接线 / P38 unioncache 读写接线 / P42 入站切换与 inline search）** |
| P2 | 视频/QR/SSRF 防护 | images/*；sys/* | media/lib.rs:108-118 未做清单 | 缺失 |
| — | idmap 核心/echo/config/热重载 | 见审计表 | 见审计表 | **已对齐**（Rust 多 `enable_at_everyone_limit` 1 键） |
| — | Settings 字段/accessor/restart 34 项 | structs.go 184 键；config.go:38-48 | 185 键（多 1）/ 1:1 accessor / diff.rs 34 项 | **已对齐** |
| — | 配置自愈 ensureConfigComplete | config.go:350 | config/complete.rs 文本补键回写（幂等/已有行零字节改动/块内注释带回/tmp+rename）；`startup.rs::load_config` 在配置载入前调用（P36，补键即生效不重启） | **已实现（P30）/已接线（P36）** |

## B. 出站 action 与消息链

### B1-3. action 全集 / 消息链特性 / 媒体链

**Action 全集**：Go 53 个唯一注册名（50 canonical + 3 别名）→ Rust **53/53 覆盖，缺失 0**；清单漂移：Rust `KNOWN_ACTIONS` 多 3 个 Go 从未注册的素材管理扩展（upload/get_list/delete_media_image），属 Rust 扩展（Medium，登记）。

**消息链特性矩阵**（对齐=已对齐）：reply/echo ✓；msg_type 0/2/6/7/8 ✓；markdown/keyboard ✓；card 部分（私聊也支持=扩展；本地 pic ✓ P29a）；input_notify ✓；stream/SSE ✓；qqmusic ✓（P29b：查询→markdown+键盘，失败回退分享文案）；forward ✓（保留 node 媒体）；raw ✓；wakeup/active ✓；SSM ✓（仅群入队同 Go）；懒消息/2000/seq ✓；at：AppID→BotID ✓（P15）、add_at_group ✓；auto_escape 刻意差异（Rust 尊重该字段）；array/string_ob11 ✓。

**媒体链**：local/url/base64 四类 ✓；多媒体段拆分 ✓（P1）；**>5MB 分片上传 ✓（P29a）**；**silk 转码/图片压缩 ✓（P29a）**；素材链 oss_type=7 ✓；图床 provider 4~10 ✓（P7）；图床上传重试 ✓（P7）；**card 本地 pic_url ✓（P29a）**；markdown 内嵌图替换 ✓；unknown_* 透传 ✓；qqmusic ✓（P29b：真实 markdown+键盘）。

### B4. CQ 码进出站

| 项 | Go | Rust | 状态 | 置信 |
|---|---|---|---|---|
| 转义口径 | 出站只转 `,` `]` `&`，不转 `[`（handlers/cqcode.go） | cqcode.rs:7-12,58-73 显式跟随 Go | 已对齐（刻意偏离规范） | High |
| 参数容错 | `:` 仅 stream 认、裸片段并入前一键值 | cqcode.rs:26-47 同口径；大小写不敏感 | 已对齐 | High |
| 未知码 | 原样保留文本 | Segment::raw_or_cq 逐字节透传 | 已对齐 | High |
| `voice` 别名 | voice≡record | 码表有 voice，OutboundPlan 归一需运行验证 | 部分 | Medium |
| at 顺序 | 纯 at 回退原文 | 缺 qq/反查失败原样透传 | 已对齐 | High |
| AppID→BotID | message_parser.go:1679-1682（do_not_replace_appid） | `/users/@me` + 出站替换 + `do_not_replace_appid`（P15） | **已对齐** | High |
| node 出站 | CQ 管道无 outbound node | cqcode.rs:775-780 outbound:false | 已对齐 | High |

### B5. 限速/队列/重试

| 项 | Go | Rust | 状态 | 置信 |
|---|---|---|---|---|
| 发送队列 | messagequeue/queue.go:96-122（Go 队列为**死代码**，活路径仅令牌桶+重试） | 无全局队列（与 Go 死代码同口径）；forward_send_interval + 令牌桶 + 3 次线性退避（P10/P10b） | **已对齐（按活路径）** | High |
| 令牌桶 | ratelimit.go:26-33（5qps/burst10，WaitWithTimeout 5s） | queue.rs 令牌桶，`DeferredDeps::limiter` 可注入（P10/P10b） | **已对齐** | High |
| 错误分类/重试 | retry_policy.go:18-30（22009/40034025/26）、默认 3 次 | 22009→SSM；40034025 无分支 | 部分 | High |
| get_g_list_delay | config.go:2520；逐群 sleep | group_admin.rs 每请求读快照、N-1 次异步 sleep | **已对齐（P2 `e1f822c`）** | High |
| SSM 队列 | send_group_msg.go:483-490 | deferred.rs:297-420 | 已对齐 | High |

### B6. 缺口汇总（按 severity）

| Sev | 缺口 | 建议补法 | 估量 |
|---|---|---|---|
| ~~High~~ | ~~>5MB 分片上传缺失（Rust 直接 1400）~~ | ~~runtime/media 补 init/分片/complete~~ | **已修（P29a）** |
| ~~High~~ | ~~silk 转码 + 图片压缩未接线~~ | ~~复用 gensokyo-media/silk.rs + compress.rs~~ | **已修（P29a）** |
| ~~High~~ | ~~qqmusic 降级为分享文案~~ | ~~歌曲查询 + markdown 模板 + 键盘（music_prefix）~~ | **已修（P29b）** |
| High | 多媒体段无 markdown 时 1400 | 照 Go 逐媒体段各发一条 | ~~120-200 行~~ **已修（P1 `3fcb426`）** |
| ~~High~~ | ~~card 本地 pic_url 未上传~~ | ~~照 resolveCardPic 接素材链~~ | **已修（P29a）** |
| Medium | 图床 provider 仅 7/8，缺 4/5/6/9/10 | 每 provider 一个 backend（hosting trait 已有） | ~~500-700 行~~ **已修（P7，待提交；接线 P11）** |
| Medium | 图床上传无重试 | 3 次 1s/2s 退避 | ~~40-60 行~~ **已修（P7 retry.rs）** |
| Medium | 无发送队列/5qps/发送间隔 | runtime 全局令牌桶 + 串行队列 | 200-300 行 |
| Medium | get_g_list_delay 缺失 | get_group_list/成员列表逐群 sleep | ~~30-50 行~~ **已修（P2 `e1f822c`）** |
| ~~Medium~~ | ~~add_at_group 无消费者、AppID→BotID 缺失~~ | ~~入站/文本渲染各补一处~~ | **已修（P15 + 入站 add_at_group）** |
| Low | lib.rs:50 陈旧文档（SSM/#62 已落地仍写未接线） | 改注释 | 5 行 |
| Low | KNOWN_ACTIONS 3 个幽灵项 | 对齐 Go 实际注册表 | 10 行 |

**推荐顺序**：媒体四件套（分片、silk/压缩、多媒体拆条、card pic）→ qqmusic → 图床 provider → 队列/限速 → 其余。

## D. 运行效率（Rust 侧特化，非 Go 对照项）

| # | 措施 | 实现 | 状态 |
|---|---|---|---|
| P31 | 事件只序列化一次 | `PublishedEvent{Arc<Event> + 发布时一次 serde_json}`，订阅通道 `Arc<PublishedEvent>`；反向 WS / 正向 WS / post_url 复用同一份 JSON（字节等价 2 用例锁定） | **已落地** |
| P31 | release 口径 | `lto = "fat"`、`panic = "abort"`（release）；bench 只设 lto（cargo 忽略 bench 的 panic） | **已落地** |
| P34 | PGO 流水线 | `xtask pgo`（generate → 负载 → profdata merge → use → 复测 + `pgo-report.md`）；Windows GNU 缺 profiler runtime（E0463），需 Linux/msvc 宿主；BOLT Linux-only | **工具就绪 / 宿主受限** |
| P35 | redb 批量写 | `IdMapWriteBatch` / `with_write_batch`；群消息 5→2、私聊 3→2 事务；实测 42×（4000 次身份写） | **已落地** |
| P35 | ID 热查询缓存 | 有界 LRU（默认 4096 条/60s，可关）、提交后代际失效；实测 2.8× | **已落地** |
| — | tracing 编译期裁剪 | 用户裁定：release 保留全部日志级别 | **不做** |
| 排队 | mimalloc 分配器（P33）、共享 HTTP 传输 | 分配器需新依赖（已批准）；共享 transport 收益仅启动期，P31 评估后暂缓 | 待做 |

## E. 接线缺口（P36）

P36 是「库 / 函数已落地、生产调用点未接」的收口波次。A 表里的「待接线 P36」即下列各项：

| # | 项 | 现状 | 接线点 / 验收 | P36 状态 |
|---|---|---|---|---|
| 1 | 短链 HTTP 路由 | `gensokyo-storage::short_url` 库 + 校验 / 恶意判定 / token 规则齐全（P30） | 主 listener 挂 `POST /url`（400 或 `{"shortURL":…}`）与 `GET /url/:shortURL`（301 / 404） | **已接线**：`runtime/http_api.rs::short_url_router`（urlencoded + multipart、base64 归一、301 手写 `Location`），`boot.rs::attach_short_url` 打开 `short-urls.redb` 并挂主 listener（主 listener 缺席时落 HTTP API 端口）；库打不开只 warn、两条路由回 500（Go 是 `url.init()` panic）。**挂载位置差异（只登记，不改）**：Go 挂在**永远存在**的主 server（`main.go:566-567` 注册、`:602` 起 `0.0.0.0:serverPort`，只过 `Recovery`），Rust 绑 WS listener（`enable_ws_server=true` 才有），WS 关闭时落 HTTP API 端口，两者都关则 `/url` 不存在——Rust 没有「恒存在的主 server」这一层，为一个路由新起监听器不合最小改动口径。**另两处已知差异（只登记，不改）**：301 无响应体（Go `c.Redirect` 带 `<a href=…>Moved Permanently</a>` 正文）；`/url/a/b` 等多段路径在 Rust 落 WebUI SPA 回退（Go 路由不匹配 → 404） |
| 2 | `ensure_config_complete` 启动调用 | `gensokyo-config::complete` 文本补键齐全（P30） | `gensokyo-bin/src/startup.rs` 在 `ensure_config_file` 之后调用并处理 `CompletionReport`（Go `config.go:350` 的写后重启改为由 runtime 决定） | **已接线**：`startup.rs::load_config` 在 `ConfigManager::new` **之前**调用（本次启动即生效，不重启）；`changed()` → warn 继续；读失败 / 语法坏 → warn 且文件原样。**追加前提**：只有顶层 `settings:` 块映射能承接缩进两格的子项，其余形态（settings 正文如 `port: "5700"`、裸 `version: 1`、`settings: null` / 流式映射）返回 `CompleteError::NoSettings`，调用点 warn 且**绝不写盘**（否则会写出非法 YAML，进程直接起不来）；空文件 / 纯注释文件先补 `version: 1\nsettings:\n` 外壳再追加 |
| 3 | unioncache 生产接线 | `gensokyo-storage::union_cache` 双向映射齐全（P30） | 入站群消息写 `id→union`；`mdutil` / inline search 反查 `union→id`（Go 调用点见模块头） | **写 + markdown 读 + 入站切换已接（P38 / P42）**：`Inbound::group_message` 在建映射的同一位置 `store(id, union_openid)`（`ProcessGroupMessage.go:55` / `ProcessGroupNormalMessage.go:46`，两种群消息形态共用，`GROUP_MEMBER_*` 负载本身没有该字段），随后按 `union_id` 切换上报身份（`ProcessGroupMessage.go:58-64` / `ProcessGroupNormalMessage.go:49-54`：`author.id ← union_openid`，string 形态 `platform` 随之为 `unionqq`）；出站 `deferred::compose_markdown` 在 `RenderMarkdown` 之前按 Go 顺序反查（`send_group_msg.go:1531-1533`），`union_id` 关闭时整段跳过；`Inbound::interaction` 接 `ProcessInlineSearch.go:112-116` 的读点。进程级共享实例 `UnionCache::shared()`（对位 Go 包级 `sync.Map`）。**两处登记差异**：① `union_id` 打开但 `union_openid` 为空时，Go 无条件把 `author.id` 置空（`ProcessGroupMessage.go:60`），Rust 保留原 id——空 ID 事件对客户端只是噪声；② inline search 的 `user_id` / `real_user_id` 仍是原始 OpenID：Go 的 `fromuid` 在 `ProcessInlineSearch.go:39`（替换之前）取值，`:112-116` 只改回显的 `data.group_member_openid`，`RealUserID = fromuid`（`:134`）读的是那个副本，Rust 照抄实际行为 |
| 4 | `SystemInfo` / `executable_name` 消费者 | `gensokyo-core::sys` 只读信息齐全（P30） | WebUI `/api/status` 投影这两个字段（对齐 Go `sys.GetExecutableName` 的用途） | **已接线**：`webui/status.rs` 增 `os` / `arch` / `executable` / `local_ip` 四个**Rust 扩展字段**（Go `handleSysInfo` 无此四项，前端多键无影响）；`GetPublicIP` 需外呼 `/` 固定第三方站点，**不做**（`banner::console_urls` 用 `server_dir` 拼公网地址，无需外呼） |
| 5 | 存量短码导入（impact 评审） | Go 短码在 `gensokyo.db` bbolt bucket `shortURLs`；Rust 用独立 `short-urls.redb` | 复用既有 bbolt 读取通路 | **缺口（不迁移）**：仓内唯一的导入通路是 G1 `legacy-export` 的 idmap JSONL（`importer/mod.rs` 明确「不读 bbolt」，`kind` 表里没有短链行）；Go 侧 `legacy-export` 也**不导出** `shortURLs`（全仓 grep 仅 `url/shorturl.go` 命中该 bucket）。按边界不为它新写 bbolt 解析器或加依赖——**老短码不迁移，需重新生成** |
| 6 | 短链 token 校验自指 bug | Go `isValidToken`（`shorturl.go:303-324`）比较 `hex(HMAC(pwd, token)) == token` | 保持 Go 实际行为 | **已照抄**：`http_api::is_valid_token`（实现随 P36 接线迁到 `runtime/http_api.rs`），`lotus_password` 非空时创建一律 400（Go 客户端送的是 `HMAC(pwd, 长链)`，两端口径从来不一致）。修复要两端同时改，不在 P36 |
| 7 | `cleanupDuplicateSettings` | Go `config.go:717-792` 在 `LoadConfig` 早期删重复 `settings:`、补缺失顶层键 | 配置载入前清洗 | **已移植（P43；P45 收口评审 1C/2M/3m；P46 复评 2 条非阻断建议收口）**：`loader::cleanup_duplicate_settings` 逐行对齐四步（`\r\n`/`\r`→`\n`；第二个**列 0** 的 `settings:` 起整段截断；首个内容行（跳过空行 / 注释 / 文档标记 `---`·`...` / null 标量）既不是 `settings:` 也不以 `version:` 开头时重建 `version: 1` + `settings:` 外壳、其余行只整体 `"  "+line.trim_end()`（**保留行内相对缩进**）、注释原样、空行与文档标记丢弃）。`loader::load` 读盘后清洗、**内容有变化才原子写回**并 `tracing::warn!` 一次（写失败只 warn、继续用内存结果，Go `config.go:61-64` 同样 `_ = os.WriteFile`）。`startup::load_config` 在 `ensure_config_complete` **之前**跑同一个 `cleanup_config_file`，故「重复 settings + 缺键」一次启动内都修好。**与 Go 的差异（登记）**：① 写回走 `loader::write_atomic`（同目录 `config.yml.<pid>.tmp` → rename，rename 前搬权限位；加载期清洗、`complete.rs` 补键、`template.rs` 首启生成三处共用同一份，全仓只有这一份 temp + rename），Go 是原地 `os.WriteFile`——换来「写一半不留半截配置」；② BOM 先剥开再原样接回，Go 会把 `\ufeff` 当内容行触发重建；③ Go 只比长度决定写回，这里比内容（长度相同但内容不同的清洗也会落盘）；④ 告警走 `warn_early`（`logging::init` 之前直接写 stderr，之后交 `tracing`）而非 `fmt.Printf`——早期三条路径（模板生成 / 清洗 / 补键）的告警在生产上不再被丢弃（P46）；⑤ 重建时 `settings:` **无条件**补——Go 只在全文没有任何 `settings:` 行时才补，`settings:` 掉在正文后面的形态（`port: "1"\nsettings:`）它会省掉外壳又不保留该行，写出 `version: 1\n  port: "1"` 这种非法 YAML；这里以「写回的文件必须仍可解析」优先；⑥ **重建只整体 +2、保留行内相对缩进**（Go `config.go:782-784` 用 `TrimSpace`，多级结构被拍平——`cos:` 的 `secret_id` / `bucket` 静默丢失）；⑦ **列 0 的文档标记 `---` / `...` 与 null 标量不当内容**（Go 会把 `---\nversion: 1` 判成结构损坏，重建出 `settings:\n  ---\n  port: …` 这种非法 YAML，进程起不来；块标量内容按 YAML 规则必缩进，缩进的 `---` 列号不为 0，重建时原样保留——P46 更正此前"会丢掉块标量内 `---`"的表述）；⑧ **截断只认列 0 的 `settings:`**（Go 的 `TrimSpace` 口径会被正文里缩进的 `settings:` 键骗到，把后面整段截掉）；⑨ **写回前先 `parse_str` 自检**——清洗结果解析不了就放弃写回（原文可解析则退回原文并 warn），Go 无条件写。**与 P40 守护的交互**：`NoSettings` 分支只覆盖清洗修不出的形态（`settings: null` / 流式映射 / 裸 `version: 1`）；「settings 正文」现在先被清洗重建成带 `settings:` 外壳，补键照常生效，P36 定的「正文形态不改文件」随之作废 |

（本文由三分域审计汇总；A/B/C/D 表与 §E 状态已按 R4g 波次回写，时点见 `docs/rust-m0.md`。）

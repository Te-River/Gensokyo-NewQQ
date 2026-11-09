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
| P1 | 三后端审核客户端（COS CI/阿里 Green/百度 AIP） | oss/ 的 UploadAndAudit* | oss/audit.rs 真实实现 + AuditRejected（P21）；语音仅 /uploadrecord 使用（端点未移植，缺口在册） | **图片已对齐/语音缺口** |
| P1 | `/getid` type 18/19 | getIDHandler.go:351-373；service.go:2394,2488 | storage list_identities/force_unbind + upload_http 18/19（P27；Go 自身缺陷在文档） | **已对齐** |
| P1 | lotus gRPC idmap 服务 | main.go:507；main_grpc.go | boot 起 IdMapService 服务端 + 优雅关闭；仅服务端起才隐藏 /getid（P28）；客户端模式未实现（缺口） | **服务端已对齐** |
| P0 | WebUI 登录路径兼容 `/api/check-login-status` | webui/api.go:47 | 双路径并存同 handler + login isLoggedIn 契约 | **已对齐（P6a）** |
| P0 | `/uploadpic`、`/uploadpicv2`、`/getid` HTTP 端点 | main.go:508,523-524；server/uploadpic.go；server/getIDHandler.go:104,203-267 | upload_http.rs（10 用例；getid 1~4/6/7/8~11/13/15/16 同码同形，5/12/14/18/19 依赖 bbolt 迁移期结构回 501） | **已对齐（P14/P24；gRPC 装配前 /getid 恒挂载）** |
| P1 | oss 阿里/百度/腾讯后端 | oss/{aliyun,baidu,tencent}.go | media/oss/（0~3 直传 + deferred 分派；审核开关 fail-closed） | **已对齐（P13）** |
| P1 | botstats + Prometheus `/metrics` | botstats/botstats.go；main.go:522,1140-1175 | botstats.rs redb + GET /metrics + sent/received 打点（P12/P12b） | **已对齐** |
| P1 | WebUI logs WS | webui/api.go:52-70；mylog WsHandler | logs.rs 环形缓冲 + WS（快照+实时+登录 401） | **已对齐（P16）** |
| P0 | idmap 解绑/重绑定 UpdateVirtualValue*/ForceUnbindID | service.go:1405,1654,2036,2394 | grpc.rs 2 RPC unimplemented | 缺失 |
| P1 | WebUI 余 9 路由（config 写/accounts/status/logs WS/cookie 持久化） | webui/api.go:36-134；cookie.go:15 | P6a+P6b：config 三方法/accounts/status/process/status/cookie.redb；余 logs WS 与 PUT /api/{uin} | **大部分对齐（待提交；logs WS 在册）** |
| P1 | 图床后端 4/5/6/9/10 | imagehosting/hosting.go:45-63 | 仅 ChatGlm(7)/Ukaka(8) | 部分 |
| P1 | acnode AC 自动机 | acnode/acnode.go:17-111 | core/acnode.rs（手写 AC、IN/OUT 双接入、词表回写） | **已对齐（P8，待提交）** |
| P1 | messagequeue（发送队列/限速） | messagequeue/queue.go:96-122 | queue.rs（令牌桶+重试，限速可注入；Go 队列死代码） | **已对齐（P10/P10b）** |
| P1 | botstats + Prometheus `/metrics` | botstats/botstats.go；main.go:522,1140-1175 | builtin.rs 返回 0；无端点 | 缺失 |
| P2 | unioncache / url 短链 / sys 全量 | unioncache/*；url/shorturl.go；sys/* | 无 / 仅 local_ip | 缺失 |
| P2 | 视频/QR/SSRF 防护 | images/*；sys/* | media/lib.rs:108-118 未做清单 | 缺失 |
| — | idmap 核心/echo/config/热重载 | 见审计表 | 见审计表 | **已对齐**（Rust 多 `enable_at_everyone_limit` 1 键） |
| — | Settings 字段/accessor/restart 34 项 | structs.go 184 键；config.go:38-48 | 185 键（多 1）/ 1:1 accessor / diff.rs 34 项 | **已对齐** |
| — | 配置自愈 ensureConfigComplete | config.go:350 | 无（仅首次模板生成） | 缺失 |

## B. 出站 action 与消息链

### B1-3. action 全集 / 消息链特性 / 媒体链

**Action 全集**：Go 53 个唯一注册名（50 canonical + 3 别名）→ Rust **53/53 覆盖，缺失 0**；清单漂移：Rust `KNOWN_ACTIONS` 多 3 个 Go 从未注册的素材管理扩展（upload/get_list/delete_media_image），属 Rust 扩展（Medium，登记）。

**消息链特性矩阵**（对齐=已对齐）：reply/echo ✓；msg_type 0/2/6/7/8 ✓；markdown/keyboard ✓；card 部分（私聊也支持=扩展；本地 pic 未上传）；input_notify ✓；stream/SSE ✓；**qqmusic 缺失**（降级分享文案，无歌曲查询/模板/键盘）；forward ✓（保留 node 媒体）；raw ✓；wakeup/active ✓；SSM ✓（仅群入队同 Go）；懒消息/2000/seq ✓；**at：AppID→BotID 未实现、add_at_group 无消费者**；auto_escape 刻意差异（Rust 尊重该字段）；array/string_ob11 ✓。

**媒体链**：local/url/base64 四类 ✓；多媒体段拆分 ✓（P1）；**>5MB 分片上传 ✓（P29a）**；**silk 转码/图片压缩 ✓（P29a）**；素材链 oss_type=7 ✓；图床 provider 4~10 ✓（P7）；图床上传重试 ✓（P7）；**card 本地 pic_url ✓（P29a）**；markdown 内嵌图替换 ✓；unknown_* 透传 ✓；qqmusic 仍降级文本（待 P29b）。

### B4. CQ 码进出站

| 项 | Go | Rust | 状态 | 置信 |
|---|---|---|---|---|
| 转义口径 | 出站只转 `,` `]` `&`，不转 `[`（handlers/cqcode.go） | cqcode.rs:7-12,58-73 显式跟随 Go | 已对齐（刻意偏离规范） | High |
| 参数容错 | `:` 仅 stream 认、裸片段并入前一键值 | cqcode.rs:26-47 同口径；大小写不敏感 | 已对齐 | High |
| 未知码 | 原样保留文本 | Segment::raw_or_cq 逐字节透传 | 已对齐 | High |
| `voice` 别名 | voice≡record | 码表有 voice，OutboundPlan 归一需运行验证 | 部分 | Medium |
| at 顺序 | 纯 at 回退原文 | 缺 qq/反查失败原样透传 | 已对齐 | High |
| AppID→BotID | message_parser.go:1679-1682（do_not_replace_appid） | 全仓无实现 | **缺失** | High |
| node 出站 | CQ 管道无 outbound node | cqcode.rs:775-780 outbound:false | 已对齐 | High |

### B5. 限速/队列/重试

| 项 | Go | Rust | 状态 | 置信 |
|---|---|---|---|---|
| 发送队列 | messagequeue/queue.go:96-122（GetSendDelay≤0 取 300ms、失败 3 次线性退避） | 无全局队列；仅 forward_send_interval | **缺失** | High |
| 令牌桶 | ratelimit.go:26-33（5qps/burst10，WaitWithTimeout 5s） | 无 | **缺失** | High |
| 错误分类/重试 | retry_policy.go:18-30（22009/40034025/26）、默认 3 次 | 22009→SSM；40034025 无分支 | 部分 | High |
| get_g_list_delay | config.go:2520；逐群 sleep | group_admin.rs 每请求读快照、N-1 次异步 sleep | **已对齐（P2 `e1f822c`）** | High |
| SSM 队列 | send_group_msg.go:483-490 | deferred.rs:297-420 | 已对齐 | High |

### B6. 缺口汇总（按 severity）

| Sev | 缺口 | 建议补法 | 估量 |
|---|---|---|---|
| High | >5MB 分片上传缺失（Rust 直接 1400） | runtime/media 补 init/分片/complete，deferred 按需切换 | 400-600 行 |
| High | silk 转码 + 图片压缩未接线 | 复用 gensokyo-media/silk.rs + compress.rs | 200-300 行 |
| High | qqmusic 降级为分享文案 | 歌曲查询 + markdown 模板 + 键盘（music_prefix） | 250-350 行 |
| High | 多媒体段无 markdown 时 1400 | 照 Go 逐媒体段各发一条 | ~~120-200 行~~ **已修（P1 `3fcb426`）** |
| High | card 本地 pic_url 未上传 | 照 resolveCardPic 接素材链 | 60-100 行 |
| Medium | 图床 provider 仅 7/8，缺 4/5/6/9/10 | 每 provider 一个 backend（hosting trait 已有） | ~~500-700 行~~ **已修（P7，待提交；接线 P11）** |
| Medium | 图床上传无重试 | 3 次 1s/2s 退避 | ~~40-60 行~~ **已修（P7 retry.rs）** |
| Medium | 无发送队列/5qps/发送间隔 | runtime 全局令牌桶 + 串行队列 | 200-300 行 |
| Medium | get_g_list_delay 缺失 | get_group_list/成员列表逐群 sleep | ~~30-50 行~~ **已修（P2 `e1f822c`）** |
| Medium | add_at_group 无消费者、AppID→BotID 缺失 | 入站/文本渲染各补一处 | 40-60 行 |
| Low | lib.rs:50 陈旧文档（SSM/#62 已落地仍写未接线） | 改注释 | 5 行 |
| Low | KNOWN_ACTIONS 3 个幽灵项 | 对齐 Go 实际注册表 | 10 行 |

**推荐顺序**：媒体四件套（分片、silk/压缩、多媒体拆条、card pic）→ qqmusic → 图床 provider → 队列/限速 → 其余。

（本文由三分域审计汇总；A/C 表的「状态」列以 HEAD 8c69129 时点为准。）

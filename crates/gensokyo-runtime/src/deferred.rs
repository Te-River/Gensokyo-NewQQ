//! deferred 计划的执行：`SendRequest` → `MessageToCreate` → QQ 上游。
//!
//! `gensokyo-actions` 的 `send_*` handler 只产出计划（`data.deferred == true`），本模块负责把
//! 计划变成真实发送：[`Dispatcher`](crate::dispatcher::Dispatcher) 识别出三种握手形状——单条
//! [`SendRequest`]（[`execute`]）、SSE 直发 [`SseRequest`]（[`execute_sse`]）、逐条重发
//! [`ForwardRequest`]（[`execute_forward`]）——收件人还原成上游 OpenID、消息合成
//! [`MessageToCreate`]、再按 target 分派到
//! [`QqApi::send_group_message`](gensokyo_actions::QqApi::send_group_message) /
//! [`QqApi::send_c2c_message`](gensokyo_actions::QqApi::send_c2c_message)。
//!
//! # 支持范围
//!
//! | 段 | 落到 | 说明 |
//! |----|------|------|
//! | 文本 | `content` | 按 CQ 重新分词，`[CQ:at,qq=…]` 转 v2 的 `<@openid>`（虚拟号经 idmap 反查）；缺 `qq` 的 at 段（`[CQ:at]` / `[CQ:at ]` / `[CQ:at,foo]` / `[CQ:at,qq=]`）与反查失败的数字虚拟号都原样透传（后者只 `warn`）；`at` 后与键值两侧允许空白（`[CQ:at , qq = 1]` 照常解析）；`auto_escape = true` 时原样发送、不解析 |
//! | `[CQ:at,qq=all]` / `[CQ:at,qq=everyone]` / 字面 `<qqbot-at-everyone />` | `<qqbot-at-everyone />` | 仅群聊，且受「每群每自然日 1 次」限制，见下节「@全员」 |
//! | `markdown` | `markdown` + `msg_type=2` | 只取 `content`：模板字段（`template_id` / `custom_template_id`）已废弃，出站不构造；文本被丢弃、`card` 让位；正文里的本地图片按 `oss_type` 换引用：`7` → `qqbot-media://` 素材库，`1~3` → 内置云 OSS 对象 URL，`4~6/8~10` → 图床 CDN URL（[`resolve_markdown_images`]）；与媒体同现时先发这一条，媒体另起 `msg_type=7` 条 |
//! | `card` | `card` + `msg_type=8` | 群聊与单聊同构；`{type:"tuwen", content:{title,description,url,pic_url}}`，四字段校验见 [`compose_card`]；`pic_url` 是本地路径时先上传成公网 URL（Go `resolveCardPic`，失败保留原值）；与媒体同现时卡片先发、媒体另起 `msg_type=7` 条 |
//! | 图片 / 语音 / 视频 / 文件 | `media` + `msg_type=7` | 见下节「媒体」 |
//! | `[CQ:reply]` | `msg_id` | 虚拟号先经 idmap / echo 还原成真实消息 ID；还原失败 `1400`，不静默丢 |
//! | `keyboard` | `keyboard` | 与 text / markdown / card 同发，不改变 `msg_type`（Go `applyGroupMarkdownKeyboard` 口径）；`data` 必须是 JSON 对象，否则 `1400` |
//! | `input_notify` | `input_notify` + `msg_type=6` | 仅单聊；不承载正文 / 媒体，与其它内容同条出现 `1400` |
//! | `stream` | `stream` + `msg_type=2` | 仅单聊；正文进 `markdown.content`，`stream.state` 取 1（start / mid）/ 10（finish），见 [`compose_stream`] |
//! | `music` / `qqmusic` | `markdown` + `keyboard` + `msg_type=2` | 取第一首：查 QQ 音乐歌曲详情（Go `FetchSongDetail`，走媒体下载 seam）→ 原生 markdown（封面 `T002R180x180M000{album_mid}.jpg` + 曲名 / 歌手）→ 键盘「立刻播放 / 我也要点歌」（第二个按钮的 `data` 是 `music_prefix` + 空格）；查询失败降级成分享文案并 `warn`（Go 直接丢整条消息） |
//! | `unknown` / `unknown_image` / `unknown_record` | 不支持 | `1400`（请求侧段类型），不静默丢 |
//! | `active` / `wakeup` | 忽略 | 控制型标记，不影响本条合成；`send_private_msg_wakeup` 的
//!   召回语义由 [`SendRequest::wakeup`] 承载，[`execute`] 置 `is_wakeup` 并强制不带回复 ID；
//!   `[CQ:active]` 清空 `msg_id` / `event_id` 强制主动推送（见下节「被动回复身份」）；
//!   SSM（`code:22009` 入队补发）见下节「SSM 补发」 |
//!
//! ## 消息体拆分与优先级
//!
//! `msg_type` 只能取一个值，因此 markdown / card / rich media 互为排他：同时出现时按
//! markdown > card > media 取更高优先级的那个，**其余媒体各起一条**发送
//! （Go `handlers/send_group_msg.go:606-613` 先发 markdown 条、`:742-783` 卡片条、
//! `:787-802` 再把每个媒体段各发一条）：首条 `msg_type=2` / `8`（正文走 `markdown.content`
//! 或卡片，`content` 留空），随后每个媒体段一条 `msg_type=7`（单条消息仍只有一个 `media`
//! 槽）。纯多段媒体（无 markdown、无卡片）同样一段一条，首条带走正文 / 键盘。markdown 与
//! card 同现时 card 不参与校验（避免一张坏卡片拦住合法的 markdown），与 Go 的「markdown
//! 胜 card」一致。
//!
//! `keyboard` 是加性字段，不参与上面的优先级：文本 / markdown / card 谁胜出都照样挂上去；
//! 拆条时只挂在首条，后续媒体条不带（Go 只在 markdown 那轮挂键盘，
//! `send_group_msg.go:831-843`）。`[CQ:reply]` 同样只落在首条（`:659-680`；循环里只在
//! `key == "markdown"` 时补 reply，`:815`、`send_private_msg.go:625`）。
//! `input_notify`（`msg_type=6`）与 `stream`（`msg_type=2` + `stream`）则**独占**整条消息：
//! 它们带不了正文 / 媒体，与其它内容同条出现一律 `1400`，而不是丢掉另一部分。
//! `music` 自成一条消息（Go 在 foundItems 循环里单独发 `qqmusic`）：与 markdown / card / 媒体
//! 同条出现时，主消息照旧按上面的优先级选，音乐另起一条跟在后面，不参与让位。只有音乐一条时
//! 它就是主消息；此时再带独立 `[CQ:keyboard]` 会与音乐自带键盘抢同一个字段（Go 另发一条），
//! Rust 侧 `1400` 而不是静默丢用户键盘。
//!
//! ## @全员 与 @用户
//!
//! 归一在 [`render_text`]（文本）与 [`compose_markdown`]（markdown 内容）各做一次，都走
//! [`at_everyone::normalize`]：`[CQ:at,qq=all]` 与 `[CQ:at,qq=everyone]` 产出官方标签
//! `<qqbot-at-everyone />`，字面标签原样保留；归一后重复的标签由
//! [`at_everyone::dedupe_tag`] 收敛成一个（字面标签与 CQ 写法可能同时出现）。文本与 markdown
//! 的 @用户 走两套标签：
//! 文本仍是旧式 `<@openid>`，markdown 是官方 `<qqbot-at-user id="openid" />`（数字虚拟号
//! 必须经 idmap 反查，失败 `1400`）。缺 `qq` 的 at 段两条路径口径不同：markdown 正文
//! `1400`（否则会原样发一段不可渲染的 CQ 码），文本路径原样透传（与 Go 一致）；数字虚拟号
//! 反查失败同样只在 markdown `1400`，文本路径保留原文并 `warn`。发送前扫最终载荷：
//!
//! - 单聊出现标签 → `1400`（官方只允许群聊，静默丢弃不可接受）；
//! - 群聊出现标签 → 先占当天的名额（[`at_everyone::reserve`]，落 idmap config 的
//!   `at_everyone` section），**发送失败回滚**，超限 `1400`；
//! - 文本 + markdown 同现时标签迁移进 markdown 正文（否则随 `content` 清空丢失）；
//! - 卡片与 @全员 同现 → `1400`（`msg_type=8` 会清空正文，标签无处安放）；
//! - `enable_at_everyone_limit=false` 时既不拦也不计数；未装配 idmap 时只 `warn` 放行
//!   （没有计数载体，不能把消息卡死在这里）。
//!
//! ## 媒体
//!
//! `MediaRef` 的三种来源（`local_*` / `url_*` / `base64_*`）都会先经
//! [`gensokyo_media::Fetcher`] 取字节（见 [`MediaRuntime`](crate::media::MediaRuntime)），再调
//! [`QqApi::upload_group_file`](gensokyo_actions::QqApi::upload_group_file) /
//! [`QqApi::upload_c2c_file`](gensokyo_actions::QqApi::upload_c2c_file)
//! 换成 `file_info`，最后填进 [`MessageToCreate::media`]，并按 Go
//! `handlers/send_group_msg.go` 的口径把 `msg_type` 置为 [`msg_type::RICH_MEDIA`]（7）。
//!
//! 限制（细节见 [`crate::media`]）：
//! - **单条消息只有一个 `media` 槽**：多段媒体一段一条发送（见上节「消息体拆分与优先级」），
//!   既不丢段、也不塞进同一条；
//! - **`file_data` 直传上限 5MB**（[`MAX_MEDIA_BYTES`]）：超过改走分片上传——预上传 →
//!   逐片预签名 PUT + `upload_part_finish` → `files` 合并（Go `chunkedUpload`）。任一步失败
//!   按上游错误返回，**不**降级回 `file_data` 直传（Go 同）；下载硬上限 200MB
//!   （[`crate::media::MAX_UPLOAD_BYTES`]）；
//! - [`MediaRef::Unknown`] 视为「已经拿到上游 `file_info`」，直接透传、不再上传（Go 的
//!   `unknown_*` 回退口径）；
//! - **本地 / base64 来源**先做发送前处理（Go 条件，URL 来源原样）：图片按
//!   `image_sizelimit`（KB，`0` = 不压缩）压缩；语音不是 AMR / SILK 时经 `ffmpeg` + `pts`
//!   转码（工具缺失报 1500）；视频 / 文件不处理。
//!
//! ## QQ 音乐
//!
//! `[CQ:music,type=qq,id=…]` / `qqmusic` 只取第一首（Go `foundItems["qqmusic"][0]`），先 GET
//! `u.y.qq.com` 的 `get_song_detail_yqq`（URL 与 Go `FetchSongDetail` 一致，响应上限 2MB），
//! 读 `songinfo.data.track_info` 的 `mid` / `name` / `singer[0].name` / `album.mid`。请求失败
//! 或曲目不存在只降级成「分享一首 QQ 音乐：{id}」正文并 `warn`：Go 在这里返回 nil、整条消息
//! 被丢掉，Rust 保留分享落点。模板消息（`custom_template_id` + `params`）已按出站口径废弃，
//! 音乐卡片改用等价的**原生 markdown**：封面图 + 曲名 / 歌手两行（对应 Go 的 `img_url` 与
//! `text_end`），封面尺寸取 CDN 固定的 180x180。
//!
//! ## 被动回复身份与 msg_seq
//!
//! 每条计划的 `msg_id` / `event_id` 由 [`resolve_passive`] 按 Go `send_group_msg.go:207-331`
//! / `send_private_msg.go:319-355` 的先后顺序一次性定好，主条与后续媒体条共用同一份
//! （`send_group_msg.go:801` 的媒体循环用同一个 `messageID`）：懒消息池（群有 `user_id`
//! 上下文查 v2、否则 v1；私聊只有 v1）→ 请求 `echo` 当键查 `msg_id` 表 → 收件人键查表 →
//! 群「无用户上下文」清空 → `[CQ:active]` 双清 → `"2000"` 回填 `event_id`。`string_ob11`
//! 决定 `event_id` 查虚拟号键还是真实 OpenID 键（群；私聊恒用真实 OpenID）。未装配
//! [`EchoStore`] 时整段跳过，行为与接线前一致。
//!
//! 每条实际发出的消息再取一个 `msg_seq`：键就是解析出的 `msg_id`、用**递增**版
//! [`EchoStore::next_mapping_seq`]（Go `IncrementMappingSeq(messageID)`）。同一回复链的多段
//! 回复复用同一 `msg_id`，seq 连续递增才不会撞 `40054005` 去重（Issue #19）；`msg_id` 与
//! `event_id` 始终二选一（有 `msg_id` 就不带 `event_id`，`[CQ:reply]` 反查出的真实 ID 同样
//! 优先于被动 `msg_id`）。
//!
//! ## echo 回写
//!
//! 发送成功后把上游 `message_id` 写进 [`EchoStore::store_cache_in_memory`]（Go
//! `StoreCacheInMemory` 的行号双向映射），后续 `[CQ:reply,id=<虚拟号>]` 才能反查到它；
//! 回写失败只 `warn`，不改变本次发送结果。
//!
//! ## SSM 补发（`code:22009`）
//!
//! 群聊发送失败且上游 `code == 22009`（消息被频控）时，把本次要发的消息体序列化成 JSON
//! 放进 [`EchoStore::push_global_stack`]（Go `send_group_msg.go:483-490` 的
//! `IsQQError(err, 22009)` 分支）；私聊**不入队**（Go `send_private_msg.go` 只记日志）。
//!
//! 补发要求三件事同时成立：群聊、`lazy_message_id` 开启、懒消息池取到的值不是 `"2000"`
//! （主动推送不给补发机会，Go `send_group_msg.go:207-229`），且发生在**本条消息发送之前**
//! （`:284-288`）。条数取配置 `AMsgRetryAsPMsg_Count`（结构体零值 0，模板给 30），只补发与本次群
//! 相同的积压（`SendStackMessages`，`:2662-2697`）：每条覆盖 `msg_id`（懒池那个值）与
//! `msg_seq`（每次 `NextMappingSeq` 一个新值），**不动** `event_id`；成功从队列删除，失败留在
//! 队列里，再次 `22009` 会重新入队。删除按下标**倒序**做——Go 边遍历边正序删会跳过元素
//! （在册缺陷，不复刻）。
//!
//! ## wakeup 终局回推
//!
//! 唤醒计划（`SendRequest::wakeup`）过了收件人还原之后的终局——成功，或合成 / 上传 /
//! 发送任一步失败——都经 [`EventHub`](crate::event_hub::EventHub) 回推一条
//! `notice_type = c2c_wakeup_resp` 事件（对齐 Go `send_private_msg_wakeup.go` 的
//! `sendWakeupNotice`：每个终局出口推一次，收件人还原失败的早退不推，Go 同点也不推）。
//! 一计划一 notice：Go 的逐条消息推送口径由计划切分承担（拆条只在有多个媒体段时发生，
//! wakeup 召回本身不带媒体，推送计数不受影响）。
//! JSON 逐字段对齐 Go `WakeupResponseNotice`，见 [`publish_wakeup_notice`]；总线未装配
//! 时静默跳过，回推不影响 API 回执。
//!
//! # 错误口径
//!
//! 请求侧问题（媒体来源 / 大小 / 类型、reply 映射缺失、不支持的段）返回 [`Error::Protocol`]
//! → `retcode 1400`；下载、上传与发送失败（[`Error::Network`] / [`Error::Upstream`]）→ `1500`。
//! 映射在 [`crate::dispatcher`] 里完成。

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::Once;
use std::time::Duration;

use gensokyo_actions::{
    ActionContext, ForwardRequest, MessageOut, SendRequest, SendTarget, SseRequest,
    upload_bytes_to_media_store,
};
use gensokyo_core::{Error, Ob11Id, Result};
use gensokyo_media::source::FILE_PREFIX;
use gensokyo_media::{HostingKind, ImageKind, MediaSource, OssKind, sniff_image};

use gensokyo_protocol::cqcode;
use gensokyo_protocol::event::NoticeEvent;
use gensokyo_protocol::outbound::{InputNotifyPayload, StreamKind, StreamPayload};
use gensokyo_protocol::{Event, MediaRef, OutboundPlan};
use gensokyo_qq::dto::message::{MessageCard, MessageCardContent};
use gensokyo_qq::dto::{
    FileUpload, FileUploadMergeRequest, FileUploadPartFinishRequest, FileUploadPrepareRequest,
    InputNotify, Markdown, Media, MessageResponse, MessageToCreate, StreamSse, file_type, msg_type,
};
use gensokyo_storage::EchoStore;
use gensokyo_storage::echo::{DEFAULT_MESSAGE_ID, MessageGroupPair};
use serde_json::{Value, json};

use crate::at_everyone::{self, Reservation};
use crate::event_hub::EventHub;
use crate::hosting::HostingRuntime;
use crate::media::{MAX_MEDIA_BYTES, MediaRuntime};
use crate::queue;

/// handler 响应 `data` 里的 deferred 标记（与 `gensokyo_actions::DEFERRED_FIELD` 同值）。
pub const DEFERRED_FIELD: &str = gensokyo_actions::DEFERRED_FIELD;

/// `data` 是否是一份 deferred 计划（`{"deferred": true, …}`）。
#[must_use]
pub fn is_deferred(data: &Value) -> bool {
    data.get(DEFERRED_FIELD).and_then(Value::as_bool) == Some(true)
}

/// deferred 执行链的额外依赖（[`ActionContext`] 之外的运行时句柄）。
#[derive(Debug, Clone, Copy)]
pub struct DeferredDeps<'a> {
    /// 媒体下载器；生产用 [`MediaRuntime::default`]，单测注入假传输。
    pub media: &'a MediaRuntime,
    /// 上传出口（`oss_type` 1~3 内置云 OSS 与 4~6/8~10 图床的 markdown 本地图片）；生产用
    /// [`crate::hosting::shared`]，单测注入假传输。
    pub hosting: &'a HostingRuntime,
    /// echo 存储：发送成功后回写 `message_id`，`[CQ:reply]` 反查虚拟号也走它。
    pub echo: Option<&'a EchoStore>,
    /// 事件总线：wakeup 发送到达终局后回推 `c2c_wakeup_resp` notice（对齐 Go
    /// `sendWakeupNotice`）。未装配时静默跳过，不报错。
    pub hub: Option<&'a EventHub>,
    /// 合并转发逐条重发之间的间隔（Go 固定 500ms；生产走
    /// [`DEFAULT_FORWARD_SEND_INTERVAL`]，测试可注入 [`Duration::ZERO`] 免真睡，评审 ③）。
    pub forward_send_interval: Duration,
    /// 群消息发送前的令牌桶：生产走 [`queue::RateLimiter::shared`]（进程级 5qps / burst 10，
    /// Go `GetRateLimiter`），`None` 即不限速。
    ///
    /// 测试必须传 `None`：全量并行用例共享同一个进程级桶，突发额度打满后
    /// [`queue::WAIT_TIMEOUT`] 等不到令牌、按失败尝试走重试，而重试会重取 `msg_seq`，
    /// 把「首条消息」的序号推高。
    pub limiter: Option<&'a queue::RateLimiter>,
}

/// 执行一条 deferred 计划，返回上游消息 ID。
///
/// 一条计划可能发出多条消息（markdown + 媒体拆条，见 [`compose`]）：每条都回写 echo，返回值
/// 取**最后一条**的结果（Go 的 `resp` / `err` 逐条覆盖，`send_group_msg.go:987-1005`）。
/// `plan.wakeup` 的终局（成功或失败）另经 `deps.hub` 回推一条 `c2c_wakeup_resp` notice（见
/// [`publish_wakeup_notice`]）。
///
/// # Errors
/// 收件人无法反查成 OpenID（[`ActionContext::resolve_send_target`]）、消息含不支持的媒体 /
/// 段、markdown 缺 `content` 或其中的 `[CQ:at,qq=数字]` 反查失败、卡片字段非法、卡片与
/// @全员 同现、单聊出现 @全员、同群同日第二次 @全员、reply 无法还原成真实消息 ID
/// （以上 [`Error::Protocol`] → `1400`），或下载 / 上传 / 上游发送失败
/// （[`Error::Network`] / [`Error::Upstream`] → `1500`）。
pub async fn execute(
    context: &ActionContext,
    plan: &SendRequest,
    deps: &DeferredDeps<'_>,
) -> Result<String> {
    let openid = context.resolve_send_target(&plan.target)?;
    let result = execute_inner(context, plan, deps, openid.clone()).await;
    // 唤醒终局回推（对齐 Go `sendWakeupNotice`）：合成 / 上传 / 发送任一步失败都推 failed；
    // 收件人还原失败的早退不推（Go 该早退同样不推，口径一致）。
    if plan.wakeup {
        publish_wakeup_notice(context, deps, &openid, &result);
    }
    result
}

/// [`execute`] 的落体：合成 → 占 @全员 名额 → 逐条发送 → echo 回写。
///
/// 拆条发送（markdown + 媒体）时单条失败不回滚已发、也不阻断后续：每条各自发送、失败逐条
/// 告警，回执取**最后一条**的结果（Go `send_group_msg.go:787-802` 每个 foundItems 项独立发送、
/// `:987-1005` 用最后一次的 `resp` / `err` 回执）。
async fn execute_inner(
    context: &ActionContext,
    plan: &SendRequest,
    deps: &DeferredDeps<'_>,
    openid: String,
) -> Result<String> {
    // 互动召回（Go `send_private_msg_wakeup.go`）：`is_wakeup` 与 `msg_id` / `event_id` 互斥，
    // 回复目标强制不带，正文合成完统一置标记。
    let (mut primary, mut extras) = if plan.wakeup {
        let mut cleared = plan.message.clone();
        cleared.plan.reply_msg_id = None;
        compose(context, &plan.target, &openid, &cleared, deps).await?
    } else {
        compose(context, &plan.target, &openid, &plan.message, deps).await?
    };
    if plan.wakeup {
        primary.is_wakeup = true;
        for extra in &mut extras {
            extra.is_wakeup = true;
        }
    }
    // 被动回复身份（`msg_id` / `event_id`）与逐条 `msg_seq`：Go 在发送前一次性定好，主条与
    // 后续媒体条共用同一份（`send_group_msg.go:801` 的媒体循环用同一个 messageID）。
    let passive = if plan.wakeup {
        Passive::default()
    } else {
        resolve_passive(context, plan, deps, &openid)
    };
    // SSM 补发（Go `send_group_msg.go:207-288`）：群聊、`lazy_message_id` 开启、且懒消息池取到
    // 的不是 `"2000"` 时，先把栈里积压的同群消息补发掉，再发本条。补发用懒池那个值当被动身份
    // ——Go 此时还没做后面那几步映射表兜底（`:291-331`）。
    if !plan.wakeup
        && let (SendTarget::Group(_), Some(echo)) = (&plan.target, deps.echo)
        && context.config().snapshot().get_lazy_message_id()
    {
        let lazy = lazy_message_id(echo, plan, plan.target.id(), &openid);
        if lazy != DEFAULT_MESSAGE_ID {
            send_stack_messages(context, echo, &openid, &lazy).await;
        }
    }
    stamp_message(&mut primary, &passive, deps.echo);
    for extra in &mut extras {
        stamp_message(extra, &passive, deps.echo);
    }
    // 先占当天的 @全员 名额再发送：并发两条 @全员 只有一条能占住。标签只可能在首条
    // （正文在 markdown 条），后续媒体条没有正文。
    let mut reservation = reserve_at_everyone(context, &plan.target, &openid, &primary)?;
    let mut outcome = match send_body(context, &plan.target, &openid, primary, deps).await {
        Ok(message_id) => Ok(message_id),
        Err(err) => {
            // 发送失败要回滚，不能让一次失败吃掉当天的名额
            if let (Some(reservation), Some(idmap)) = (reservation.take(), context.idmap()) {
                reservation.rollback(idmap);
            }
            Err(err)
        }
    };
    for (offset, body) in extras.into_iter().enumerate() {
        // 首条占 1，媒体条从 2 起编号，日志里好对账
        let number = offset + 2;
        match send_body(context, &plan.target, &openid, body, deps).await {
            Ok(message_id) => outcome = Ok(message_id),
            Err(err) => {
                tracing::warn!(message = number, error = %err, "deferred 拆条发送有一条失败，继续发后续");
                outcome = Err(err);
            }
        }
    }
    outcome
}

/// 按 target 发出单条消息，成功后回写 echo；失败原样上抛，由调用方决定回滚还是继续。
///
/// 走 Go 的**活路径**（`messagequeue/queue.go` 的 worker 在 Go 里没有启动点，见 [`crate::queue`]
/// 模块文档）：
/// - 群消息每次尝试前先过 [`DeferredDeps::limiter`] 的令牌桶（生产 5qps / burst 10、最多等
///   [`queue::WAIT_TIMEOUT`]，Go `send_group_msg.go:3065`），私聊不过桶（Go
///   `postC2CMessageWithRetry` 同）；
/// - 投递超时最多尝试 [`queue::MAX_ATTEMPTS`] 次、线性退避 [`queue::backoff`]，每次重试前重取
///   `msg_seq`（Go `postGroupMessageWithRetry` 在循环里调 `echo.NextMappingSeq`）；
/// - 其余错误（频控 `22009`、事件过期等）**一次即返回**，群消息照旧落 SSM 队列
///   （见 [`enqueue_ssm`]）——重试与 SSM 是两条互不影响的路径。
async fn send_body(
    context: &ActionContext,
    target: &SendTarget,
    openid: &str,
    mut message: MessageToCreate,
    deps: &DeferredDeps<'_>,
) -> Result<String> {
    let echo = deps.echo;
    // 入队要用原始消息体，而发送会把它移走，所以群聊在发送前先序列化一份
    let payload = match target {
        SendTarget::Group(_) => serde_json::to_string(&message).ok(),
        SendTarget::Private(_) => None,
    };
    let mut attempt: u32 = 0;
    let sent = loop {
        let permitted = match (target, deps.limiter) {
            (SendTarget::Group(_), Some(limiter)) => {
                limiter.wait_with_timeout(queue::WAIT_TIMEOUT).await
            }
            _ => true,
        };
        let outcome = if permitted {
            dispatch_body(context, target, openid, message.clone()).await
        } else {
            // Go 在这里只记日志并 `continue`（耗尽后返回 nil 错误，调用方会踩空 resp）；
            // 这里按一次失败尝试处理，语义更明确
            Err(Error::Network("群消息发送等待限流超时".to_owned()))
        };
        match outcome {
            Ok(response) => break Ok(response),
            Err(err) if queue::should_retry(&err, attempt) => {
                attempt += 1;
                refresh_msg_seq(echo, &mut message);
                tokio::time::sleep(queue::backoff(attempt)).await;
            }
            Err(err) => break Err(err),
        }
    };
    let response = match sent {
        Ok(response) => response,
        Err(err) => {
            // 私聊不入队（Go `send_private_msg.go` 在该分支只记日志）
            if let (SendTarget::Group(_), Some(echo)) = (target, echo)
                && is_ssm_retry(&err)
            {
                enqueue_ssm(echo, openid, payload, &err);
            }
            return Err(err);
        }
    };
    record_sent_message(echo, &response.id);
    Ok(response.id)
}

/// 单次上游发送（群 / 私聊各一个端点）。
async fn dispatch_body(
    context: &ActionContext,
    target: &SendTarget,
    openid: &str,
    message: MessageToCreate,
) -> Result<MessageResponse> {
    match target {
        SendTarget::Group(_) => {
            context
                .api()
                .send_group_message(openid.to_owned(), message)
                .await
        }
        SendTarget::Private(_) => {
            context
                .api()
                .send_c2c_message(openid.to_owned(), message)
                .await
        }
    }
}

/// 重试前重取 `msg_seq`（Go `postGroupMessageWithRetry` 每次尝试都 `echo.NextMappingSeq(MsgID)`）。
///
/// 只动 `msg_seq`：`msg_id` / `event_id` 沿用本次发送定好的被动身份，否则重试会换掉回复目标。
fn refresh_msg_seq(echo: Option<&EchoStore>, message: &mut MessageToCreate) {
    let Some(echo) = echo else {
        return;
    };
    let key = message.msg_id.clone().unwrap_or_default();
    message.msg_seq = Some(u32::try_from(echo.next_mapping_seq(&key)).unwrap_or_default());
}

/// 上游业务码是否是需要 SSM 补发的频控（Go `retry_policy.go` 的 `IsQQError(err, 22009)`）。
fn is_ssm_retry(err: &Error) -> bool {
    matches!(err, Error::Upstream { code: 22009, .. })
}

/// 把发送失败的群消息体放进 SSM 队列（Go `send_group_msg.go:483-490` 的入队分支）。
///
/// 队列元素只承载不解释的字符串载荷，这里存的就是 [`MessageToCreate`] 的 JSON——
/// 补发时反序列化回来，`msg_id` / `msg_seq` 再按当时那条被动回复重写。
fn enqueue_ssm(echo: &EchoStore, group_openid: &str, payload: Option<String>, err: &Error) {
    let Some(payload) = payload else {
        tracing::warn!(group = %group_openid, error = %err, "SSM 入队跳过：消息体无法序列化");
        return;
    };
    echo.push_global_stack(MessageGroupPair {
        payload: Some(payload),
        ..MessageGroupPair::new(group_openid)
    });
}

/// 补发 SSM 队列里积压的**本群**消息（Go `SendStackMessages`，`send_group_msg.go:2662-2697`）。
///
/// `count` 取配置 `AMsgRetryAsPMsg_Count`（结构体零值 0，模板给 30）；`pop` 只取副本，发送
/// 成功才删。每条覆盖 `msg_id` / `msg_seq`（每次 [`EchoStore::next_mapping_seq`] 一个新值），**不动** `event_id`
/// （Go 同；上游仍可能因此回报 40034025，是否刷新另议）。失败只告警留在队列里，再次 `22009`
/// 则重新入队（Go 同——`pop` 未删除，重入队等于多留一份副本，下次补发会再试一遍）。
///
/// 删除按下标**倒序**做：队列在遍历期间可能被重新入队追加（只会加在尾部，不影响前面的下标），
/// 倒序删则前面的元素下标始终有效；Go 的正序删除会跳过元素，是在册缺陷。
async fn send_stack_messages(
    context: &ActionContext,
    echo: &EchoStore,
    group_openid: &str,
    msg_id: &str,
) {
    let count = usize::try_from(context.config().snapshot().get_ato_p_count()).unwrap_or(0);
    let pairs = echo.pop_global_stack_multi(count);
    let mut sent = Vec::new();
    for (index, pair) in pairs.iter().enumerate() {
        // 别的群的积压留给它自己的被动回复补发
        if pair.group != group_openid {
            continue;
        }
        let Some(mut body) = pair
            .payload
            .as_deref()
            .and_then(|payload| serde_json::from_str::<MessageToCreate>(payload).ok())
        else {
            // 载荷是发送失败时自己写进去的，解不出来说明已损坏；留着只会每次补发都失败
            tracing::warn!(
                correlation_id = %pair.correlation_id,
                "SSM 消息体缺失或无法解析，丢弃该条"
            );
            sent.push(index);
            continue;
        };
        body.msg_seq = Some(u32::try_from(echo.next_mapping_seq(msg_id)).unwrap_or_default());
        // Go 直接赋 `MsgID = messageid`；空值时那边靠 `omitempty` 整键消失，这里同样落 `None`
        body.msg_id = (!msg_id.is_empty()).then(|| msg_id.to_owned());
        match context
            .api()
            .send_group_message(pair.group.clone(), body)
            .await
        {
            Ok(_) => sent.push(index),
            Err(err) => {
                tracing::warn!(
                    correlation_id = %pair.correlation_id,
                    error = %err,
                    "SSM 补发失败，留在队列里等下次被动回复"
                );
                if is_ssm_retry(&err) {
                    echo.push_global_stack(pair.clone());
                }
            }
        }
    }
    for index in sent.into_iter().rev() {
        echo.remove_from_global_stack(index);
    }
}

/// 唤醒终局回推的 `notice_type`（Go `WakeupResponseNotice.NoticeType`）。
const C2C_WAKEUP_RESP: &str = "c2c_wakeup_resp";

/// 把 wakeup 发送的终局结果回推事件总线（逐字段对齐 Go
/// `send_private_msg_wakeup.go:sendWakeupNotice`）。
///
/// JSON 与 Go `WakeupResponseNotice` 逐键一致：`post_type` / `notice_type` / `user_id`
/// （恒数字 0，防客户端 unmarshal 炸）/ `time` / `self_id` / `message_id`（成功且有真实
/// 上游 ID 时为字符串）走 [`NoticeEvent`] 原生字段；`real_user_id` / `status` /
/// `error_msg` 进 `extra`（flatten 后同为顶层键）。成功无 `error_msg`、失败无
/// `message_id`，对齐 Go 的 omitempty。`self_id` 取配置 `app_id`——Go 传的是
/// `config.GetAppID()`（不含 `use_uin` 分支）。总线未装配（`deps.hub` 为 `None`）时
/// 静默跳过；broadcast 发布不阻塞、无订阅者也不报错，不影响 API 回执（Go 推送失败仅
/// log 同口径）。
fn publish_wakeup_notice(
    context: &ActionContext,
    deps: &DeferredDeps<'_>,
    openid: &str,
    result: &Result<String>,
) {
    let Some(hub) = deps.hub else {
        return;
    };
    let mut extra = serde_json::Map::new();
    let mut message_id = None;
    let mut status = "success";
    match result {
        // 成功但上游未返回 message_id（空串）：整键消失，对齐 Go `omitempty`
        Ok(id) if !id.is_empty() => message_id = Some(Ob11Id::str(id.clone())),
        Ok(_) => {}
        Err(err) => {
            status = "failed";
            extra.insert("error_msg".to_owned(), Value::String(err.to_string()));
        }
    }
    extra.insert("real_user_id".to_owned(), Value::String(openid.to_owned()));
    extra.insert("status".to_owned(), Value::String(status.to_owned()));
    hub.publish(Event::Notice(NoticeEvent {
        time: crate::inbound::now_unix(),
        self_id: Ob11Id::num(
            i64::try_from(context.config().snapshot().get_app_id()).unwrap_or(i64::MAX),
        ),
        notice_type: C2C_WAKEUP_RESP.to_owned(),
        sub_type: None,
        group_id: None,
        user_id: Some(Ob11Id::num(0)),
        operator_id: None,
        target_id: None,
        message_id,
        duration: None,
        file: None,
        card_new: None,
        card_old: None,
        honor_type: None,
        online: None,
        client: None,
        extra,
    }));
}

/// 执行一条 SSE 计划（`send_private_msg_sse`），返回上游消息 ID。
///
/// 载荷是规划侧拼好的 `MessageSSE` 形态体（`msg_type=2` + `stream`），与普通 C2C 同端点
/// 直发（Go `PostC2CMessageSSE` 就是 `POST /v2/users/{user_id}/messages`），不再走
/// [`compose`]——Go 的 SSE 分支也不做 CQ 解析，正文原样进 `markdown.content`。
///
/// # Errors
/// 收件人无法反查成 OpenID 或发送失败，口径同 [`execute`]。
pub async fn execute_sse(
    context: &ActionContext,
    request: &SseRequest,
    deps: &DeferredDeps<'_>,
) -> Result<String> {
    let openid = context.resolve_send_target(&request.target)?;
    let response = context
        .api()
        .send_c2c_message(openid, request.message_sse.clone())
        .await?;
    record_sent_message(deps.echo, &response.id);
    Ok(response.id)
}

/// Go `send_group_forward_msg.go:73`：逐条重发之间睡 500ms（上游速率保护）的生产默认值。
///
/// 实际间隔经 [`DeferredDeps::forward_send_interval`] 注入（评审 ③）：dispatcher 缺省用
/// 这个常量，测试传 `Duration::ZERO` 免真睡。
pub const DEFAULT_FORWARD_SEND_INTERVAL: Duration = Duration::from_millis(500);

/// 一条合并转发的执行结果：成功的消息 ID 与失败清单（`index` 是**原 node 序号**，供调用方
/// 回查具体是哪个 node 失败；计划下标与其不同——坏形状 / 空正文 node 会被跳过，评审 ①）。
#[derive(Debug, Clone, Default)]
pub struct ForwardOutcome {
    /// 发送成功的上游消息 ID（按发送顺序）。
    pub message_ids: Vec<String>,
    /// 失败清单：已发送的不回滚，失败的逐条列出（含规划层解析失败的 node）。
    pub failed: Vec<FailedSend>,
}

/// 一条失败的转发。
#[derive(Debug, Clone)]
pub struct FailedSend {
    /// 原 node 序号（`messages` 数组下标，非计划下标）。
    pub index: usize,
    /// 失败原因（上游错误文本，或规划层的消息段解析错误）。
    pub error: String,
}

/// 执行一条合并转发计划：逐 node 循环调用 [`execute`]（Go 同款「逐条重发」口径）。
///
/// 单条失败**不打断后续**、已发送的不回滚（Go 的 `HandleSendGroupMsg` 逐条独立处理，
/// 整体回执恒定 ok）；相邻两次发送之间按 [`DeferredDeps::forward_send_interval`] 限速。
/// 规划层解析失败的 node（[`ForwardRequest::parse_failed`]）也并入同一份失败清单，
/// 回执按原 node 序号排序。
pub async fn execute_forward(
    context: &ActionContext,
    request: &ForwardRequest,
    deps: &DeferredDeps<'_>,
) -> ForwardOutcome {
    let mut outcome = ForwardOutcome {
        message_ids: Vec::new(),
        failed: request
            .parse_failed
            .iter()
            .map(|item| FailedSend {
                index: item.node_index,
                error: item.error.clone(),
            })
            .collect(),
    };
    for (position, plan) in request.forward.iter().enumerate() {
        if position > 0 {
            tokio::time::sleep(deps.forward_send_interval).await;
        }
        match execute(context, &plan.request, deps).await {
            Ok(message_id) => outcome.message_ids.push(message_id),
            Err(err) => outcome.failed.push(FailedSend {
                index: plan.node_index,
                error: err.to_string(),
            }),
        }
    }
    // 解析失败与发送失败分两路插入：统一按原 node 序号排，回执顺序稳定
    outcome.failed.sort_by_key(|item| item.index);
    outcome
}

/// 消息里是否带 @全员 标签（扫 `content` 与 `markdown.content`）。
fn has_at_everyone(message: &MessageToCreate) -> bool {
    message
        .content
        .as_deref()
        .is_some_and(at_everyone::contains_tag)
        || message
            .markdown
            .as_ref()
            .and_then(|markdown| markdown.content.as_deref())
            .is_some_and(at_everyone::contains_tag)
}

/// 群聊带 @全员 时占当天名额；没有标签直接返回 `None`。
///
/// # Errors
/// 单聊出现标签、同群同日第二次都返回 [`Error::Protocol`]（`1400`）；写 idmap 失败
/// 返回 [`Error::Storage`]（读失败按「未占用」放行，见 [`at_everyone::reserve`]）。
fn reserve_at_everyone(
    context: &ActionContext,
    target: &SendTarget,
    openid: &str,
    message: &MessageToCreate,
) -> Result<Option<Reservation>> {
    if !has_at_everyone(message) {
        return Ok(None);
    }
    if !matches!(target, SendTarget::Group(_)) {
        return Err(Error::Protocol(
            "@全员 只支持群聊：单聊消息不能包含 <qqbot-at-everyone />".to_owned(),
        ));
    }
    if !context.config().snapshot().get_enable_at_everyone_limit() {
        return Ok(None);
    }
    let Some(idmap) = context.idmap() else {
        tracing::warn!("@全员 限流未生效：未装配 idmap，无法写入每日计数");
        return Ok(None);
    };
    Ok(Some(at_everyone::reserve(idmap, openid)?))
}

/// 发送成功后的 echo 回写（Go `StoreCacheInMemory`）：失败只告警，不改变发送结果。
fn record_sent_message(echo: Option<&EchoStore>, message_id: &str) {
    let Some(echo) = echo else {
        return;
    };
    if message_id.is_empty() {
        tracing::warn!("deferred 发送成功但上游未返回 message_id，跳过 echo 回写");
        return;
    }
    if let Err(err) = echo.store_cache_in_memory(message_id) {
        tracing::warn!(
            message_id = %message_id,
            error = %err,
            "deferred 发送成功后写 echo 缓存失败"
        );
    }
}

/// 被动回复身份：Go `send_group_msg.go:207-331` / `send_private_msg.go:319-355` 解析出的
/// `messageID` / `eventID`，对同一次发送的每条消息生效。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Passive {
    /// 被动回复的 `msg_id`（懒消息池 / echo 映射表的命中值）；`None` 即主动推送。
    msg_id: Option<String>,
    /// 主动消息的 `event_id`；与 `msg_id` 互斥（有前者就不带后者）。
    event_id: Option<String>,
}

/// 按 Go 的先后顺序确定本次发送的 `msg_id` / `event_id`（语义见 [`Passive`]）。
///
/// 顺序（群 / 私聊同构，差异逐条标注）：
/// 1. 懒消息池：群有 `user_id` 上下文查 v2，否则 v1；私聊只有 v1（Go `GetLazyMessagesId*`）。
/// 2. 池里没命中 → 请求 `echo` 当键查 `msg_id` 表（Go `GetMsgIDByKey(echoStr)`）。
/// 3. 仍空 → 「群 × 用户」/ 收件人键查表（Go `GetMessageIDByUseridAndGroupid` /
///    `GetMessageIDByUseridOrGroupid`）。
/// 4. 群特例：没有 `user_id` 上下文（Rust 侧也没有 CQ 派生的 `event_id`）时，缓存里的
///    `msg_id` 已过期 → 清空走主动推送（Go `send_group_msg.go:308-312`）。
/// 5. `[CQ:active]` 标记：`msg_id` / `event_id` 双清（强制主动推送）。
/// 6. 结果是 `"2000"`：`msg_id` 清空、回填可用的 `event_id`，尽量不消耗主动次数。
///
/// 未装配 echo（[`DeferredDeps::echo`] 为 `None`）时整段跳过，行为与接线前一致。
fn resolve_passive(
    context: &ActionContext,
    plan: &SendRequest,
    deps: &DeferredDeps<'_>,
    openid: &str,
) -> Passive {
    let Some(echo) = deps.echo else {
        return Passive::default();
    };
    let settings = context.config().snapshot();
    let app_id = settings.get_app_id().to_string();
    let given = plan.target.id();
    let user = plan.echo_context.user_id.as_deref();
    let group = matches!(plan.target, SendTarget::Group(_));

    let mut msg_id = String::new();
    if settings.get_lazy_message_id() {
        msg_id = lazy_message_id(echo, plan, given, openid);
    }
    if msg_id.is_empty()
        && let Some(key) = plan.echo_context.echo_key.as_deref()
    {
        msg_id = echo.get_msg_id_by_key(key);
    }
    if msg_id.is_empty() {
        msg_id = match (group, user) {
            // 群 × 用户：入站按虚拟号写键，客户端传的也正是它；原始 OpenID 兜底一次
            (true, Some(user)) => first_message_id(
                echo,
                &[
                    format!("{app_id}_{given}_{user}"),
                    format!("{app_id}_{openid}_{user}"),
                ],
            ),
            _ => first_message_id(
                echo,
                &[format!("{app_id}_{given}"), format!("{app_id}_{openid}")],
            ),
        };
    }
    if group && !msg_id.is_empty() && user.is_none() {
        msg_id.clear();
    }

    // `[CQ:active]`：强制主动推送。Go 私聊把这一步放在 "2000" 分支之后，但主动标记本来就会
    // 连 `event_id` 一起清空，先后顺序的最终结果相同。
    let active = plan
        .message
        .plan
        .active
        .as_ref()
        .is_some_and(|active| active.active);
    let mut event_id = String::new();
    if active {
        msg_id.clear();
    } else if msg_id == DEFAULT_MESSAGE_ID {
        msg_id.clear();
        event_id = event_id_for(
            echo,
            &app_id,
            group,
            given,
            openid,
            settings.get_string_ob11(),
        );
    }

    Passive {
        msg_id: (!msg_id.is_empty()).then_some(msg_id),
        event_id: (!event_id.is_empty()).then_some(event_id),
    }
}

/// 按顺序取第一个非空的 `msg_id` 表值（键已含 `app_id` 前缀）。
fn first_message_id(echo: &EchoStore, keys: &[String]) -> String {
    keys.iter()
        .map(|key| echo.get_msg_id_by_key(key))
        .find(|id| !id.is_empty())
        .unwrap_or_default()
}

/// 懒消息池取值：群有 `user_id` 上下文查 v2、否则 v1；私聊只有 v1。
///
/// 池按客户端可见的 ID 写键（虚拟号；`string_action` 时再补原始 OpenID 键），所以先用调用方
/// 传来的原样 ID，没命中且它与真实 OpenID 不同时再试一次。
fn lazy_message_id(echo: &EchoStore, plan: &SendRequest, given: &str, openid: &str) -> String {
    let user = plan.echo_context.user_id.as_deref();
    let group = matches!(plan.target, SendTarget::Group(_));
    let lookup = |key: &str| match (group, user) {
        (true, Some(user)) => echo.get_lazy_messages_id_v2(key, user),
        _ => echo.get_lazy_messages_id(key),
    };
    let id = lookup(given);
    if id != DEFAULT_MESSAGE_ID || openid == given {
        return id;
    }
    lookup(openid)
}

/// Go `GetEventIDByUseridOrGroupid{,-v2}`：群按 `string_ob11` 选键（关闭时先虚拟号、再原始
/// OpenID 兜底），私聊一律用真实 OpenID（Go 那边已先把 `user_id` 反查成 OpenID，`IsOpenID`
/// 恒真，等价于 v2 分支）。
fn event_id_for(
    echo: &EchoStore,
    app_id: &str,
    group: bool,
    given: &str,
    openid: &str,
    string_ob11: bool,
) -> String {
    let mut keys = Vec::with_capacity(2);
    if group && !string_ob11 && given != openid {
        keys.push(format!("{app_id}_{given}"));
    }
    keys.push(format!("{app_id}_{openid}"));
    keys.iter()
        .map(|key| echo.get_event_id_by_key(key))
        .find(|id| !id.is_empty())
        .unwrap_or_default()
}

/// 把被动身份写到一条待发消息上，并给它分一个 `msg_seq`。
///
/// - `msg_id` 已存在（`[CQ:reply]` 反查出的真实消息 ID）时不覆盖，并清掉 `event_id`
///   （Go 的「二选一」：回复分支写 `MsgID` 后紧跟 `EventID = ""`）。
/// - `msg_seq` 用**递增**版（Go `IncrementMappingSeq(messageID)`），键就是解析出的 `msg_id`：
///   同一回复链的多段回复复用同一 `msg_id`，seq 连续递增才不会撞 40054005（Issue #19）。
///   自带 `msg_seq` 的形态（流式分片按自己的序号走）保持原值。
fn stamp_message(body: &mut MessageToCreate, passive: &Passive, echo: Option<&EchoStore>) {
    if body.msg_id.is_none() {
        body.msg_id.clone_from(&passive.msg_id);
    }
    if body.msg_id.is_some() {
        body.event_id = None;
    } else if body.event_id.is_none() {
        body.event_id.clone_from(&passive.event_id);
    }
    let Some(echo) = echo else {
        return;
    };
    if body.msg_seq.is_none() {
        let key = passive.msg_id.as_deref().unwrap_or_default();
        body.msg_seq = Some(u32::try_from(echo.next_mapping_seq(key)).unwrap_or_default());
    }
}

/// 把一条待发送消息合成 **一条或多条** [`MessageToCreate`]（文本 + reply + media + 键盘 /
/// 输入状态 / 流式）。
///
/// 返回 `(首条, 后续媒体条)`：markdown / 卡片与媒体同现、或多段纯媒体时，首条带走
/// `msg_type` 与正文 / 键盘，其余每个媒体段各一条 `msg_type=7`（Go
/// `handlers/send_group_msg.go:606-613` 先发 markdown / card 条、`:787-802` 再把每个媒体段
/// 各发一条）；单段媒体只有首条、后续为空。
///
/// `openid` 由调用方从 `target` 还原：媒体上传要按群 / C2C 选端点，两者都要它。
///
/// # Errors
/// markdown 正文里的 `[CQ:at]` 缺 `qq`、markdown 里的 `at` 虚拟号无法反查、reply 无法还原、
/// 单聊专属段（`input_notify` / `stream`）出现在群聊、它们与其它内容同条出现，或消息含本阶段
/// 仍不支持的段。
pub async fn compose(
    context: &ActionContext,
    target: &SendTarget,
    openid: &str,
    message: &MessageOut,
    deps: &DeferredDeps<'_>,
) -> Result<(MessageToCreate, Vec<MessageToCreate>)> {
    if let Some(kind) = unsupported_segment(&message.plan) {
        return Err(Error::Protocol(format!(
            "deferred 发送暂不支持 {kind} 段：{message:?}"
        )));
    }
    let mut out = MessageToCreate::default();
    // Go：`[CQ:reply]` → `msg_id`（被动回复）。事件 ID 不在计划里，不必与 `event_id` 互斥。
    if let Some(reply) = &message.plan.reply_msg_id {
        out.msg_id = Some(resolve_reply(context, deps.echo, reply)?);
    }

    // 单聊专属段独占整条消息：msg_type=6 / 流式都带不了正文与媒体
    if let Some(notify) = &message.plan.input_notify {
        return compose_input_notify(target, &message.plan, notify, message, out.msg_id)
            .map(|body| (body, Vec::new()));
    }
    if let Some(stream) = &message.plan.stream {
        return compose_stream(context, target, &message.plan, stream, message, out.msg_id)
            .map(|body| (body, Vec::new()));
    }

    let mut rendered = render_text(context, message)?;
    // QQ 音乐：先查曲目再拼消息（Go `parseQQMuiscMDData`）。查询失败只降级成分享文案
    // （Go 那边返回 nil、整条消息被丢掉），并 `warn` 留下线索。
    let mut music = None;
    if let Some(id) = message.plan.qqmusic.first() {
        match compose_music(context, deps, id).await {
            Ok(body) => music = Some(body),
            Err(err) => {
                tracing::warn!(music = %id, error = %err, "QQ 音乐查询失败，降级为分享文案");
                rendered = music_share_text(&rendered, id);
            }
        }
    }
    if let Some(mut markdown) = compose_markdown(context, &message.plan, &rendered)? {
        // 正文里的本地图片换引用（Go `ResolveMarkdownImages`）；失败保留原引用
        if let Some(content) = markdown.content.take() {
            markdown.content = Some(resolve_markdown_images(context, deps.hosting, &content).await);
        }
        // markdown 胜 card，文本让位：msg_type=2 只认 markdown.content
        out.content = None;
        out.msg_type = Some(msg_type::MARKDOWN);
        out.markdown = Some(markdown);
        out.keyboard = compose_keyboard(&message.plan)?;
        // 其余媒体各一条 msg_type=7：单条消息只有一个 media 槽，只能拆；音乐同样自成一条
        let mut extras: Vec<MessageToCreate> = music.into_iter().collect();
        extras.extend(compose_media_extras(context, target, openid, &message.plan, deps).await?);
        return Ok((out, extras));
    }
    if let Some(card) = compose_card(context, deps.hosting, &message.plan, &rendered).await? {
        // 卡片独占正文：官方要求 msg_type=8 时 content 为空
        out.content = None;
        out.msg_type = Some(msg_type::CARD);
        out.card = Some(card);
        out.keyboard = compose_keyboard(&message.plan)?;
        // 卡片胜出只改本轮消息体：Go 的 card 分支（`send_group_msg.go:742-783`）之后
        // foundItems 循环照旧把每个媒体段各发一条（`:787-802`）
        let mut extras: Vec<MessageToCreate> = music.into_iter().collect();
        extras.extend(compose_media_extras(context, target, openid, &message.plan, deps).await?);
        return Ok((out, extras));
    }
    if !rendered.is_empty() {
        out.content = Some(rendered);
    }
    out.keyboard = compose_keyboard(&message.plan)?;
    let mut music_bodies = Vec::new();
    if out.content.is_none() {
        // 只有音乐时它自己就是主消息（Go 的 qqmusic 分支：msg_type=2 + 自带键盘）。独立
        // `[CQ:keyboard]` 与音乐键盘抢同一个字段（Go 会另发一条），这里明确拒绝而不是静默丢
        if let Some(body) = music.take() {
            if !message.plan.keyboard.is_empty() {
                return Err(Error::Protocol(
                    "[CQ:music] 不能同时带 [CQ:keyboard]：音乐消息自带播放 / 点歌按钮".to_owned(),
                ));
            }
            out.msg_type = Some(msg_type::MARKDOWN);
            out.markdown = body.markdown;
            out.keyboard = body.keyboard;
        }
    } else {
        // 有正文时音乐另起一条（Go 先发文本条，再在 foundItems 循环里发音乐条）
        music_bodies.extend(music);
    }
    // 单条消息只有一个 media 槽：多段媒体一段一条（Go 的 foundItems 循环把每个 URL 单独
    // 塞进 `singleItem` 再发一条，`send_group_msg.go:787-802`）。首段留在本条（正文 / 键盘
    // 都挂在这里），其余各自成条；纯媒体时它就是主消息，音乐已占主消息时媒体全部另起
    let mut bodies = compose_media_extras(context, target, openid, &message.plan, deps).await?;
    if !bodies.is_empty() && (out.content.is_some() || out.markdown.is_none()) {
        out.media = bodies.remove(0).media;
        out.msg_type = Some(msg_type::RICH_MEDIA);
    }
    music_bodies.extend(bodies);
    Ok((out, music_bodies))
}

/// 计划里是否有任何媒体段。
fn has_media(plan: &OutboundPlan) -> bool {
    !plan.images.is_empty()
        || !plan.records.is_empty()
        || !plan.videos.is_empty()
        || !plan.files.is_empty()
}

/// `plan.markdown` → markdown 消息体：**只取 `content`**。
///
/// `template_id` / `custom_template_id` 官方已标注废弃（出站不再构造）；载荷里带了也不读，
/// 缺 `content` 直接 `1400`。多份 markdown 取第一份（与 card 口径一致）。
///
/// 组装顺序与 Go 一致：先把文本侧的 @全员 标签迁移进正文（`migrateAtEveryoneToMarkdown`），
/// 再把 `[CQ:at,qq=数字]` 换成 `qqbot-at-user` 标签（`ResolveMarkdownAtMentions`）。
fn compose_markdown(
    context: &ActionContext,
    plan: &OutboundPlan,
    rendered_text: &str,
) -> Result<Option<Markdown>> {
    let Some(payload) = plan.markdown.first() else {
        return Ok(None);
    };
    let value = payload.decode()?;
    let Some(content) = value.get("content").and_then(Value::as_str) else {
        return Err(Error::Protocol(
            "markdown 载荷缺少 content：模板消息已废弃，只支持原生 markdown 内容".to_owned(),
        ));
    };
    let content = migrate_at_everyone(content, rendered_text);
    Ok(Some(Markdown::content(render_markdown(context, &content)?)))
}

/// markdown 抢占正文时，把文本里的 @全员 标签并入 markdown 正文（Go `migrateAtEveryoneToMarkdown`）。
///
/// markdown 胜出时 `content` 会被清空，文本里的标签不迁移就会整条丢失（额度却已经占了）。
/// 正文为空（模板 markdown 没有 `content`）时不塞标签：官方模板消息塞进正文就是非法载荷；
/// 正文已有标签则保持原样，避免重复 @。
fn migrate_at_everyone(content: &str, rendered_text: &str) -> String {
    if content.is_empty()
        || at_everyone::contains_tag(content)
        || !at_everyone::contains_tag(rendered_text)
    {
        return content.to_owned();
    }
    // Go：strings.TrimSpace(content + " " + atEveryoneTag)
    format!("{content} {}", at_everyone::TAG).trim().to_owned()
}

/// markdown 正文里的 `[CQ:at,qq=…]` → 官方 `qqbot-at-user` 标签（Go `ResolveMarkdownAtMentions`）。
///
/// markdown 只认 `<qqbot-at-user id="OpenID" />`，文本那套 `<@openid>` 在 markdown 里不会
/// 渲染成 @。数字虚拟号必须经 idmap 反查到真实 OpenID；反查失败直接 `1400`——Go 这里会原样
/// 留着 CQ 码（等于发一段不可渲染的文本），Rust 侧不静默。
/// 缺 `qq` 的 at 段（`[CQ:at]` / `[CQ:at ]` / `[CQ:at,foo]` / `[CQ:at,qq=]`）同样 `1400`：
/// markdown 里留一段 CQ 码只会显示成字面文本。`at` 后带空白、但参数齐备的写法
/// （`[CQ:at ,qq=1]`）照常解析，不算缺 `qq`。
/// 文本路径相反，原样透传（见 [`render_text`]）。
///
/// 只替换 at 段，其余字节原样保留：不重新转义 `[` `]`，否则会破坏 `[文本](链接)` 语法。
fn render_markdown(context: &ActionContext, content: &str) -> Result<String> {
    // 归一后再去重：字面标签与 `[CQ:at,qq=all]` 归一后是同一个标签，留两个会重复 @
    let normalized = at_everyone::dedupe_tag(&at_everyone::normalize(content));
    let Some(first) = at_everyone::find_at_code(&normalized, 0) else {
        return Ok(normalized);
    };
    let mut out = String::with_capacity(normalized.len());
    out.push_str(&normalized[..first]);
    let mut from = first;
    loop {
        let Some(start) = at_everyone::find_at_code(&normalized, from) else {
            out.push_str(&normalized[from..]);
            return Ok(out);
        };
        out.push_str(&normalized[from..start]);
        let after = &normalized[start..];
        let Some(end) = after.find(']') else {
            // 未闭合的 at 段按原文保留（与 normalize 的 malformed 分支一致）
            out.push_str(after);
            return Ok(out);
        };
        let code = &after[..=end];
        let id = at_everyone::at_mention_id(code).ok_or_else(|| {
            Error::Protocol(format!("markdown 里的 {code} 不是合法的 at 段（缺 qq）"))
        })?;
        let openid = context.resolve_openid("qq", id, "user_openid")?;
        out.push_str("<qqbot-at-user id=\"");
        out.push_str(&openid);
        out.push_str("\" />");
        from = start + end + 1;
    }
}

/// Markdown 正文里的本地图片 → 可公网访问的引用（Go `ResolveMarkdownImages`）。
///
/// 只替换 **URL**：`![alt #480px #640px](path)` 的 alt 与尺寸语法、`[text](path)` 的链接
/// 文本、`<img>` 的其余属性都逐字节保留——与 Go 的两条正则同口径：
///
/// - `(!?)\[([^\]]*)\]\(([^)]+)\)`（`!?` 让普通 `[text](path)` 也参与转换）
/// - `(?i)(<img\b[^>]*?\bsrc=)(["']?)([^"'\s>]+)([^>]*>)`
///
/// 分流照 Go：`http(s)://` / `data:` / `qqbot-media://` 原样；其余当本地路径读字节、嗅探
/// 格式后按 `oss_type` 选上传链（见 [`resolve_url`]）。**任一步失败保留原引用**：一张坏
/// 图片不能拖死整条消息。
///
/// 两遍扫描（先 markdown 链接、后 HTML img）与 Go 一致；workspace 没有 `regex` 依赖，
/// 为两条正则引入外部依赖不值得，这里用等价的手写扫描（见 [`markdown_link_urls`] /
/// [`html_img_urls`]）。
async fn resolve_markdown_images(
    context: &ActionContext,
    hosting: &HostingRuntime,
    content: &str,
) -> String {
    let content = rewrite_urls(content, markdown_link_urls(content), context, hosting).await;
    rewrite_urls(&content, html_img_urls(&content), context, hosting).await
}

/// 一处待改写的 URL 在原文里的字节区间（`[start, end)`）。
struct UrlSpan {
    start: usize,
    end: usize,
}

/// 按 `spans` 逐个改写 URL；[`resolve_url`] 返回 `None` 时保留原引用。
async fn rewrite_urls(
    content: &str,
    spans: Vec<UrlSpan>,
    context: &ActionContext,
    hosting: &HostingRuntime,
) -> String {
    let mut out = String::with_capacity(content.len());
    let mut cursor = 0;
    for span in spans {
        out.push_str(&content[cursor..span.start]);
        match resolve_url(context, hosting, &content[span.start..span.end]).await {
            Some(url) => out.push_str(&url),
            None => out.push_str(&content[span.start..span.end]),
        }
        cursor = span.end;
    }
    out.push_str(&content[cursor..]);
    out
}

/// Markdown 链接 / 图片里 URL 的位置：`(!?)\[([^\]]*)\]\(([^)]+)\)` 的等价扫描。
///
/// `!?` 只是可选前缀，不改变 URL 区间，因此扫描只看 `[…](…)`：alt 到第一个 `]` 为止
/// （字符类不含 `]`，正则无路可退），URL 从 `(` 之后到下一个 `)`，且至少一个字符。
fn markdown_link_urls(content: &str) -> Vec<UrlSpan> {
    let mut spans = Vec::new();
    let mut index = 0;
    while let Some(open) = content[index..].find('[').map(|offset| index + offset) {
        let Some(close) = content[open + 1..].find(']').map(|offset| open + 1 + offset) else {
            break;
        };
        // `](` 必须紧邻；否则正则从下一个字节重新尝试
        if !content[close + 1..].starts_with('(') {
            index = open + 1;
            continue;
        }
        let start = close + 2;
        let Some(end) = content[start..].find(')').map(|offset| start + offset) else {
            break;
        };
        if end == start {
            // `()` 空 URL 不满足 `[^)]+`
            index = open + 1;
            continue;
        }
        spans.push(UrlSpan { start, end });
        index = end + 1;
    }
    spans
}

/// HTML `<img … src=…>` 里 URL 的位置：
/// `(?i)(<img\b[^>]*?\bsrc=)(["']?)([^"'\s>]+)([^>]*>)` 的等价扫描。
fn html_img_urls(content: &str) -> Vec<UrlSpan> {
    let bytes = content.as_bytes();
    let mut spans = Vec::new();
    let mut index = 0;
    while let Some(open) = find_ascii_ci(content, b"<img", index) {
        // `<img\b`：后面紧跟单词字符（`<imgx`）不算标签
        let after = open + 4;
        if bytes
            .get(after)
            .copied()
            .is_some_and(is_word_byte)
        {
            index = open + 1;
            continue;
        }
        // `[^>]*?` 不跨 `>`：`src=` 只在当前标签内找，标签在第一个 `>` 处闭合
        let Some(tag_end) = content[after..].find('>').map(|offset| after + offset) else {
            break;
        };
        let Some(src) = find_src_attr(bytes, after, tag_end) else {
            index = open + 1;
            continue;
        };
        let mut start = src + 4;
        if bytes
            .get(start)
            .is_some_and(|byte| *byte == b'"' || *byte == b'\'')
        {
            start += 1;
        }
        let mut end = start;
        while let Some(byte) = bytes.get(end).copied() {
            if byte == b'"' || byte == b'\'' || byte == b'>' || byte.is_ascii_whitespace() {
                break;
            }
            end += 1;
        }
        if end == start {
            // `src=""` / `src=>`：`[^"'\s>]+` 不匹配空值
            index = open + 1;
            continue;
        }
        spans.push(UrlSpan { start, end });
        index = tag_end + 1;
    }
    spans
}

/// `[from, to)` 里第一个 `src=`（ASCII 大小写不敏感），要求前面是边界（`\b`）。
fn find_src_attr(bytes: &[u8], from: usize, to: usize) -> Option<usize> {
    // `[^>]*?` 至少要吃掉一个字符：`<imgsrc=` 里 `g` 与 `s` 之间没有边界
    let mut index = from + 1;
    while index + 4 <= to {
        if bytes[index..index + 4].eq_ignore_ascii_case(b"src=")
            && !is_word_byte(bytes[index - 1])
        {
            return Some(index);
        }
        index += 1;
    }
    None
}

/// ASCII 大小写不敏感查找（`(?i)`）。
fn find_ascii_ci(haystack: &str, needle: &[u8], from: usize) -> Option<usize> {
    let bytes = haystack.as_bytes();
    let last = bytes.len().checked_sub(needle.len())?;
    (from..=last).find(|&index| bytes[index..index + needle.len()].eq_ignore_ascii_case(needle))
}

/// 正则 `\b` / `\w` 的 ASCII 口径。
fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// 一处引用 → 本地路径（Go `trimFilePrefix` + `safeLocalPath`）。
///
/// 安全口径与 Go 一致，**不额外加根目录白名单**：拒绝含 `..` 的路径、统一转绝对路径；
/// 百分号解码只在 `file://` 形态下做（[`MediaSource::parse`] 已实现同一套校验，裸路径
/// 不解码也就不存在 `%2e%2e` 绕过）。
fn local_media_path(raw: &str) -> Option<PathBuf> {
    if raw.starts_with(FILE_PREFIX) {
        return match MediaSource::parse(raw) {
            Ok(MediaSource::File(path)) => Some(path),
            _ => None,
        };
    }
    if raw.is_empty() || raw.contains("..") {
        return None;
    }
    std::path::absolute(raw).ok()
}

/// 一处本地引用（markdown 图片 / 卡片 `pic_url`）→ 公网 URL；`None` 表示保留原引用。
///
/// 按 `oss_type` 选上传链（Go `resolveLocalImageMedia` → `images.UploadBase64ImageToServer`）：
/// - `7`（ChatGLM）：官方素材库，产出 `qqbot-media://` 引用；
/// - `4~6` / `8~10`：图床后端，产出 CDN URL（[`HostingRuntime::upload`]）；
/// - `1~3`：内置云 OSS（腾讯云 COS / 百度云 BOS / 阿里云 OSS），产出对象存储 URL
///   （[`HostingRuntime::upload_oss`]）；
/// - `0`：Go 走 `localUploadBehavior`——把图片 POST 给本机（或 lotus）的 `/uploadpic` 换公网
///   URL；Rust 侧没有这个端点，保留原引用并提示一次。
async fn resolve_url(
    context: &ActionContext,
    hosting: &HostingRuntime,
    raw: &str,
) -> Option<String> {
    // Go 同口径：远端地址、内联数据、已是素材引用的都原样保留（远端稳定化不在本阶段）
    if raw.starts_with("http://")
        || raw.starts_with("https://")
        || raw.starts_with("data:")
        || raw.starts_with(gensokyo_actions::MEDIA_REFERENCE_PREFIX)
    {
        return None;
    }
    // 0（本地上传）与越界值两条链都不是：保留原引用
    let oss_type = context.config().snapshot().get_oss_type();
    let hosting_kind = HostingKind::from_oss_type(oss_type);
    let oss_kind = OssKind::from_oss_type(oss_type);
    if hosting_kind.is_none() && oss_kind.is_none() {
        if oss_type == 0 {
            warn_local_upload_unavailable();
        }
        return None;
    }
    let Some(path) = local_media_path(raw) else {
        tracing::warn!(url = %raw, "本地图片路径未通过安全校验，保留原引用");
        return None;
    };
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(err) => {
            tracing::warn!(
                path = %path.display(),
                error = %err,
                "本地图片读取失败，保留原引用"
            );
            return None;
        }
    };
    // 1~3：内置云 OSS 直传。Go 这条分支不做格式嗅探（对象 key 写死 `.jpg`），这里也不做。
    if let Some(kind) = oss_kind {
        let config = crate::hosting::oss_config_from_settings(&context.config().snapshot());
        return match hosting.upload_oss(kind, &config, &bytes).await {
            Ok(url) => Some(url),
            Err(err) => {
                warn_oss_failure(kind, &path, &err);
                None
            }
        };
    }
    let Some(image) = sniff_image(&bytes) else {
        tracing::warn!(path = %path.display(), "本地图片格式无法识别，保留原引用");
        return None;
    };
    let extension = match image {
        ImageKind::Png => "png",
        ImageKind::Jpeg => "jpg",
        ImageKind::Gif => "gif",
        ImageKind::WebP => "webp",
        ImageKind::Bmp => "bmp",
    };
    // Go `uploadImageBytesToMediaStore` 的文件名是 `<source>.<ext>`，Markdown 侧 source = "markdown"
    let file_name = format!("markdown.{extension}");
    // 走到这里只可能是图床（1~3 已在上面返回）
    let kind = hosting_kind?;
    if kind == HostingKind::ChatGlm {
        // `validate_image_upload` 只收 png / jpeg / gif / webp（Go `sniffMediaImageFormat` 同）
        if image == ImageKind::Bmp {
            tracing::warn!(path = %path.display(), "本地图片是 BMP，素材库不收，保留原引用");
            return None;
        }
        return match upload_bytes_to_media_store(context.api(), &bytes, &file_name, image.mime())
            .await
        {
            Ok(uploaded) => Some(uploaded.url),
            Err(err) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %err,
                    "本地图片上传素材库失败，保留原引用"
                );
                None
            }
        };
    }
    let config = crate::hosting::config_from_settings(&context.config().snapshot());
    match hosting.upload(kind, &config, &bytes, &file_name).await {
        Ok(url) => Some(url),
        Err(err) => {
            warn_hosting_failure(kind, &path, &err);
            None
        }
    }
}

/// 图床上传失败的告警：缺凭据（配置问题）只喊一次，其余错误逐图喊（带路径，便于排障）。
fn warn_hosting_failure(kind: HostingKind, path: &std::path::Path, err: &gensokyo_media::Error) {
    if matches!(err, gensokyo_media::Error::HostingConfig(_)) {
        static WARNED: Once = Once::new();
        WARNED.call_once(|| {
            tracing::warn!(
                provider = kind.provider_name(),
                error = %err,
                "图床缺少必要配置，markdown 本地图片保留原引用"
            );
        });
        return;
    }
    tracing::warn!(
        provider = kind.provider_name(),
        path = %path.display(),
        error = %err,
        "本地图片图床上传失败，保留原引用"
    );
}

/// 内置云 OSS 上传失败的告警：缺凭据 / 审核未实现（配置或能力问题）只喊一次，
/// 其余错误逐图喊（带路径，便于排障）。
fn warn_oss_failure(kind: OssKind, path: &std::path::Path, err: &gensokyo_media::Error) {
    if matches!(err, gensokyo_media::Error::HostingConfig(_)) {
        static WARNED: Once = Once::new();
        WARNED.call_once(|| {
            tracing::warn!(
                provider = kind.name(),
                error = %err,
                "内置云 OSS 不可用（配置缺失或能力未实现），markdown 本地图片保留原引用"
            );
        });
        return;
    }
    tracing::warn!(
        provider = kind.name(),
        path = %path.display(),
        error = %err,
        "本地图片内置云 OSS 上传失败，保留原引用"
    );
}

/// `oss_type=0`（Go `localUploadBehavior`）在 Rust 侧没有落地条件：Go 把 base64 图片 POST 给
/// 本机或 lotus 的 `/uploadpic` 换公网 URL，本仓既无该端点也没有 lotus 通道。
fn warn_local_upload_unavailable() {
    static WARNED: Once = Once::new();
    WARNED.call_once(|| {
        tracing::warn!(
            "oss_type=0（本地上传）在 Rust 侧未实现：缺少 /uploadpic 端点，markdown 本地图片保留原引用"
        );
    });
}

/// `plan.card` → 图文卡片：`{type:"tuwen", content:{title,description,url,pic_url}}`。
///
/// `pic_url` 按 Go `resolveCardPic` 处理：`http(s)` 原样保留；其余（本地路径）先走统一上传链
/// （[`resolve_url`]，按 `oss_type` 选内置 OSS / 图床 / 素材库）换成公网 URL，失败保留原值并
/// `warn`。四字段官方校验（title 1~50、description 1~200、url / pic_url https 且 ≤1024）在
/// [`MessageCardContent::validate`]；四项缺一或任一字段非法都 `1400`。
/// 多份 card 只发第一份（与 Go 的第一份语义一致）。
async fn compose_card(
    context: &ActionContext,
    hosting: &HostingRuntime,
    plan: &OutboundPlan,
    rendered_text: &str,
) -> Result<Option<MessageCard>> {
    let Some(payload) = plan.card.first() else {
        return Ok(None);
    };
    // 卡片独占正文（msg_type=8 时 content 必须为空）：@全员 标签无处安放，直接 1400，
    // 而不是清空正文后静默丢标签（还白占当天额度）
    if at_everyone::contains_tag(rendered_text) {
        return Err(Error::Protocol(
            "卡片消息不能同时 @全员：msg_type=8 的正文会被清空，标签无法投递".to_owned(),
        ));
    }
    let fields = payload.fields()?;
    // Go `resolveCardPic`：空串 / http(s) 原样；本地路径上传后换公网 URL；失败保留原值
    let pic = match fields.pic {
        Some(pic) => Some(resolve_url(context, hosting, &pic).await.unwrap_or(pic)),
        None => None,
    };
    let content = MessageCardContent {
        title: fields.title,
        description: fields.desc,
        url: fields.url,
        pic_url: pic,
    };
    content
        .validate()
        .map_err(|err| Error::Protocol(format!("卡片字段非法（retcode 1400）: {err}")))?;
    Ok(Some(MessageCard::tuwen(content)))
}

/// 输入状态通知的缺省持续秒数（Go `sendInputNotify` 里的 `inputSecond = 60`）。
const INPUT_NOTIFY_DEFAULT_SECONDS: i64 = 60;

/// 流式消息 `stream.state`：续写（`start` / `mid`）。
const STREAM_STATE_CONTINUE: i64 = 1;

/// 流式消息 `stream.state`：结束（`finish`）。
const STREAM_STATE_FINISH: i64 = 10;

/// `[CQ:keyboard,data=…]` → `keyboard` 字段（与 text / markdown / card 同条发送）。
///
/// Go `applyGroupMarkdownKeyboard` 只在 markdown 之外单独挂键盘时赋值，且不动 `msg_type`：
/// 键盘是加性字段，不参与 msg_type 的优先级。`foundItems["keyboard"]` 已是 JSON 串
/// （协议层把对象 / `base64://` 归一成 JSON），多份只发第一份（与 markdown / card 同口径）。
/// 非 JSON / 非对象一律 `1400`：Go 那边是静默不挂键盘，Rust 侧不吞用户配置。
fn compose_keyboard(plan: &OutboundPlan) -> Result<Option<Value>> {
    let Some(payload) = plan.keyboard.first() else {
        return Ok(None);
    };
    let value: Value = serde_json::from_str(&payload.raw)
        .map_err(|error| Error::Protocol(format!("keyboard 载荷不是合法 JSON: {error}")))?;
    if !value.is_object() {
        return Err(Error::Protocol("keyboard 载荷不是 JSON 对象".to_owned()));
    }
    Ok(Some(value))
}

/// `[CQ:input_notify]` → `msg_type=6` + `input_notify`（仅单聊）。
///
/// 官方只有 C2C 支持输入状态，群聊一律 `1400`。`msg_type` 只能取一个值，正文 / 媒体 /
/// markdown / 卡片 / 键盘都带不进去：同时出现直接 `1400`，而不是清空正文静默丢弃
/// （Go 是先发通知再发正文两条消息，deferred 一次计划只发一条）。
fn compose_input_notify(
    target: &SendTarget,
    plan: &OutboundPlan,
    notify: &InputNotifyPayload,
    message: &MessageOut,
    msg_id: Option<String>,
) -> Result<MessageToCreate> {
    if matches!(target, SendTarget::Group(_)) {
        return Err(Error::Protocol(
            "[CQ:input_notify] 只有单聊支持（官方 msg_type=6 仅 C2C）".to_owned(),
        ));
    }
    if !message.text.trim().is_empty()
        || !plan.markdown.is_empty()
        || !plan.card.is_empty()
        || !plan.keyboard.is_empty()
        || !plan.qqmusic.is_empty()
        || has_media(plan)
    {
        return Err(Error::Protocol(
            "[CQ:input_notify] 必须单独成条：msg_type=6 不承载正文 / 媒体 / markdown / 卡片 / 键盘 / 音乐"
                .to_owned(),
        ));
    }
    Ok(MessageToCreate {
        msg_type: Some(msg_type::INPUT_NOTIFY),
        input_notify: Some(InputNotify {
            input_type: notify.kind,
            input_second: notify.second.unwrap_or(INPUT_NOTIFY_DEFAULT_SECONDS),
        }),
        msg_id,
        ..MessageToCreate::default()
    })
}

/// `[CQ:stream]` → `msg_type=2` + `stream`（仅单聊的流式消息，Go `MessageSSE` 口径）。
///
/// Go 的 SSE 分支（`handlers/send_private_msg_sse.go`）把正文塞进 `markdown.content`、
/// 阶段写进 `stream.state`：`start` / `mid` 是 1（续写），`finish` 是 10（结束）。
/// `mid` / `finish` 必须带 `id`（首片返回的 `stream_msg_id`）：缺了服务端无从续写，
/// 这里直接 `1400`，而不是发一条会被丢掉的分片。群聊官方不支持流式，`1400`。
fn compose_stream(
    context: &ActionContext,
    target: &SendTarget,
    plan: &OutboundPlan,
    stream: &StreamPayload,
    message: &MessageOut,
    msg_id: Option<String>,
) -> Result<MessageToCreate> {
    if matches!(target, SendTarget::Group(_)) {
        return Err(Error::Protocol(
            "[CQ:stream] 只有单聊支持流式消息".to_owned(),
        ));
    }
    if !plan.markdown.is_empty()
        || !plan.card.is_empty()
        || !plan.keyboard.is_empty()
        || !plan.qqmusic.is_empty()
        || has_media(plan)
    {
        return Err(Error::Protocol(
            "[CQ:stream] 不能与 markdown / 卡片 / 键盘 / 音乐 / 媒体同条发送（流式消息自带正文与 msg_type=2）"
                .to_owned(),
        ));
    }
    let state = match &stream.kind {
        StreamKind::Start | StreamKind::Mid => STREAM_STATE_CONTINUE,
        StreamKind::Finish => STREAM_STATE_FINISH,
        StreamKind::Other(kind) => {
            return Err(Error::Protocol(format!(
                "[CQ:stream] 未知阶段 {kind}（只支持 start / mid / finish）"
            )));
        }
    };
    if !matches!(stream.kind, StreamKind::Start) && stream.id.is_none() {
        return Err(Error::Protocol(
            "[CQ:stream] mid / finish 必须带 id（首片返回的 stream_msg_id）".to_owned(),
        ));
    }

    let mut out = MessageToCreate {
        msg_type: Some(msg_type::MARKDOWN),
        msg_id,
        ..MessageToCreate::default()
    };
    if !message.text.trim().is_empty() {
        // 流式正文走 markdown 通道：`[CQ:at]` 要转成 `<qqbot-at-user>` 才渲染
        let content = if message.auto_escape {
            message.text.clone()
        } else {
            render_markdown(context, &message.text)?
        };
        out.markdown = Some(Markdown::content(content));
    }
    out.stream = Some(StreamSse {
        state,
        index: stream.index.unwrap_or(0),
        id: stream.id.clone(),
    });
    Ok(out)
}

/// QQ 音乐歌曲详情响应上限：2MB（Go `maxQQMusicResponseBytes = 2 << 20`）。
const QQ_MUSIC_MAX_RESPONSE_BYTES: usize = 2 << 20;

/// 专辑封面 CDN 前缀（Go `parseQQMuiscMDData` 的 `T002R180x180M000` 拼接）。
const QQ_MUSIC_COVER_PREFIX: &str = "https://y.gtimg.cn/music/photo_new/T002R180x180M000";

/// 专辑封面尺寸：`T002R180x180M000` 这一档固定 180x180（Go 现查 `GetImageDimensions`，结果
/// 相同但多一次网络请求；这里直接取 CDN 固定的尺寸）。
const QQ_MUSIC_COVER_SIZE_PX: u32 = 180;

/// 播放页跳转前缀（Go `jumpURL`，中间拼 `track_info.mid`）。
const QQ_MUSIC_JUMP_PREFIX: &str = "https://i.y.qq.com/v8/playsong.html?platform=11&appshare=android_qq&appversion=10030010&hosteuin=oKnlNenz7i-s7c**&songmid=";

/// 播放页跳转后缀。
const QQ_MUSIC_JUMP_SUFFIX: &str = "&type=0&appsongtype=1&_wv=1&source=qq&ADTAG=qfshare";

/// `songinfo.data.track_info` 里出站要用的字段（Go 用 gjson 逐字段取，缺失按空串）。
struct MusicTrack {
    mid: String,
    name: String,
    singer: String,
    album_mid: String,
}

/// `[CQ:music,type=qq,id=…]` → `msg_type=2` 的 markdown + 键盘（Go `parseQQMuiscMDData`）。
///
/// 查询经 [`DeferredDeps::media`] 的下载 seam（生产是 `gensokyo_media::HttpTransport`，单测
/// 注入假传输），失败由调用方降级成分享文案。只取第一首，与 Go
/// `foundItems["qqmusic"][0]` 同口径。
async fn compose_music(
    context: &ActionContext,
    deps: &DeferredDeps<'_>,
    id: &str,
) -> Result<MessageToCreate> {
    let track = fetch_music_track(deps, id).await?;
    let settings = context.config().snapshot();
    Ok(MessageToCreate {
        msg_type: Some(msg_type::MARKDOWN),
        markdown: Some(Markdown::content(music_markdown(&track))),
        keyboard: Some(music_keyboard(&track.mid, settings.get_music_prefix())),
        ..MessageToCreate::default()
    })
}

/// 查 QQ 音乐歌曲详情（Go `FetchSongDetail` + `QQMusicSongInfo`）。
///
/// 字段缺失按空串（gjson 口径），只有 `track_info` 不存在才算「歌曲不存在」。
async fn fetch_music_track(deps: &DeferredDeps<'_>, id: &str) -> Result<MusicTrack> {
    let url = format!(
        "https://u.y.qq.com/cgi-bin/musicu.fcg?format=json&inCharset=utf8&outCharset=utf-8&notice=0&platform=yqq.json&needNewCode=0&data={{\"comm\":{{\"ct\":24,\"cv\":0}},\"songinfo\":{{\"method\":\"get_song_detail_yqq\",\"param\":{{\"song_type\":0,\"song_mid\":\"\",\"song_id\":{id}}},\"module\":\"music.pf_song_detail_svr\"}}}}"
    );
    let fetched = deps.media.fetch(&MediaRef::Url(url)).await?;
    if fetched.bytes.len() > QQ_MUSIC_MAX_RESPONSE_BYTES {
        return Err(Error::Network(format!(
            "QQ 音乐响应超过 {QQ_MUSIC_MAX_RESPONSE_BYTES} 字节"
        )));
    }
    let value: Value = serde_json::from_slice(&fetched.bytes)
        .map_err(|err| Error::Network(format!("QQ 音乐响应不是合法 JSON: {err}")))?;
    let Some(track) = value.pointer("/songinfo/data/track_info") else {
        return Err(Error::Network(
            "QQ 音乐没有返回 track_info（歌曲不存在）".to_owned(),
        ));
    };
    let field = |path: &str| {
        track
            .pointer(path)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    Ok(MusicTrack {
        mid: field("/mid"),
        name: field("/name"),
        singer: field("/singer/0/name"),
        album_mid: field("/album/mid"),
    })
}

/// 曲目 → 原生 markdown：封面 + 曲名 / 歌手（对应 Go 模板的 `img_url` + `text_end`）。
fn music_markdown(track: &MusicTrack) -> String {
    let size = QQ_MUSIC_COVER_SIZE_PX;
    let cover = format!("{QQ_MUSIC_COVER_PREFIX}{}.jpg", track.album_mid);
    format!(
        "![图片 #{size}px #{size}px]({cover})\r{}\r{}",
        track.name, track.singer
    )
}

/// 音乐键盘（Go `createMusicKeyboard`）：两个按钮同处一行。
///
/// 按钮结构逐字段照 Go `keyboard.Button` 的序列化结果（`anchor` / `enter` / `reply` 是 Go
/// 结构的零值常量，照样带）；第二个按钮是指令类型，`data` = `music_prefix` + 空格，用户点一下
/// 就能接着输入歌名。
fn music_keyboard(mid: &str, music_prefix: &str) -> Value {
    let jump = format!("{QQ_MUSIC_JUMP_PREFIX}{mid}{QQ_MUSIC_JUMP_SUFFIX}");
    let button = |label: &str, visited: &str, kind: u32, data: &str| {
        json!({
            "render_data": {"label": label, "visited_label": visited, "style": 1},
            "action": {
                "type": kind,
                "permission": {"type": 2},
                "data": data,
                "unsupport_tips": "请升级新版手机QQ",
                "anchor": 0,
                "enter": false,
                "reply": false
            }
        })
    };
    json!({
        "content": {
            "rows": [{
                "buttons": [
                    button("立刻播放", "已播放", 0, &jump),
                    button("我也要点歌", "再次点歌", 2, &format!("{music_prefix} ")),
                ]
            }]
        }
    })
}

/// 查询失败时的降级正文：分享文案接在正文之后（Go 在这里整条丢弃）。
fn music_share_text(rendered: &str, id: &str) -> String {
    let share = format!("分享一首 QQ 音乐：{id}");
    if rendered.is_empty() {
        share
    } else {
        format!("{rendered}\n{share}")
    }
}

/// 正文里的 `AppID → BotID`（Go `transformMessageTextAt` / `transformMessageTextAtNoGroupID`，
/// `message_parser.go:1669-1683` / `:1704-1718`）。
///
/// OneBot 客户端用 `app_id` 认机器人，QQ 上游要的是真实 ID，所以正文里以 `app_id` 写的
/// 纯文本在发送前换成 BotID（频道 bot 需要「自己 at 自己」的场景靠它）。CQ 码区间整段跳过
/// （Go 用 `[CQ:[^\]]*]` 占位再还原），否则 `[CQ:at,qq=<app_id>]` 这类参数会被改写。
///
/// `do_not_replace_appid` 为真、BotID 未取到、或 BotID 与 AppID 相同时原样返回。`auto_escape`
/// 的正文不进这里（[`render_text`] 先返回），保持「原样发送」的既有口径。
fn replace_app_id<'a>(context: &ActionContext, text: &'a str) -> Cow<'a, str> {
    let settings = context.config().snapshot();
    if settings.get_do_not_replace_appid() {
        return Cow::Borrowed(text);
    }
    let app_id = settings.get_app_id().to_string();
    let Some(bot_id) = context
        .bot_id()
        .filter(|id| !id.is_empty() && *id != app_id.as_str())
    else {
        return Cow::Borrowed(text);
    };
    if !text.contains(&app_id) {
        return Cow::Borrowed(text);
    }
    Cow::Owned(replace_outside_cq_codes(text, &app_id, bot_id))
}

/// 只替换 CQ 码之外的 `needle`，CQ 码逐字节原样保留。
///
/// 等价 Go 的占位符方案（`cqAnyRe` = `\[CQ:[^\]]*\]`）：未闭合的 `[CQ:` 在 Go 里不算 CQ 码，
/// 这里同样按普通文本处理，不把其后整段当成码。
fn replace_outside_cq_codes(text: &str, needle: &str, replacement: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("[CQ:") {
        out.push_str(&rest[..start].replace(needle, replacement));
        let code = &rest[start..];
        let Some(close) = code.find(']') else {
            out.push_str(&code.replace(needle, replacement));
            return out;
        };
        out.push_str(&code[..=close]);
        rest = &code[close + 1..];
    }
    out.push_str(&rest.replace(needle, replacement));
    out
}

/// 文本主体 → v2 `content`：`[CQ:at,qq=…]` 转 `<@openid>`，`qq=all` / `everyone` 转 @全员标签，
/// 缺 `qq` 的 at 段与其余 CQ 码保持原样。
///
/// 缺 `qq` 的 at 段（`[CQ:at]` / `[CQ:at ]` / `[CQ:at,foo]` / `[CQ:at,qq=]`）在文本路径
/// **原样透传**，不报错（与 Go 文本路径一致）；`at` 后带空白但参数齐备的写法照常反查。
/// 数字虚拟号反查失败同样保留原文并 `warn`。markdown 路径相反，这两种情况都 `1400`，见
/// [`render_markdown`]。
/// 这里按 at 段切原文再逐段渲染，而不是整条过一遍 `cqcode::parse`：解析器会丢掉
/// `[CQ:at,foo]` 里那个不成对的参数，重新序列化后就吐不出原文了。
fn render_text(context: &ActionContext, message: &MessageOut) -> Result<String> {
    if message.auto_escape {
        return Ok(message.text.clone());
    }

    let text = replace_app_id(context, &message.text);
    let text: &str = text.as_ref();
    let mut out = String::with_capacity(text.len());
    let mut from = 0;
    while let Some(start) = at_everyone::find_at_code(text, from) {
        let after = &text[start..];
        let Some(end) = after.find(']') else {
            // 未闭合的 at 段：剩余部分按原文处理（与 `normalize` 的 malformed 分支一致）
            out.push_str(&render_plain(&text[from..]));
            return Ok(at_everyone::dedupe_tag(&out));
        };
        let code = &after[..=end];
        out.push_str(&render_plain(&text[from..start]));
        match at_everyone::at_mention_id(code) {
            // @全员 是另一套标签，`all` / `everyone` 大小写不敏感（Go 同样认 `ALL`）
            Some(id) if at_everyone::is_everyone_id(id) => out.push_str(at_everyone::TAG),
            // 文本消息官方仍在用旧式 `<@openid>`（标注即将弃用但当前可用）
            Some(id) if !id.is_empty() => match context.resolve_openid("qq", id, "user_openid") {
                Ok(openid) => {
                    out.push_str("<@");
                    out.push_str(&openid);
                    out.push('>');
                }
                // 文本路径反查失败只告警、保留 CQ 原文：与 Go 同口径，不让整条文本消息 1400
                // （markdown 路径相反，反查失败仍然 1400，见 [`render_markdown`]）
                Err(err) => {
                    tracing::warn!(%id, error = %err, "文本 @ 反查失败，保留 CQ 原文");
                    out.push_str(code);
                }
            },
            // 缺 `qq`（含 `qq=` 空值）：原样透传，与 Go 文本路径一致
            _ => out.push_str(code),
        }
        from = start + end + 1;
    }
    out.push_str(&render_plain(&text[from..]));
    // 字面标签与 CQ 写法可能同时出现，收敛成一个
    Ok(at_everyone::dedupe_tag(&out))
}

/// 一段不含 at 段（或含不了 at 段）的文本：走一次 CQ 解析，文本段取原文，其余码段透传原文。
fn render_plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for segment in cqcode::parse(text) {
        match segment.kind.as_str() {
            "text" => out.push_str(segment.text_content().unwrap_or_default()),
            // 非 at 段是纯透传：输出 parse 的原文切片（自产段回退规范化序列化）。
            // 走 to_cq() 的规范化形态会改写用户「段名后/键值两侧空白」等字节（Major 修复）
            _ => out.push_str(&segment.raw_or_cq()),
        }
    }
    out
}

/// `[CQ:reply]` 目标 → 上游真实消息 ID。
///
/// 字符串形态（32 位 OpenID / flag 型 ID）本身就是真实 ID，直接使用；数字形态是 OneBot
/// 虚拟消息编号，按两条来源依次反查：
/// 1. idmap（入站消息经 [`gensokyo_storage::IdMapStore::store_cache`] 登记，Go
///    `RetrieveRowByCachev2`），值为 `"<群号> <消息ID>"` 时取最后一段；
/// 2. echo 行号反表（本进程发送成功的消息经 [`EchoStore::store_cache_in_memory`] 登记）。
///
/// 都查不到说明引用的是别的进程 / 已过期的 ID——报 `1400` 说明原因，而不是静默丢掉 reply。
fn resolve_reply(
    context: &ActionContext,
    echo: Option<&EchoStore>,
    reply: &Ob11Id,
) -> Result<String> {
    if let Some(raw) = reply.as_str() {
        if raw.trim().is_empty() {
            return Err(Error::Protocol("[CQ:reply] 的 id 为空".to_owned()));
        }
        return Ok(raw.to_owned());
    }
    let row = reply
        .as_i64()
        .ok_or_else(|| Error::Protocol(format!("[CQ:reply] 的 id 非法: {reply}")))?;

    if let Some(idmap) = context.idmap() {
        match idmap.retrieve_row_by_cache(row) {
            Ok(real) => {
                let real = last_token(&real);
                if !real.is_empty() {
                    return Ok(real);
                }
                tracing::debug!(row, "idmap 反查到空消息 ID，继续查 echo");
            }
            // 未登记 / 已过期都按「查不到」处理，继续走 echo
            Err(err) if err.is_not_found() => {}
            Err(err) => return Err(Error::Storage(format!("reply 反查 idmap 失败: {err}"))),
        }
    }

    if let Some(real) =
        echo.and_then(|echo| echo.get_cache_id_from_memory_by_row_id(&row.to_string()))
        && !real.trim().is_empty()
    {
        return Ok(real);
    }

    Err(Error::Protocol(format!(
        "[CQ:reply,id={row}] 无法还原成真实消息 ID（idmap / echo 均未命中）：\
         请改用真实消息 ID，或确认被引用的消息未被清理"
    )))
}

/// idmap 里有的记录形如 `"<群号> <消息ID>"`（Go `StoreCachev2`），取最后一段。
fn last_token(raw: &str) -> String {
    raw.split_whitespace().last().unwrap_or_default().to_owned()
}

/// `MediaToCreate.media` 的段类型。
#[derive(Clone, Copy)]
enum MediaKind {
    /// 图片（`file_type=1`）。
    Image,
    /// 语音（`file_type=3`）。
    Record,
    /// 视频（`file_type=2`）。
    Video,
    /// 文件（`file_type=4`）。
    File,
}

impl MediaKind {
    /// 上游 `file_type`。
    const fn file_type(self) -> u32 {
        match self {
            Self::Image => file_type::IMAGE,
            Self::Record => file_type::VOICE,
            Self::Video => file_type::VIDEO,
            Self::File => file_type::FILE,
        }
    }
}

/// 计划里的媒体段逐段展开成消息体：每个媒体段一条 `msg_type=7`。
///
/// 单条消息只有一个 `media` 槽，因此 markdown / 卡片与媒体同现、以及纯多段媒体，都靠
/// 这个函数把后面的段拆出来。Go 的 foundItems 循环把每个 URL 单独塞进 `singleItem` 再发
/// 一条（`handlers/send_group_msg.go:787-802`），私聊同构（`send_private_msg.go:596-611`）。
/// 拆出来的条不带正文 / reply / 键盘：Go 那三样只挂首条那轮（`:815` / `:831-843`）。
async fn compose_media_extras(
    context: &ActionContext,
    target: &SendTarget,
    openid: &str,
    plan: &OutboundPlan,
    deps: &DeferredDeps<'_>,
) -> Result<Vec<MessageToCreate>> {
    let mut extras = Vec::new();
    for (kind, reference, file_name) in media_candidates(plan) {
        let media =
            compose_single_media(context, target, openid, kind, reference, file_name, deps).await?;
        extras.push(MessageToCreate {
            msg_type: Some(msg_type::RICH_MEDIA),
            media: Some(media),
            ..MessageToCreate::default()
        });
    }
    Ok(extras)
}

/// 计划里的媒体段按段序展开成扁平清单：图片 → 语音 → 视频 → 文件。
///
/// Go 按 `foundItems` 的 map 顺序遍历（每次发送顺序随机）；这里固定成计划顺序，多图时
/// 发出的次序可复现。
fn media_candidates(plan: &OutboundPlan) -> Vec<(MediaKind, &MediaRef, Option<&str>)> {
    let mut candidates = Vec::new();
    candidates.extend(
        plan.images
            .iter()
            .map(|media| (MediaKind::Image, media, None)),
    );
    candidates.extend(
        plan.records
            .iter()
            .map(|media| (MediaKind::Record, media, None)),
    );
    candidates.extend(
        plan.videos
            .iter()
            .map(|media| (MediaKind::Video, media, None)),
    );
    candidates.extend(
        plan.files
            .iter()
            .map(|file| (MediaKind::File, &file.media, file.file_name.as_deref())),
    );
    candidates
}

/// 单个媒体段 → `MessageToCreate.media`（取字节 → 上传 → `file_info`）。
async fn compose_single_media(
    context: &ActionContext,
    target: &SendTarget,
    openid: &str,
    kind: MediaKind,
    reference: &MediaRef,
    file_name: Option<&str>,
    deps: &DeferredDeps<'_>,
) -> Result<Media> {
    // 已是上游 `file_info` 的形态（Go 的 `unknown_*` 回退）：直接透传，不再上传
    if let MediaRef::Unknown(file_info) = reference {
        return Ok(Media {
            file_info: file_info.clone(),
        });
    }

    let fetched = deps.media.fetch(reference).await?;
    // Go 的发送前处理：图片只压缩本地 / base64 来源（`local_image` / `base64_image`），
    // URL 图片原样；语音三种来源都先判 AMR/SILK 再转码。这里按同一口径处理。
    let local_or_inline = matches!(reference, MediaRef::Local(_) | MediaRef::Base64(_));
    let mut bytes = fetched.bytes;
    match kind {
        MediaKind::Image if local_or_inline => {
            let threshold_kb =
                u64::try_from(context.config().snapshot().get_image_size_limit()).unwrap_or(0);
            if threshold_kb > 0 {
                bytes = deps.media.prepare_image(bytes, threshold_kb).await?;
            }
        }
        MediaKind::Record => {
            let sample_rate =
                u32::try_from(context.config().snapshot().get_record_sample_rate()).unwrap_or(0);
            bytes = deps.media.prepare_record(bytes, sample_rate).await?;
        }
        MediaKind::Image | MediaKind::Video | MediaKind::File => {}
    }
    let file_name = file_name.map_or_else(|| fetched.file_name.clone(), str::to_owned);
    // 官方 `file_data` 直传上限 5MB；超过走分片链（Go `needsChunkedUpload` → `chunkedUpload`）
    if bytes.len() as u64 > MAX_MEDIA_BYTES {
        return upload_media_chunked(context, target, openid, kind, &bytes, &file_name).await;
    }
    let upload = FileUpload {
        file_type: kind.file_type(),
        file_data: Some(gensokyo_media::encode_base64(&bytes)),
        file_name: Some(file_name),
        ..FileUpload::default()
    };
    let uploaded = match target {
        SendTarget::Group(_) => context.api().upload_group_file(openid, upload).await?,
        SendTarget::Private(_) => context.api().upload_c2c_file(openid, upload).await?,
    };
    if uploaded.file_info.is_empty() {
        return Err(Error::Upstream {
            code: 0,
            message: "媒体上传成功但未返回 file_info".to_owned(),
        });
    }
    Ok(Media {
        file_info: uploaded.file_info,
    })
}

/// 分片上传默认分片大小：5MiB（Go `defaultBlockSize`），仅当服务端未下发 `block_size` 时使用。
const CHUNK_DEFAULT_BLOCK_SIZE: u64 = 5 * 1024 * 1024;

/// `md5_10m` 的取样上限：前 10002432 字节（Go `computeMD510m`）。
const CHUNK_MD5_10M_BYTES: usize = 10_002_432;

/// >5MB 媒体上传：预上传 → 逐片 PUT + 完成通知 → 合并（Go `chunkedUpload`）。
///
/// 分片按服务端下发的 `block_size` 切；每片起点 `index * block_size`（Go 同，忽略片自带的
/// `part.block_size`）。任一步失败都原样上抛，不降级回 `file_data` 直传（Go 同样直接返回错误）。
async fn upload_media_chunked(
    context: &ActionContext,
    target: &SendTarget,
    openid: &str,
    kind: MediaKind,
    bytes: &[u8],
    file_name: &str,
) -> Result<Media> {
    let is_group = matches!(target, SendTarget::Group(_));
    let file_size = bytes.len();
    let name = if file_name.is_empty() {
        format!("upload_{}", unix_timestamp())
    } else {
        file_name.to_owned()
    };
    let prepared = context
        .api()
        .prepare_file_upload(
            openid,
            is_group,
            FileUploadPrepareRequest {
                file_type: kind.file_type(),
                file_size: file_size.to_string(),
                file_name: name.clone(),
                md5: md5_hex(bytes),
                sha1: String::new(),
                md5_10m: md5_hex(&bytes[..file_size.min(CHUNK_MD5_10M_BYTES)]),
            },
        )
        .await?;
    if prepared.parts.is_empty() {
        return Err(Error::Upstream {
            code: 0,
            message: "分片上传预上传未返回任何分片".to_owned(),
        });
    }
    let block_size = prepared
        .block_size
        .parse::<u64>()
        .ok()
        .filter(|size| *size > 0)
        .unwrap_or(CHUNK_DEFAULT_BLOCK_SIZE);
    for part in &prepared.parts {
        let start = part
            .index
            .checked_mul(block_size)
            .and_then(|offset| usize::try_from(offset).ok())
            .filter(|offset| *offset < file_size)
            .ok_or_else(|| Error::Upstream {
                code: 0,
                message: format!(
                    "分片索引越界: part={} block_size={block_size} file_size={file_size}",
                    part.index
                ),
            })?;
        let end = part
            .index
            .saturating_add(1)
            .checked_mul(block_size)
            .and_then(|offset| usize::try_from(offset).ok())
            .map_or(file_size, |offset| offset.min(file_size));
        let chunk = &bytes[start..end];
        // 分片 PUT 与素材直传同为「预签名 URL、不带 Authorization」，复用同一出口
        context
            .api()
            .put_presigned_image(&part.presigned_url, "application/octet-stream", chunk.to_vec())
            .await?;
        context
            .api()
            .finish_file_upload_part(
                openid,
                is_group,
                FileUploadPartFinishRequest {
                    upload_id: prepared.upload_id.clone(),
                    part_index: part.index,
                    block_size: chunk.len().to_string(),
                    md5: md5_hex(chunk),
                },
            )
            .await?;
    }
    let merged = context
        .api()
        .merge_file_upload(
            openid,
            is_group,
            FileUploadMergeRequest {
                file_type: kind.file_type(),
                file_name: name,
                srv_send_msg: false,
                upload_id: prepared.upload_id,
            },
        )
        .await?;
    if merged.file_info.is_empty() {
        return Err(Error::Upstream {
            code: 0,
            message: "分片合并成功但未返回 file_info".to_owned(),
        });
    }
    Ok(Media {
        file_info: merged.file_info,
    })
}

/// 全文件 / 分片的 MD5（32 位小写十六进制；Go `computeMD5`）。
fn md5_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    use md5::Digest as _;

    let digest = md5::Md5::digest(bytes);
    let mut hex = String::with_capacity(32);
    for byte in digest {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// Unix 秒（Go `time.Now().Unix()`，仅用于缺省文件名）。
fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// 本阶段仍不支持的消息段；返回第一个命中的段名。
///
/// `keyboard` / `music` / `stream` / `input_notify` 已按 Go 口径落地，不再走这里。
fn unsupported_segment(plan: &OutboundPlan) -> Option<&'static str> {
    let candidates: [(&'static str, bool); 3] = [
        ("unknown", !plan.extra.is_empty()),
        ("unknown_image", !plan.unknown_images.is_empty()),
        ("unknown_record", !plan.unknown_records.is_empty()),
    ];
    candidates
        .iter()
        .find(|(_, present)| *present)
        .map(|(kind, _)| *kind)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, LazyLock, Mutex};
    use std::time::{Duration, SystemTime};

    use gensokyo_actions::{
        ActionContext, EchoContext, ForwardParseFailure, ForwardPlan, ForwardRequest, MessageOut,
        QqApi, QqFuture, SendRequest, SendTarget, SseRequest,
    };
    use gensokyo_config::ConfigManager;
    use gensokyo_core::{Error, Ob11Id, Result};
    use gensokyo_media::silk::{ProcessOutput, ProcessRunner, SilkCodec};
    use gensokyo_media::{BoxFuture, FetchRequest, Fetcher, MediaTransport, TransportResponse};
    use gensokyo_media::{
        HostingTransport, UploadMethod, UploadRequest, UploadResponse,
    };
    use gensokyo_protocol::outbound::{
        ActiveKind, CardFields, CardPayload, InputNotifyPayload, KeyboardPayload, MarkdownPayload,
        StreamKind, StreamPayload,
    };
    use gensokyo_protocol::{MediaRef, OutboundPlan};
    use gensokyo_qq::dto::media_image::{
        MediaImageCompleteRequest, MediaImageCompleteResponse, MediaImagePrepareRequest,
        MediaImagePrepareResponse,
    };
    use gensokyo_qq::dto::{
        BotInGroupState, FileUpload, FileUploadMergeRequest, FileUploadPart,
        FileUploadPartFinishRequest, FileUploadPrepareRequest, FileUploadPrepareResponse, GroupInfo,
        GroupMember, GroupMemberList, InputNotify, JoinRequestList, Markdown, MediaResponse,
        MessageResponse, MessageToCreate, StreamSse, file_type, msg_type,
    };
    use gensokyo_storage::echo::MessageGroupPair;
    use gensokyo_storage::{EchoStore, IdMapOptions, IdMapStore};
    use serde_json::Value;
    use tempfile::TempDir;

    use super::{
        DeferredDeps, execute, execute_forward, execute_sse, render_plain, resolve_markdown_images,
    };
    use crate::at_everyone::SECTION;
    use crate::event_hub::EventHub;
    use crate::hosting::HostingRuntime;
    use crate::media::{MAX_MEDIA_BYTES, MAX_UPLOAD_BYTES, MediaRuntime};

    /// 测试统一的 32 位 OpenID（`resolve_openid` 对 32 位直通，不需要 idmap）。
    const OPENID: &str = "0123456789ABCDEF0123456789ABCDEF";

    /// @用户 目标的 32 位 OpenID（与收件人区分，验证反查走的是 idmap 而不是直通）。
    const MEMBER_OPENID: &str = "FEDCBA9876543210FEDCBA9876543210";

    /// Major 回归用例：`render_plain` 透传臂输出 parse 原文切片——非 at 码段逐字节不变
    /// （段名后/键值两侧空白、JSON 值内逗号都保持用户原样）。
    #[test]
    fn render_plain_passthrough_keeps_cq_bytes_verbatim() {
        for raw in [
            "[CQ:image , file = a b.png]",
            "prefix[CQ:keyboard , data = {\"a\":1,2}]suffix",
        ] {
            assert_eq!(render_plain(raw), raw, "透传应逐字节一致: {raw}");
        }
        assert_eq!(render_plain("纯文本"), "纯文本");
    }

    // ---- 装置 ----

    fn manager() -> (TempDir, Arc<ConfigManager>) {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        std::fs::write(&path, "version: 1\nsettings: {}\n").expect("写测试配置失败");
        let manager = Arc::new(ConfigManager::new(&path).expect("加载配置失败"));
        (dir, manager)
    }

    fn context(api: Arc<FakeQqApi>) -> (TempDir, ActionContext) {
        let (dir, manager) = manager();
        (dir, ActionContext::new(api, manager))
    }

    /// 记录入参、按槽位注入结果的 `QqApi` 替身（只覆盖 deferred 用到的四个方法）。
    #[derive(Debug, Default)]
    struct FakeQqApi {
        uploads: Mutex<Vec<FileUpload>>,
        sent: Mutex<Vec<MessageToCreate>>,
        upload_result: Mutex<Option<Result<MediaResponse>>>,
        /// 按顺序消费的发送结果；同一条用例可以排多个（成功 / 失败）。
        send_results: Mutex<Vec<Result<MessageResponse>>>,
        /// 素材链（prepare → PUT → complete）的注入结果：`Some(url)` = complete 返回该引用，
        /// `None` = complete 报上游错误。按调用顺序消费，多图用例逐个排队。
        media_results: Mutex<Vec<Option<String>>>,
        /// 预上传响应（每次 prepare 消费一次）；`None` = 测试未配置。
        chunked: Mutex<Option<Result<FileUploadPrepareResponse>>>,
        /// 合并响应（每次 merge 消费一次）。
        chunk_merge_results: Mutex<Option<Result<MediaResponse>>>,
        /// 预上传入参记录。
        prepares: Mutex<Vec<FileUploadPrepareRequest>>,
        /// 分片 PUT 记录（预签名 URL, 字节数）。
        chunk_puts: Mutex<Vec<(String, usize)>>,
        /// 分片完成通知记录。
        part_finishes: Mutex<Vec<FileUploadPartFinishRequest>>,
        /// 合并入参记录。
        merges: Mutex<Vec<FileUploadMergeRequest>>,
    }

    impl FakeQqApi {
        fn with_upload(file_info: &str) -> Self {
            Self {
                upload_result: Mutex::new(Some(Ok(MediaResponse {
                    file_info: file_info.to_owned(),
                    ..MediaResponse::default()
                }))),
                ..Self::default()
            }
        }

        fn with_upload_error(err: Error) -> Self {
            Self {
                upload_result: Mutex::new(Some(Err(err))),
                ..Self::default()
            }
        }

        fn with_send(self, message_id: &str) -> Self {
            self.push_send(Ok(MessageResponse {
                id: message_id.to_owned(),
                ..MessageResponse::default()
            }))
        }

        /// 排队一次成功的素材上传，complete 返回 `url`。
        fn with_media(self, url: &str) -> Self {
            self.media_results
                .lock()
                .expect("素材槽锁未中毒")
                .push(Some(url.to_owned()));
            self
        }

        /// 排队一次失败的素材上传（complete 报上游业务错误）。
        fn with_media_error(self) -> Self {
            self.media_results
                .lock()
                .expect("素材槽锁未中毒")
                .push(None);
            self
        }

        fn with_send_error(self, err: Error) -> Self {
            self.push_send(Err(err))
        }

        /// 配置分片上传：两片（首片 `MAX_MEDIA_BYTES` 字节、次片剩余），合并返回 `file_info`。
        fn with_chunked(self, file_info: &str) -> Self {
            *self.chunked.lock().expect("分片槽锁未中毒") = Some(Ok(FileUploadPrepareResponse {
                upload_id: "UP-1".to_owned(),
                block_size: MAX_MEDIA_BYTES.to_string(),
                parts: vec![
                    FileUploadPart {
                        index: 0,
                        presigned_url: "https://cos.example.com/part-0".to_owned(),
                        block_size: MAX_MEDIA_BYTES.to_string(),
                    },
                    FileUploadPart {
                        index: 1,
                        presigned_url: "https://cos.example.com/part-1".to_owned(),
                        block_size: "1".to_owned(),
                    },
                ],
            }));
            *self.chunk_merge_results.lock().expect("合并槽锁未中毒") = Some(Ok(MediaResponse {
                file_info: file_info.to_owned(),
                ..MediaResponse::default()
            }));
            self
        }

        /// 配置预上传失败（验证不降级回 `file_data` 直传）。
        fn with_chunked_prepare_error(self, err: Error) -> Self {
            *self.chunked.lock().expect("分片槽锁未中毒") = Some(Err(err));
            *self.chunk_merge_results.lock().expect("合并槽锁未中毒") = Some(Err(
                Error::Unsupported("预上传已失败，不应走到合并".to_owned()),
            ));
            self
        }

        fn prepares(&self) -> Vec<FileUploadPrepareRequest> {
            self.prepares.lock().expect("预上传记录锁未中毒").clone()
        }

        fn chunk_puts(&self) -> Vec<(String, usize)> {
            self.chunk_puts.lock().expect("分片 PUT 记录锁未中毒").clone()
        }

        fn part_finishes(&self) -> Vec<FileUploadPartFinishRequest> {
            self.part_finishes
                .lock()
                .expect("分片完成记录锁未中毒")
                .clone()
        }

        fn merges(&self) -> Vec<FileUploadMergeRequest> {
            self.merges.lock().expect("合并记录锁未中毒").clone()
        }

        fn push_send(self, result: Result<MessageResponse>) -> Self {
            self.send_results
                .lock()
                .expect("发送槽锁未中毒")
                .push(result);
            self
        }

        fn uploads(&self) -> Vec<FileUpload> {
            self.uploads.lock().expect("上传记录锁未中毒").clone()
        }

        fn sent(&self) -> Vec<MessageToCreate> {
            self.sent.lock().expect("发送记录锁未中毒").clone()
        }

        fn upload(&self, file: FileUpload) -> Result<MediaResponse> {
            self.uploads.lock().expect("上传记录锁未中毒").push(file);
            self.upload_result
                .lock()
                .expect("上传槽锁未中毒")
                .take()
                .expect("测试未配置 upload 响应")
        }

        fn send(&self, msg: MessageToCreate) -> Result<MessageResponse> {
            self.sent.lock().expect("发送记录锁未中毒").push(msg);
            let mut results = self.send_results.lock().expect("发送槽锁未中毒");
            assert!(!results.is_empty(), "测试未配置 send 响应");
            results.remove(0)
        }
    }

    impl QqApi for FakeQqApi {
        fn get_group_info(&self, _group_openid: &str) -> QqFuture<'_, GroupInfo> {
            Box::pin(async { Err(Error::Unsupported("未提供".to_owned())) })
        }

        fn get_group_member_info(
            &self,
            _group_openid: &str,
            _member_openid: &str,
        ) -> QqFuture<'_, GroupMember> {
            Box::pin(async { Err(Error::Unsupported("未提供".to_owned())) })
        }

        fn get_group_member_list(
            &self,
            _group_openid: &str,
            _limit: u32,
        ) -> QqFuture<'_, GroupMemberList> {
            Box::pin(async { Err(Error::Unsupported("未提供".to_owned())) })
        }

        fn set_group_ban(
            &self,
            _group_openid: &str,
            _member_openid: &str,
            _duration_secs: i64,
        ) -> QqFuture<'_, ()> {
            Box::pin(async { Err(Error::Unsupported("未提供".to_owned())) })
        }

        fn get_group_bot_state(&self, _group_openid: &str) -> QqFuture<'_, BotInGroupState> {
            Box::pin(async { Err(Error::Unsupported("未提供".to_owned())) })
        }

        fn get_group_join_request_list(
            &self,
            _group_openid: &str,
            _cursor: Option<&str>,
            _limit: Option<u32>,
        ) -> QqFuture<'_, JoinRequestList> {
            Box::pin(async { Err(Error::Unsupported("未提供".to_owned())) })
        }

        fn send_group_message(
            &self,
            _group_openid: String,
            msg: MessageToCreate,
        ) -> QqFuture<'_, MessageResponse> {
            Box::pin(async move { self.send(msg) })
        }

        fn send_c2c_message(
            &self,
            _user_openid: String,
            msg: MessageToCreate,
        ) -> QqFuture<'_, MessageResponse> {
            Box::pin(async move { self.send(msg) })
        }

        fn upload_group_file(
            &self,
            _group_openid: &str,
            file: FileUpload,
        ) -> QqFuture<'_, MediaResponse> {
            Box::pin(async move { self.upload(file) })
        }

        fn upload_c2c_file(
            &self,
            _user_openid: &str,
            file: FileUpload,
        ) -> QqFuture<'_, MediaResponse> {
            Box::pin(async move { self.upload(file) })
        }

        fn prepare_file_upload(
            &self,
            _id: &str,
            _is_group: bool,
            request: FileUploadPrepareRequest,
        ) -> QqFuture<'_, FileUploadPrepareResponse> {
            self.prepares
                .lock()
                .expect("预上传记录锁未中毒")
                .push(request);
            let mut prepared = self.chunked.lock().expect("分片槽锁未中毒");
            let prepared = prepared.take().expect("测试未配置分片上传脚本");
            Box::pin(async move { prepared })
        }

        fn finish_file_upload_part(
            &self,
            _id: &str,
            _is_group: bool,
            request: FileUploadPartFinishRequest,
        ) -> QqFuture<'_, ()> {
            self.part_finishes
                .lock()
                .expect("分片完成记录锁未中毒")
                .push(request);
            Box::pin(async { Ok(()) })
        }

        fn merge_file_upload(
            &self,
            _id: &str,
            _is_group: bool,
            request: FileUploadMergeRequest,
        ) -> QqFuture<'_, MediaResponse> {
            self.merges
                .lock()
                .expect("合并记录锁未中毒")
                .push(request);
            // 合并结果单独存放：prepare 已把整份分片脚本取走。
            let mut results = self.chunk_merge_results.lock().expect("合并槽锁未中毒");
            let result = results.take().expect("测试未配置分片合并结果");
            Box::pin(async move { result })
        }

        fn prepare_media_image(
            &self,
            _request: MediaImagePrepareRequest,
        ) -> QqFuture<'_, MediaImagePrepareResponse> {
            Box::pin(async {
                Ok(MediaImagePrepareResponse {
                    upload_id: "up-1".to_owned(),
                    presigned_url: "https://cos.example.com/up-1".to_owned(),
                    expires_in: 3600,
                })
            })
        }

        fn put_presigned_image(
            &self,
            presigned_url: &str,
            _content_type: &str,
            bytes: Vec<u8>,
        ) -> QqFuture<'_, String> {
            self.chunk_puts
                .lock()
                .expect("分片 PUT 记录锁未中毒")
                .push((presigned_url.to_owned(), bytes.len()));
            Box::pin(async move {
                assert!(!bytes.is_empty(), "直传字节不应为空");
                Ok("etag-md5".to_owned())
            })
        }

        fn complete_media_image(
            &self,
            _request: MediaImageCompleteRequest,
        ) -> QqFuture<'_, MediaImageCompleteResponse> {
            let mut results = self.media_results.lock().expect("素材槽锁未中毒");
            assert!(!results.is_empty(), "测试未配置素材链结果");
            let url = results.remove(0);
            Box::pin(async move {
                match url {
                    Some(url) => Ok(MediaImageCompleteResponse {
                        image_id: "img-1".to_owned(),
                        url,
                        ttl: 7_776_000,
                        preview_url: "https://cos.example.com/p".to_owned(),
                    }),
                    None => Err(Error::Upstream {
                        code: 40_035_007,
                        message: "图片素材库空间不足".to_owned(),
                    }),
                }
            })
        }
    }

    /// 假下载出口：固定响应体，记录每次请求。
    #[derive(Debug, Clone)]
    struct FakeTransport {
        body: Vec<u8>,
        requests: Arc<Mutex<Vec<FetchRequest>>>,
    }

    impl FakeTransport {
        fn new(body: &[u8]) -> Self {
            Self {
                body: body.to_vec(),
                requests: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn requests(&self) -> Vec<FetchRequest> {
            self.requests.lock().expect("下载记录锁未中毒").clone()
        }
    }

    impl MediaTransport for FakeTransport {
        fn get(
            &self,
            request: FetchRequest,
        ) -> BoxFuture<'_, gensokyo_media::Result<TransportResponse>> {
            self.requests
                .lock()
                .expect("下载记录锁未中毒")
                .push(request);
            let body = self.body.clone();
            Box::pin(async move {
                Ok(TransportResponse {
                    status: 200,
                    content_type: Some("image/png".to_owned()),
                    body,
                })
            })
        }
    }

    /// 假上传递出：成功时按调用序号给 `https://cdn.example/<n>.png`（Bilibili 响应形状），
    /// 非 200 时只回状态码（后端映射成不可重试的 `HttpStatus`，用例不必真等退避）。
    #[derive(Debug, Clone)]
    struct FakeHosting {
        status: u16,
        requests: Arc<Mutex<Vec<UploadRequest>>>,
    }

    impl FakeHosting {
        fn ok() -> Self {
            Self::with_status(200)
        }

        fn with_status(status: u16) -> Self {
            Self {
                status,
                requests: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn requests(&self) -> Vec<UploadRequest> {
            self.requests.lock().expect("上传记录锁未中毒").clone()
        }
    }

    impl HostingTransport for FakeHosting {
        fn send(
            &self,
            request: UploadRequest,
        ) -> BoxFuture<'_, gensokyo_media::Result<UploadResponse>> {
            let index = {
                let mut requests = self.requests.lock().expect("上传记录锁未中毒");
                requests.push(request);
                requests.len()
            };
            let status = self.status;
            Box::pin(async move {
                let body = if status == 200 {
                    format!(
                        r#"{{"code":0,"data":{{"location":"https://cdn.example/{index}.png"}}}}"#
                    )
                    .into_bytes()
                } else {
                    Vec::new()
                };
                Ok(UploadResponse { status, body })
            })
        }
    }

    /// oss_type=5 的可用配置（Bilibili 只要 csrf_token + sessdata）。
    const BILIBILI_SETTINGS: &str =
        "  oss_type: 5\n  bilibili:\n    csrf_token: csrf\n    sessdata: sess\n    bucket: b\n";

    /// oss_type=1~3 的可用配置：三后端凭据全填，后端只读自己那一段。
    fn oss_settings(oss_type: i64) -> String {
        const REST: &str = "  app_id: 10001
  t_COS_BUCKETNAME: bucket-125
  t_COS_REGION: ap-shanghai
  t_COS_SECRETID: t-id
  t_COS_SECRETKEY: t-key
  b_BOS_BUCKETNAME: bj.bcebos.com
  b_BCE_AK: b-ak
  b_BCE_SK: b-sk
  a_OSS_EndPoint: oss-cn-hangzhou.aliyuncs.com
  a_OSS_BucketName: a-bucket
  a_OSS_AccessKeyId: a-id
  a_OSS_AccessKeySecret: a-secret
";
        format!("  oss_type: {oss_type}\n{REST}")
    }

    fn request(target: SendTarget, plan: OutboundPlan) -> SendRequest {
        SendRequest {
            target,
            message: MessageOut {
                text: String::new(),
                plan,
                auto_escape: false,
            },
            wakeup: false,
            echo_context: EchoContext::default(),
        }
    }

    fn group_request(plan: OutboundPlan) -> SendRequest {
        request(SendTarget::Group(OPENID.to_owned()), plan)
    }

    /// 默认图床运行时：惰性客户端，测试里真正上传的用例都注入专用假传输。
    static TEST_HOSTING: LazyLock<HostingRuntime> = LazyLock::new(HostingRuntime::default);

    fn deps<'a>(media: &'a MediaRuntime, echo: Option<&'a EchoStore>) -> DeferredDeps<'a> {
        deps_with_hub(media, echo, None)
    }

    /// 带事件总线注入的依赖（wakeup 终局回推用例）；`hub` 传 `None` 即「未装配」形态。
    fn deps_with_hub<'a>(
        media: &'a MediaRuntime,
        echo: Option<&'a EchoStore>,
        hub: Option<&'a EventHub>,
    ) -> DeferredDeps<'a> {
        deps_with_hosting(media, echo, hub, &TEST_HOSTING)
    }

    /// 带上传出口注入的依赖（`oss_type` 1~3 内置云 OSS 与 4~6/8~10 图床的 markdown 图片用例）。
    fn deps_with_hosting<'a>(
        media: &'a MediaRuntime,
        echo: Option<&'a EchoStore>,
        hub: Option<&'a EventHub>,
        hosting: &'a HostingRuntime,
    ) -> DeferredDeps<'a> {
        // 测试统一零间隔：合并转发用例免真睡 2×500ms（评审 ③）；不限速：并行用例共享
        // 进程级桶会打满突发额度，等待超时后重试推高 `msg_seq`（见 `limiter` 字段文档）
        DeferredDeps {
            media,
            hosting,
            echo,
            hub,
            forward_send_interval: Duration::ZERO,
            limiter: None,
        }
    }

    /// 临时 idmap（redb + bbolt 同构的身份库），返回值里的 `TempDir` 必须一直持有。
    fn idmap_store() -> (TempDir, Arc<IdMapStore>) {
        let dir = tempfile::tempdir().expect("临时目录");
        let options = IdMapOptions::new(
            dir.path().join("openid-map.redb"),
            dir.path().join("msgid-map.redb"),
        )
        .with_app_id("10001");
        let store = IdMapStore::open(options).expect("打开 idmap 存储失败");
        (dir, Arc::new(store))
    }

    /// 带自定义 `settings` 段的配置（每行已按两格缩进写成 YAML）。
    fn manager_with(settings: &str) -> (TempDir, Arc<ConfigManager>) {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("config.yml");
        std::fs::write(&path, format!("version: 1\nsettings:\n{settings}"))
            .expect("写测试配置失败");
        let manager = Arc::new(ConfigManager::new(&path).expect("加载配置失败"));
        (dir, manager)
    }

    fn context_with_settings(api: Arc<FakeQqApi>, settings: &str) -> (TempDir, ActionContext) {
        let (dir, manager) = manager_with(settings);
        (dir, ActionContext::new(api, manager))
    }

    /// 四字段齐备的合法卡片计划。
    fn card_plan() -> OutboundPlan {
        OutboundPlan {
            card: vec![CardPayload::from_fields(&CardFields {
                title: Some("标题".to_owned()),
                desc: Some("描述".to_owned()),
                pic: Some("https://example.com/a.png".to_owned()),
                url: Some("https://example.com/a".to_owned()),
            })],
            ..OutboundPlan::default()
        }
    }

    fn text_plan(target: SendTarget, text: &str) -> SendRequest {
        let mut request = request(target, OutboundPlan::default());
        request.message.text = text.to_owned();
        request
    }

    /// deferred 用不到的传输：纯文本 / markdown / card 路径不下载任何东西。
    fn no_media() -> MediaRuntime {
        MediaRuntime::with_transport(Arc::new(FakeTransport::new(b"")))
    }

    fn image_plan(image: MediaRef) -> OutboundPlan {
        OutboundPlan {
            images: vec![image],
            ..OutboundPlan::default()
        }
    }

    // ---- 媒体 ----

    #[tokio::test]
    async fn image_url_is_fetched_then_uploaded() {
        let api = Arc::new(FakeQqApi::with_upload("FILE-INFO").with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let transport = FakeTransport::new(b"PNG-BYTES");
        let media = MediaRuntime::with_transport(Arc::new(transport.clone()));
        let plan = group_request(image_plan(MediaRef::Url(
            "https://example.com/a.png".to_owned(),
        )));

        let message_id = execute(&context, &plan, &deps(&media, None))
            .await
            .expect("URL 图片应发送成功");

        assert_eq!(message_id, "MSG-1");
        let requests = transport.requests();
        assert_eq!(requests.len(), 1, "URL 必须先经下载器");
        assert_eq!(requests[0].url, "https://example.com/a.png");
        assert_eq!(requests[0].max_bytes, MAX_UPLOAD_BYTES);

        let uploads = api.uploads();
        assert_eq!(uploads.len(), 1);
        assert_eq!(uploads[0].file_type, file_type::IMAGE);
        assert_eq!(
            uploads[0].file_data.as_deref(),
            Some(gensokyo_media::encode_base64(b"PNG-BYTES").as_str())
        );
        assert_eq!(uploads[0].file_name.as_deref(), Some("media.png"));

        let sent = api.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(
            sent[0].media.as_ref().map(|media| media.file_info.as_str()),
            Some("FILE-INFO")
        );
        assert_eq!(sent[0].msg_type, Some(msg_type::RICH_MEDIA));
    }

    #[tokio::test]
    async fn local_image_is_read_without_copy_and_keeps_name() {
        let api = Arc::new(FakeQqApi::with_upload("FILE-INFO").with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("cat.png");
        std::fs::write(&path, b"LOCAL-BYTES").expect("写本地图片失败");
        let media = MediaRuntime::default();
        let plan = group_request(image_plan(MediaRef::Local(path.clone())));

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("本地图片应发送成功");

        let uploads = api.uploads();
        assert_eq!(
            uploads[0].file_data.as_deref(),
            Some(gensokyo_media::encode_base64(b"LOCAL-BYTES").as_str())
        );
        assert_eq!(uploads[0].file_name.as_deref(), Some("cat.png"));
        assert!(path.exists(), "本地原文件不能被删除");
    }

    #[tokio::test]
    async fn local_image_over_limit_is_a_request_error() {
        let api = Arc::new(FakeQqApi::with_upload("FILE-INFO").with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("big.png");
        std::fs::write(&path, b"0123456789").expect("写本地图片失败");
        // 上限收到 4 字节：10 字节的本地文件必然超限
        let media =
            MediaRuntime::with_fetcher(Fetcher::new(FakeTransport::new(b"")).with_max_bytes(4));
        let plan = group_request(image_plan(MediaRef::Local(path)));

        let err = execute(&context, &plan, &deps(&media, None))
            .await
            .expect_err("超限必须报错");

        assert!(matches!(err, Error::Protocol(_)), "超限应映射 1400: {err}");
        assert!(api.uploads().is_empty(), "超限不应触发上传");
        assert!(api.sent().is_empty(), "超限不应发送");
    }

    // ---- 分片上传 / 压缩 / 转码 / 卡片图片 ----

    /// 质量 95 编码的 JPEG：压缩阶梯从 75 起步，重编码后字节必然不同。
    fn jpeg_fixture() -> Vec<u8> {
        let mut image = image::RgbImage::new(64, 64);
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            *pixel = image::Rgb([x as u8 * 3, y as u8 * 3, (x + y) as u8]);
        }
        let mut out = Vec::new();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 95);
        encoder.encode_image(&image).expect("编码测试 JPEG");
        out
    }

    #[tokio::test]
    async fn media_over_five_megabytes_uses_chunked_upload() {
        let api = Arc::new(
            FakeQqApi::default()
                .with_chunked("CHUNK-FILE-INFO")
                .with_send("MSG-1"),
        );
        let (_dir, context) = context(Arc::clone(&api));
        let body = vec![7u8; MAX_MEDIA_BYTES as usize + 1];
        let media = MediaRuntime::with_transport(Arc::new(FakeTransport::new(&body)));
        let plan = group_request(image_plan(MediaRef::Url(
            "https://example.com/big.png".to_owned(),
        )));

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect(">5MB 应走分片上传");

        assert!(api.uploads().is_empty(), ">5MB 不应走 file_data 直传");
        let prepares = api.prepares();
        assert_eq!(prepares.len(), 1);
        assert_eq!(prepares[0].file_type, file_type::IMAGE);
        assert_eq!(prepares[0].file_size, (MAX_MEDIA_BYTES + 1).to_string());
        assert_eq!(prepares[0].md5.len(), 32, "全文件 MD5 必须是 32 位十六进制");
        assert_eq!(prepares[0].md5_10m.len(), 32);

        let puts = api.chunk_puts();
        assert_eq!(puts.len(), 2, "两片各一次 PUT");
        assert_eq!(puts[0].0, "https://cos.example.com/part-0");
        assert_eq!(puts[0].1, MAX_MEDIA_BYTES as usize);
        assert_eq!(puts[1].1, 1, "次片是剩余 1 字节");

        let finishes = api.part_finishes();
        assert_eq!(finishes.len(), 2);
        assert_eq!(finishes[0].part_index, 0);
        assert_eq!(finishes[0].block_size, MAX_MEDIA_BYTES.to_string());
        assert_eq!(finishes[1].block_size, "1");

        let merges = api.merges();
        assert_eq!(merges.len(), 1);
        assert_eq!(merges[0].upload_id, "UP-1");
        assert_eq!(merges[0].file_name, "media.png");
        assert_eq!(
            api.sent()[0]
                .media
                .as_ref()
                .map(|media| media.file_info.as_str()),
            Some("CHUNK-FILE-INFO")
        );
    }

    #[tokio::test]
    async fn chunked_prepare_failure_does_not_fall_back_to_direct_upload() {
        let api = Arc::new(
            FakeQqApi::default().with_chunked_prepare_error(Error::Upstream {
                code: 40_034_001,
                message: "prepare failed".to_owned(),
            }),
        );
        let (_dir, context) = context(Arc::clone(&api));
        let body = vec![7u8; MAX_MEDIA_BYTES as usize + 1];
        let media = MediaRuntime::with_transport(Arc::new(FakeTransport::new(&body)));
        let plan = group_request(image_plan(MediaRef::Url(
            "https://example.com/big.png".to_owned(),
        )));

        let err = execute(&context, &plan, &deps(&media, None))
            .await
            .expect_err("预上传失败必须报错");

        assert!(
            matches!(err, Error::Upstream { .. }),
            "应保留上游错误: {err}"
        );
        assert!(api.uploads().is_empty(), "不得降级回 file_data 直传");
        assert!(api.chunk_puts().is_empty(), "预上传失败不应继续 PUT");
        assert!(api.sent().is_empty(), "失败不应发送");
    }

    #[tokio::test]
    async fn local_image_is_compressed_when_image_sizelimit_is_set() {
        let api = Arc::new(FakeQqApi::with_upload("FILE-INFO").with_send("MSG-1"));
        let (_dir, context) = context_with_settings(Arc::clone(&api), "  image_sizelimit: 1\n");
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("cover.jpg");
        let source = jpeg_fixture();
        std::fs::write(&path, &source).expect("写本地图片失败");
        let media = MediaRuntime::default();

        execute(&context, &group_request(image_plan(MediaRef::Local(path))), &deps(&media, None))
            .await
            .expect("压缩后应发送成功");

        let uploads = api.uploads();
        let data = uploads[0].file_data.as_deref().expect("直传路径应带 file_data");
        let decoded = gensokyo_media::decode_base64(data).expect("file_data 应是合法 base64");
        assert_ne!(decoded, source, "命中 image_sizelimit 必须重编码");
        assert!(decoded.starts_with(&[0xFF, 0xD8]), "JPEG 应保持 JPEG");
    }

    /// 假进程出口：ffmpeg 写 PCM，pts 写 Silk；都不启动真实进程。
    #[derive(Debug, Clone, Copy)]
    struct FakeSilkRunner;

    impl ProcessRunner for FakeSilkRunner {
        fn run(&self, program: &str, args: &[String]) -> gensokyo_media::Result<ProcessOutput> {
            match program {
                "ffmpeg" => {
                    std::fs::write(
                        std::path::Path::new(args.last().expect("末参是 PCM 路径")),
                        b"pcm",
                    )
                    .expect("写 PCM");
                }
                "pts" => {
                    let index = args
                        .iter()
                        .position(|arg| arg.as_str() == "-o")
                        .expect("pts 参数含 -o");
                    std::fs::write(std::path::Path::new(&args[index + 1]), b"\x02#!SILK_V3-deferred")
                        .expect("写 Silk");
                }
                other => panic!("意外的程序: {other}"),
            }
            Ok(ProcessOutput {
                status: Some(0),
                stdout: Vec::new(),
                stderr: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn local_record_is_transcoded_before_upload() {
        let api = Arc::new(FakeQqApi::with_upload("FILE-INFO").with_send("MSG-1"));
        let (_dir, context) =
            context_with_settings(Arc::clone(&api), "  record_sampleRate: 24000\n");
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("voice.mp3");
        std::fs::write(&path, b"ID3\x04not-silk").expect("写本地语音失败");
        let media = MediaRuntime::default().with_silk_codec(SilkCodec::with_runner(FakeSilkRunner));

        execute(
            &context,
            &group_request(OutboundPlan {
                records: vec![MediaRef::Local(path)],
                ..OutboundPlan::default()
            }),
            &deps(&media, None),
        )
        .await
        .expect("语音转码后应发送成功");

        let uploads = api.uploads();
        assert_eq!(uploads[0].file_type, file_type::VOICE);
        let data = uploads[0].file_data.as_deref().expect("应带 file_data");
        assert_eq!(
            gensokyo_media::decode_base64(data).expect("file_data 应是合法 base64"),
            b"\x02#!SILK_V3-deferred"
        );
    }

    #[tokio::test]
    async fn url_record_is_transcoded_too() {
        let api = Arc::new(FakeQqApi::with_upload("FILE-INFO").with_send("MSG-1"));
        let (_dir, context) =
            context_with_settings(Arc::clone(&api), "  record_sampleRate: 24000\n");
        let media = MediaRuntime::with_transport(Arc::new(FakeTransport::new(
            b"ID3\x04url-audio",
        )))
        .with_silk_codec(SilkCodec::with_runner(FakeSilkRunner));

        execute(
            &context,
            &group_request(OutboundPlan {
                records: vec![MediaRef::Url("https://example.com/a.mp3".to_owned())],
                ..OutboundPlan::default()
            }),
            &deps(&media, None),
        )
        .await
        .expect("URL 语音也应转码后发送");

        let uploads = api.uploads();
        let data = uploads[0].file_data.as_deref().expect("应带 file_data");
        assert_eq!(
            gensokyo_media::decode_base64(data).expect("file_data 应是合法 base64"),
            b"\x02#!SILK_V3-deferred"
        );
    }

    fn card_with_pic(pic: Option<String>) -> OutboundPlan {
        OutboundPlan {
            card: vec![CardPayload::from_fields(&CardFields {
                title: Some("标题".to_owned()),
                desc: Some("描述".to_owned()),
                pic,
                url: Some("https://example.com/a".to_owned()),
            })],
            ..OutboundPlan::default()
        }
    }

    #[tokio::test]
    async fn card_local_pic_is_uploaded_to_public_url() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context_with_settings(Arc::clone(&api), BILIBILI_SETTINGS);
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("cover.jpg");
        std::fs::write(&path, jpeg_fixture()).expect("写封面图失败");
        let hosting = HostingRuntime::with_transport(Arc::new(FakeHosting::ok()));
        let plan = group_request(card_with_pic(Some(
            path.to_string_lossy().into_owned(),
        )));

        execute(
            &context,
            &plan,
            &deps_with_hosting(&no_media(), None, None, &hosting),
        )
        .await
        .expect("本地封面图应上传后发送成功");

        let sent = api.sent();
        assert_eq!(sent.len(), 1);
        let card = sent[0].card.as_ref().expect("卡片体");
        assert_eq!(
            card.content.pic_url.as_deref(),
            Some("https://cdn.example/1.png"),
            "本地路径应换成上传后的公网 URL"
        );
    }

    #[tokio::test]
    async fn card_local_pic_upload_failure_keeps_original_and_is_400() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context_with_settings(Arc::clone(&api), BILIBILI_SETTINGS);
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("cover.jpg");
        std::fs::write(&path, jpeg_fixture()).expect("写封面图失败");
        let hosting = HostingRuntime::with_transport(Arc::new(FakeHosting::with_status(400)));
        let plan = group_request(card_with_pic(Some(
            path.to_string_lossy().into_owned(),
        )));

        let err = execute(
            &context,
            &plan,
            &deps_with_hosting(&no_media(), None, None, &hosting),
        )
        .await
        .expect_err("上传失败保留本地路径后应被 https 校验拦下");

        assert!(matches!(err, Error::Protocol(_)), "应映射 1400: {err}");
        assert!(api.sent().is_empty(), "非法卡片不应发送");
    }

    #[tokio::test]
    async fn two_images_without_markdown_send_two_rich_media_messages() {
        let api = Arc::new(
            FakeQqApi::default()
                .with_send("MSG-A")
                .with_send("MSG-B"),
        );
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        // `Unknown` 已是上游 file_info：免上传，只验拆条与顺序
        let plan = group_request(OutboundPlan {
            images: vec![
                MediaRef::Unknown("FILE-A".to_owned()),
                MediaRef::Unknown("FILE-B".to_owned()),
            ],
            ..OutboundPlan::default()
        });

        let message_id = execute(&context, &plan, &deps(&media, None))
            .await
            .expect("两图应拆成两条发送");

        assert_eq!(message_id, "MSG-B", "回执取最后一条");
        let sent = api.sent();
        assert_eq!(sent.len(), 2, "每图一条 msg_type=7");
        assert!(
            sent.iter()
                .all(|message| message.msg_type == Some(msg_type::RICH_MEDIA)),
            "每条都是富媒体消息"
        );
        assert!(
            sent.iter().all(|message| message.content.is_none()),
            "纯媒体条没有正文"
        );
        let infos = sent
            .iter()
            .map(|message| message.media.as_ref().map(|media| media.file_info.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            infos,
            vec![Some("FILE-A"), Some("FILE-B")],
            "媒体条按计划顺序发出"
        );
    }

    #[tokio::test]
    async fn upload_failure_is_an_upstream_error() {
        let api = Arc::new(FakeQqApi::with_upload_error(Error::Upstream {
            code: 40034001,
            message: "file too large".to_owned(),
        }));
        let (_dir, context) = context(Arc::clone(&api));
        let media = MediaRuntime::with_transport(Arc::new(FakeTransport::new(b"PNG-BYTES")));
        let plan = group_request(image_plan(MediaRef::Url(
            "https://example.com/a.png".to_owned(),
        )));

        let err = execute(&context, &plan, &deps(&media, None))
            .await
            .expect_err("上传失败必须报错");

        assert!(
            matches!(err, Error::Upstream { .. }),
            "应保留上游错误: {err}"
        );
        assert!(api.sent().is_empty(), "上传失败不应继续发送");
    }

    #[tokio::test]
    async fn unknown_media_is_passed_through_as_file_info() {
        let api = Arc::new(FakeQqApi::with_upload("FILE-INFO").with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = MediaRuntime::with_transport(Arc::new(FakeTransport::new(b"x")));
        let plan = group_request(image_plan(MediaRef::Unknown("FILE-INFO".to_owned())));

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("未知媒体按 file_info 透传");

        assert!(api.uploads().is_empty(), "透传不应再上传");
        assert_eq!(
            api.sent()[0]
                .media
                .as_ref()
                .map(|media| media.file_info.as_str()),
            Some("FILE-INFO")
        );
    }

    // ---- reply / echo ----

    #[tokio::test]
    async fn reply_without_mapping_is_a_request_error() {
        let api = Arc::new(FakeQqApi::with_upload("FILE-INFO").with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = MediaRuntime::with_transport(Arc::new(FakeTransport::new(b"x")));
        let plan = group_request(OutboundPlan {
            reply_msg_id: Some(Ob11Id::Num(424_242)),
            ..OutboundPlan::default()
        });

        let err = execute(&context, &plan, &deps(&media, None))
            .await
            .expect_err("无映射的虚拟 reply 必须报错");

        assert!(matches!(err, Error::Protocol(_)), "应映射 1400: {err}");
        assert!(
            err.to_string().contains("无法还原"),
            "错误文案要说清原因: {err}"
        );
        assert!(api.sent().is_empty(), "reply 失败不应发送");
    }

    #[tokio::test]
    async fn reply_via_echo_sets_msg_id() {
        let api = Arc::new(FakeQqApi::with_upload("FILE-INFO").with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = MediaRuntime::with_transport(Arc::new(FakeTransport::new(b"x")));
        let echo = EchoStore::new();
        let row = echo
            .store_cache_in_memory("REAL-MSG-ID")
            .expect("登记虚拟号");
        let plan = group_request(OutboundPlan {
            reply_msg_id: Some(Ob11Id::Num(row)),
            ..OutboundPlan::default()
        });

        execute(&context, &plan, &deps(&media, Some(&echo)))
            .await
            .expect("echo 能反查时应发送成功");

        assert_eq!(api.sent()[0].msg_id.as_deref(), Some("REAL-MSG-ID"));
    }

    #[tokio::test]
    async fn reply_with_real_string_id_passes_through() {
        let api = Arc::new(FakeQqApi::with_upload("FILE-INFO").with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = MediaRuntime::with_transport(Arc::new(FakeTransport::new(b"x")));
        let plan = group_request(OutboundPlan {
            reply_msg_id: Some(Ob11Id::str("REAL-MSG-ID")),
            ..OutboundPlan::default()
        });

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("字符串形态直接当真实 ID");

        assert_eq!(api.sent()[0].msg_id.as_deref(), Some("REAL-MSG-ID"));
    }

    #[tokio::test]
    async fn send_success_writes_message_id_into_echo() {
        let api = Arc::new(FakeQqApi::with_upload("FILE-INFO").with_send("MSG-9"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = MediaRuntime::with_transport(Arc::new(FakeTransport::new(b"x")));
        let echo = EchoStore::new();
        let plan = request(
            SendTarget::Private(OPENID.to_owned()),
            OutboundPlan::default(),
        );

        execute(&context, &plan, &deps(&media, Some(&echo)))
            .await
            .expect("纯文本应发送成功");

        // 行号是 MD5 派生的稳定值：用独立实例算出期望值，再查被测实例，才能证明回写真的发生了
        let expected_row = EchoStore::new()
            .store_cache_in_memory("MSG-9")
            .expect("换算行号");
        assert_eq!(
            echo.get_cache_id_from_memory_by_row_id(&expected_row.to_string()),
            Some("MSG-9".to_owned())
        );
    }

    // ---- markdown ----

    #[tokio::test]
    async fn markdown_plan_sends_msg_type_two_with_content_only() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        let plan = request(
            SendTarget::Group(OPENID.to_owned()),
            OutboundPlan {
                markdown: vec![MarkdownPayload::from_json(
                    r##"{"content":"# 标题","custom_template_id":"TPL","params":[{"key":"a","values":["b"]}]}"##,
                )
                .expect("markdown 载荷")],
                ..OutboundPlan::default()
            },
        );

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("markdown 应发送成功");

        let sent = api.sent();
        assert_eq!(sent[0].msg_type, Some(msg_type::MARKDOWN));
        assert_eq!(
            sent[0]
                .markdown
                .as_ref()
                .and_then(|markdown| markdown.content.as_deref()),
            Some("# 标题")
        );
        assert_eq!(sent[0].content, None, "markdown 消息的 content 必须为空");

        // 出站 JSON 只有 content：模板字段已废弃，不构造也不透传
        let value = serde_json::to_value(&sent[0]).expect("序列化失败");
        assert_eq!(value["markdown"], serde_json::json!({"content": "# 标题"}));
    }

    // ---- keyboard ----

    fn keyboard_plan(raw: &str) -> OutboundPlan {
        OutboundPlan {
            keyboard: vec![KeyboardPayload {
                raw: raw.to_owned(),
            }],
            ..OutboundPlan::default()
        }
    }

    #[tokio::test]
    async fn keyboard_is_attached_without_changing_msg_type() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        let mut plan = group_request(keyboard_plan(r#"{"content":{"rows":[]}}"#));
        plan.message.text = "带按钮的文本".to_owned();

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("键盘 + 文本应发送成功");

        let sent = api.sent();
        assert_eq!(sent[0].content.as_deref(), Some("带按钮的文本"));
        assert_eq!(sent[0].msg_type, None, "键盘不改变 msg_type（Go 同口径）");
        assert_eq!(
            sent[0].keyboard,
            Some(serde_json::json!({"content": {"rows": []}}))
        );
    }

    #[tokio::test]
    async fn keyboard_rides_along_with_markdown() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        let plan = group_request(OutboundPlan {
            markdown: vec![
                MarkdownPayload::from_json(r##"{"content":"# 标题"}"##).expect("markdown 载荷"),
            ],
            keyboard: vec![KeyboardPayload {
                raw: r#"{"id":"tpl_1"}"#.to_owned(),
            }],
            ..OutboundPlan::default()
        });

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("markdown + 键盘应发送成功");

        let sent = api.sent();
        assert_eq!(sent[0].msg_type, Some(msg_type::MARKDOWN));
        assert_eq!(
            sent[0]
                .markdown
                .as_ref()
                .and_then(|markdown| markdown.content.as_deref()),
            Some("# 标题")
        );
        assert_eq!(
            sent[0].keyboard,
            Some(serde_json::json!({"id": "tpl_1"})),
            "键盘必须与 markdown 同条挂上"
        );
    }

    #[tokio::test]
    async fn invalid_keyboard_payload_is_a_request_error() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        let plan = group_request(keyboard_plan("not-json"));

        let err = execute(&context, &plan, &deps(&media, None))
            .await
            .expect_err("非法 keyboard 载荷必须报错");

        assert!(matches!(err, Error::Protocol(_)), "应映射 1400: {err}");
        assert!(api.sent().is_empty(), "非法键盘不应发送");
    }

    // ---- input_notify ----

    fn input_notify_plan(target: SendTarget, second: Option<i64>) -> SendRequest {
        request(
            target,
            OutboundPlan {
                input_notify: Some(InputNotifyPayload { kind: 1, second }),
                ..OutboundPlan::default()
            },
        )
    }

    #[tokio::test]
    async fn input_notify_private_sends_msg_type_six() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        let plan = input_notify_plan(SendTarget::Private(OPENID.to_owned()), None);

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("单聊输入状态应发送成功");

        let sent = api.sent();
        assert_eq!(sent[0].msg_type, Some(msg_type::INPUT_NOTIFY));
        assert_eq!(
            sent[0].input_notify,
            Some(InputNotify {
                input_type: 1,
                input_second: 60,
            }),
            "second 缺省 60（Go 同口径）"
        );
        let value = serde_json::to_value(&sent[0]).expect("序列化失败");
        assert_eq!(
            value["input_notify"],
            serde_json::json!({"input_type": 1, "input_second": 60})
        );
    }

    #[tokio::test]
    async fn input_notify_keeps_explicit_seconds() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        let plan = input_notify_plan(SendTarget::Private(OPENID.to_owned()), Some(5));

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("单聊输入状态应发送成功");

        assert_eq!(
            api.sent()[0].input_notify,
            Some(InputNotify {
                input_type: 1,
                input_second: 5,
            })
        );
    }

    #[tokio::test]
    async fn input_notify_in_group_is_a_request_error() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        let plan = input_notify_plan(SendTarget::Group(OPENID.to_owned()), None);

        let err = execute(&context, &plan, &deps(&media, None))
            .await
            .expect_err("群聊输入状态必须报错");

        assert!(matches!(err, Error::Protocol(_)), "应映射 1400: {err}");
        assert!(api.sent().is_empty(), "群聊输入状态不应发送");
    }

    #[tokio::test]
    async fn input_notify_with_text_is_a_request_error() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        let mut plan = input_notify_plan(SendTarget::Private(OPENID.to_owned()), None);
        plan.message.text = "正文".to_owned();

        let err = execute(&context, &plan, &deps(&media, None))
            .await
            .expect_err("输入状态与正文同条必须报错");

        assert!(matches!(err, Error::Protocol(_)), "应映射 1400: {err}");
        assert!(api.sent().is_empty(), "混用不应发送");
    }

    // ---- stream ----

    fn stream_plan(
        target: SendTarget,
        kind: StreamKind,
        id: Option<&str>,
        index: Option<i64>,
    ) -> SendRequest {
        request(
            target,
            OutboundPlan {
                stream: Some(StreamPayload {
                    kind,
                    qq: Ob11Id::str(OPENID),
                    id: id.map(str::to_owned),
                    index,
                    seq: None,
                }),
                ..OutboundPlan::default()
            },
        )
    }

    #[tokio::test]
    async fn stream_start_sends_markdown_with_state_one() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        let mut plan = stream_plan(
            SendTarget::Private(OPENID.to_owned()),
            StreamKind::Start,
            None,
            None,
        );
        plan.message.text = "你好".to_owned();

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("单聊流式首片应发送成功");

        let sent = api.sent();
        assert_eq!(sent[0].msg_type, Some(msg_type::MARKDOWN));
        assert_eq!(
            sent[0]
                .markdown
                .as_ref()
                .and_then(|markdown| markdown.content.as_deref()),
            Some("你好"),
            "正文进 markdown.content（Go MessageSSE 口径）"
        );
        let stream = sent[0].stream.as_ref().expect("应带 stream");
        assert_eq!(stream.state, 1);
        assert_eq!(stream.index, 0);
        assert_eq!(stream.id, None);
    }

    #[tokio::test]
    async fn stream_finish_sends_state_ten_with_stream_id() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        let plan = stream_plan(
            SendTarget::Private(OPENID.to_owned()),
            StreamKind::Finish,
            Some("STREAM-1"),
            Some(2),
        );

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("单聊流式终片应发送成功");

        let sent = api.sent();
        assert_eq!(sent[0].markdown, None, "没有正文时不出 markdown 字段");
        let stream = sent[0].stream.as_ref().expect("应带 stream");
        assert_eq!(stream.state, 10, "finish 的 state 是 10");
        assert_eq!(stream.index, 2);
        assert_eq!(stream.id.as_deref(), Some("STREAM-1"));
    }

    #[tokio::test]
    async fn stream_mid_without_id_is_a_request_error() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        let plan = stream_plan(
            SendTarget::Private(OPENID.to_owned()),
            StreamKind::Mid,
            None,
            Some(1),
        );

        let err = execute(&context, &plan, &deps(&media, None))
            .await
            .expect_err("续片缺 stream_msg_id 必须报错");

        assert!(matches!(err, Error::Protocol(_)), "应映射 1400: {err}");
        assert!(api.sent().is_empty(), "缺 id 不应发送");
    }

    #[tokio::test]
    async fn stream_in_group_is_a_request_error() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        let plan = stream_plan(
            SendTarget::Group(OPENID.to_owned()),
            StreamKind::Start,
            None,
            None,
        );

        let err = execute(&context, &plan, &deps(&media, None))
            .await
            .expect_err("群聊流式必须报错");

        assert!(matches!(err, Error::Protocol(_)), "应映射 1400: {err}");
        assert!(api.sent().is_empty(), "群聊流式不应发送");
    }

    // ---- music ----

    /// `get_song_detail_yqq` 的最小响应（字段口径同 Go `track_info`）。
    const MUSIC_JSON: &str = r#"{"songinfo":{"data":{"track_info":{"mid":"003OUlho2HcRHC","name":"晴天","singer":[{"name":"周杰伦"}],"album":{"mid":"000MkMni19ClKG"}}}}}"#;

    fn music_json_media(body: &str) -> MediaRuntime {
        MediaRuntime::with_transport(Arc::new(FakeTransport::new(body.as_bytes())))
    }

    fn music_plan() -> SendRequest {
        group_request(OutboundPlan {
            qqmusic: vec!["003OUlho2HcRHC".to_owned()],
            ..OutboundPlan::default()
        })
    }

    #[tokio::test]
    async fn music_is_queried_and_sent_as_markdown_with_keyboard() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-MUSIC"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = music_json_media(MUSIC_JSON);
        let plan = music_plan();

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("音乐应查曲目后发送 markdown");

        let sent = api.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].msg_type, Some(msg_type::MARKDOWN));
        assert_eq!(sent[0].content, None, "markdown 消息的 content 必须为空");
        assert_eq!(
            sent[0]
                .markdown
                .as_ref()
                .and_then(|markdown| markdown.content.as_deref()),
            Some(
                "![图片 #180px #180px](https://y.gtimg.cn/music/photo_new/T002R180x180M000000MkMni19ClKG.jpg)\r晴天\r周杰伦"
            )
        );
        // 键盘逐字段照 Go `createMusicKeyboard`；配置缺省时 music_prefix 是空串，data 仍带那个空格
        assert_eq!(
            sent[0].keyboard,
            Some(serde_json::json!({
                "content": {"rows": [{"buttons": [
                    {
                        "render_data": {"label": "立刻播放", "visited_label": "已播放", "style": 1},
                        "action": {
                            "type": 0,
                            "permission": {"type": 2},
                            "data": "https://i.y.qq.com/v8/playsong.html?platform=11&appshare=android_qq&appversion=10030010&hosteuin=oKnlNenz7i-s7c**&songmid=003OUlho2HcRHC&type=0&appsongtype=1&_wv=1&source=qq&ADTAG=qfshare",
                            "unsupport_tips": "请升级新版手机QQ",
                            "anchor": 0,
                            "enter": false,
                            "reply": false
                        }
                    },
                    {
                        "render_data": {"label": "我也要点歌", "visited_label": "再次点歌", "style": 1},
                        "action": {
                            "type": 2,
                            "permission": {"type": 2},
                            "data": " ",
                            "unsupport_tips": "请升级新版手机QQ",
                            "anchor": 0,
                            "enter": false,
                            "reply": false
                        }
                    }
                ]}]}
            }))
        );
    }

    #[tokio::test]
    async fn music_keyboard_uses_configured_prefix() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-MUSIC"));
        let (_dir, context) = context_with_settings(Arc::clone(&api), "  music_prefix: 点歌\n");
        let media = music_json_media(MUSIC_JSON);

        execute(&context, &music_plan(), &deps(&media, None))
            .await
            .expect("音乐应发送成功");

        let sent = api.sent();
        let keyboard = sent[0].keyboard.as_ref().expect("音乐消息必须带键盘");
        assert_eq!(
            keyboard.pointer("/content/rows/0/buttons/1/action/data"),
            Some(&serde_json::json!("点歌 "))
        );
    }

    #[tokio::test]
    async fn music_query_failure_falls_back_to_share_text() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-TEXT"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = music_json_media("not json");
        let plan = music_plan();

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("查询失败仍应发出分享文案");

        let sent = api.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(
            sent[0].content.as_deref(),
            Some("分享一首 QQ 音乐：003OUlho2HcRHC"),
            "查询失败保留原降级文案"
        );
        assert_eq!(sent[0].msg_type, None, "降级后是普通文本消息");
        assert_eq!(sent[0].keyboard, None, "降级后不伪造键盘");
    }

    #[tokio::test]
    async fn music_with_markdown_is_sent_as_a_second_message() {
        let api = Arc::new(
            FakeQqApi::default()
                .with_send("MSG-MD")
                .with_send("MSG-MUSIC"),
        );
        let (_dir, context) = context(Arc::clone(&api));
        let media = music_json_media(MUSIC_JSON);
        let plan = group_request(OutboundPlan {
            qqmusic: vec!["003OUlho2HcRHC".to_owned()],
            ..markdown_plan("# 标题")
        });

        let message_id = execute(&context, &plan, &deps(&media, None))
            .await
            .expect("markdown 与音乐应各成一条（Go foundItems 循环口径）");

        assert_eq!(message_id, "MSG-MUSIC", "回执取最后一条（音乐条）");
        let sent = api.sent();
        assert_eq!(sent.len(), 2, "markdown 一条 + 音乐一条");
        assert_eq!(sent[0].msg_type, Some(msg_type::MARKDOWN));
        assert_eq!(
            sent[0]
                .markdown
                .as_ref()
                .and_then(|markdown| markdown.content.as_deref()),
            Some("# 标题"),
            "markdown 仍是主消息"
        );
        assert_eq!(sent[1].msg_type, Some(msg_type::MARKDOWN));
        assert!(
            sent[1]
                .markdown
                .as_ref()
                .and_then(|markdown| markdown.content.as_deref())
                .is_some_and(|content| content.contains("晴天")),
            "音乐另起一条，不被 markdown 吞掉"
        );
        assert!(sent[1].keyboard.is_some(), "音乐条自带键盘");
    }

    #[tokio::test]
    async fn music_with_card_sends_card_then_music() {
        let api = Arc::new(
            FakeQqApi::default()
                .with_send("MSG-CARD")
                .with_send("MSG-MUSIC"),
        );
        let (_dir, context) = context(Arc::clone(&api));
        let media = music_json_media(MUSIC_JSON);
        let mut plan = group_request(card_plan());
        plan.message.plan.qqmusic = vec!["003OUlho2HcRHC".to_owned()];

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("卡片与音乐应各成一条");

        let sent = api.sent();
        assert_eq!(sent.len(), 2, "卡片一条 + 音乐一条");
        assert_eq!(sent[0].msg_type, Some(msg_type::CARD));
        assert_eq!(sent[1].msg_type, Some(msg_type::MARKDOWN));
        assert!(sent[1].keyboard.is_some(), "音乐条自带键盘");
    }

    #[tokio::test]
    async fn music_only_with_cq_keyboard_is_a_request_error() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-MUSIC"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = music_json_media(MUSIC_JSON);
        let mut plan = music_plan();
        plan.message.plan.keyboard = keyboard_plan(r#"{"content":{"rows":[]}}"#).keyboard;

        let err = execute(&context, &plan, &deps(&media, None))
            .await
            .expect_err("纯音乐 + 独立 [CQ:keyboard] 应拒绝（两者抢同一个键盘字段）");

        assert!(matches!(err, Error::Protocol(_)), "应映射 1400: {err}");
        assert!(api.sent().is_empty(), "被拒绝的消息不应发送");
    }

    #[tokio::test]
    async fn unknown_image_is_still_a_request_error() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        let plan = group_request(OutboundPlan {
            unknown_images: vec![MediaRef::Unknown("FILE-INFO".to_owned())],
            ..OutboundPlan::default()
        });

        let err = execute(&context, &plan, &deps(&media, None))
            .await
            .expect_err("unknown_image 仍是 1400");

        assert!(matches!(err, Error::Protocol(_)), "应映射 1400: {err}");
        assert!(api.sent().is_empty(), "不支持的段不应发送");
    }

    #[tokio::test]
    async fn markdown_without_content_is_a_request_error() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        let plan = request(
            SendTarget::Group(OPENID.to_owned()),
            OutboundPlan {
                markdown: vec![
                    MarkdownPayload::from_json(r#"{"custom_template_id":"TPL"}"#)
                        .expect("markdown 载荷"),
                ],
                ..OutboundPlan::default()
            },
        );

        let err = execute(&context, &plan, &deps(&media, None))
            .await
            .expect_err("缺 content 必须拒绝");
        assert!(matches!(err, Error::Protocol(_)), "应映射 1400: {err}");
        assert!(api.sent().is_empty(), "非法 markdown 不应发送");
    }

    // ---- card ----

    #[tokio::test]
    async fn card_plan_sends_msg_type_eight_for_group_and_private() {
        for target in [
            SendTarget::Group(OPENID.to_owned()),
            SendTarget::Private(OPENID.to_owned()),
        ] {
            let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
            let (_dir, context) = context(Arc::clone(&api));
            let media = no_media();
            let plan = request(target, card_plan());

            execute(&context, &plan, &deps(&media, None))
                .await
                .expect("卡片应发送成功");

            let sent = api.sent();
            assert_eq!(
                sent[0].msg_type,
                Some(msg_type::CARD),
                "卡片必须是 msg_type=8"
            );
            assert_eq!(sent[0].content, None, "卡片消息的 content 必须为空");
            let card = sent[0].card.as_ref().expect("card 必须有值");
            assert_eq!(card.card_type, "tuwen");
            assert_eq!(card.content.title.as_deref(), Some("标题"));
            assert_eq!(
                card.content.pic_url.as_deref(),
                Some("https://example.com/a.png")
            );
        }
    }

    #[tokio::test]
    async fn card_without_title_is_a_request_error() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        let plan = request(
            SendTarget::Group(OPENID.to_owned()),
            OutboundPlan {
                card: vec![CardPayload::from_fields(&CardFields {
                    desc: Some("有描述没标题".to_owned()),
                    pic: Some("https://example.com/a.png".to_owned()),
                    url: Some("https://example.com/a".to_owned()),
                    ..CardFields::default()
                })],
                ..OutboundPlan::default()
            },
        );

        let err = execute(&context, &plan, &deps(&media, None))
            .await
            .expect_err("缺 title 必须拒绝");
        assert!(matches!(err, Error::Protocol(_)), "应映射 1400: {err}");
        assert!(api.sent().is_empty(), "非法卡片不应发送");
    }

    #[tokio::test]
    async fn markdown_wins_over_card() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        let plan = request(
            SendTarget::Group(OPENID.to_owned()),
            OutboundPlan {
                markdown: vec![
                    MarkdownPayload::from_json(r##"{"content":"# 标题"}"##).expect("markdown 载荷"),
                ],
                card: vec![CardPayload::from_fields(&CardFields {
                    title: Some("卡片标题".to_owned()),
                    ..CardFields::default()
                })],
                ..OutboundPlan::default()
            },
        );

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("markdown 与 card 同现应发送");

        let sent = api.sent();
        assert_eq!(
            sent[0].msg_type,
            Some(msg_type::MARKDOWN),
            "markdown 胜 card"
        );
        assert!(sent[0].card.is_none(), "card 必须让位");
    }

    #[tokio::test]
    async fn markdown_with_one_image_sends_markdown_then_media() {
        let api = Arc::new(
            FakeQqApi::with_upload("FILE-1")
                .with_send("MSG-MD")
                .with_send("MSG-IMG"),
        );
        let (_dir, context) = context(Arc::clone(&api));
        let media = MediaRuntime::with_transport(Arc::new(FakeTransport::new(b"PNG-BYTES")));
        let plan = group_request(OutboundPlan {
            images: vec![MediaRef::Url("https://example.com/a.png".to_owned())],
            ..markdown_plan("# 标题")
        });

        let message_id = execute(&context, &plan, &deps(&media, None))
            .await
            .expect("markdown + 图片应拆成两条发送");

        assert_eq!(message_id, "MSG-IMG", "回执取最后一条（媒体条）");
        let sent = api.sent();
        assert_eq!(sent.len(), 2, "markdown 一条 + 图片一条");
        assert_eq!(sent[0].msg_type, Some(msg_type::MARKDOWN));
        assert!(
            sent[0].content.is_none(),
            "markdown 条正文走 markdown.content"
        );
        assert!(sent[0].media.is_none(), "markdown 条不占 media 槽");
        assert_eq!(sent[1].msg_type, Some(msg_type::RICH_MEDIA));
        assert!(sent[1].markdown.is_none(), "媒体条不带 markdown");
        assert!(sent[1].content.is_none(), "媒体条没有正文");
        assert_eq!(
            sent[1].media.as_ref().map(|media| media.file_info.as_str()),
            Some("FILE-1")
        );
    }

    #[tokio::test]
    async fn markdown_with_two_images_sends_three_messages_in_plan_order() {
        let api = Arc::new(
            FakeQqApi::default()
                .with_send("MSG-MD")
                .with_send("MSG-A")
                .with_send("MSG-B"),
        );
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        let plan = group_request(OutboundPlan {
            // `Unknown` 已是上游 file_info：免上传，只验拆条与顺序
            images: vec![
                MediaRef::Unknown("FILE-A".to_owned()),
                MediaRef::Unknown("FILE-B".to_owned()),
            ],
            ..markdown_plan("# 标题")
        });

        let message_id = execute(&context, &plan, &deps(&media, None))
            .await
            .expect("markdown + 两图应拆成三条发送");

        assert_eq!(message_id, "MSG-B", "回执取最后一条");
        let sent = api.sent();
        assert_eq!(sent.len(), 3, "markdown 一条 + 每图一条");
        assert_eq!(sent[0].msg_type, Some(msg_type::MARKDOWN));
        let infos = sent[1..]
            .iter()
            .map(|message| message.media.as_ref().map(|media| media.file_info.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            infos,
            vec![Some("FILE-A"), Some("FILE-B")],
            "媒体条按计划顺序发出"
        );
    }

    #[tokio::test]
    async fn keyboard_and_reply_ride_on_the_markdown_body_only() {
        let api = Arc::new(
            FakeQqApi::default()
                .with_send("MSG-MD")
                .with_send("MSG-IMG"),
        );
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        let mut plan = group_request(OutboundPlan {
            keyboard: vec![KeyboardPayload {
                raw: r#"{"id":"tpl_1"}"#.to_owned(),
            }],
            images: vec![MediaRef::Unknown("FILE-1".to_owned())],
            ..markdown_plan("# 标题")
        });
        plan.message.plan.reply_msg_id = Some(Ob11Id::str("MSG-REF"));

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("markdown + 键盘 + 图片应发送成功");

        let sent = api.sent();
        assert_eq!(sent.len(), 2);
        assert_eq!(
            sent[0].keyboard,
            Some(serde_json::json!({"id": "tpl_1"})),
            "键盘只挂 markdown 条"
        );
        assert_eq!(
            sent[0].msg_id.as_deref(),
            Some("MSG-REF"),
            "reply 只挂 markdown 条"
        );
        assert!(sent[1].keyboard.is_none(), "媒体条不带键盘");
        assert!(sent[1].msg_id.is_none(), "媒体条不带 reply");
    }

    #[tokio::test]
    async fn failed_media_body_does_not_block_later_bodies() {
        let api = Arc::new(
            FakeQqApi::default()
                .with_send("MSG-MD")
                .with_send_error(Error::Upstream {
                    code: 40_054_005,
                    message: "msgseq 去重".to_owned(),
                })
                .with_send("MSG-B"),
        );
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        let plan = group_request(OutboundPlan {
            images: vec![
                MediaRef::Unknown("FILE-A".to_owned()),
                MediaRef::Unknown("FILE-B".to_owned()),
            ],
            ..markdown_plan("# 标题")
        });

        let message_id = execute(&context, &plan, &deps(&media, None))
            .await
            .expect("单条媒体失败不能挡住后面的条");

        assert_eq!(message_id, "MSG-B", "回执取最后一条");
        assert_eq!(api.sent().len(), 3, "失败的那条不阻断后续发送");
    }

    #[tokio::test]
    async fn failed_first_media_body_does_not_block_the_next_image() {
        let api = Arc::new(
            FakeQqApi::default()
                .with_send_error(Error::Upstream {
                    code: 40_054_005,
                    message: "msgseq 去重".to_owned(),
                })
                .with_send("MSG-B"),
        );
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        let plan = group_request(OutboundPlan {
            images: vec![
                MediaRef::Unknown("FILE-A".to_owned()),
                MediaRef::Unknown("FILE-B".to_owned()),
            ],
            ..OutboundPlan::default()
        });

        let message_id = execute(&context, &plan, &deps(&media, None))
            .await
            .expect("首条失败不能挡住后面的媒体条");

        assert_eq!(message_id, "MSG-B", "回执取最后一条");
        assert_eq!(api.sent().len(), 2, "首条失败不阻断后续发送");
    }

    #[tokio::test]
    async fn card_with_media_sends_card_then_media() {
        let api = Arc::new(
            FakeQqApi::default()
                .with_send("MSG-CARD")
                .with_send("MSG-IMG"),
        );
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        let plan = group_request(OutboundPlan {
            images: vec![MediaRef::Unknown("FILE-1".to_owned())],
            ..card_plan()
        });

        let message_id = execute(&context, &plan, &deps(&media, None))
            .await
            .expect("card 与媒体应拆成两条发送");

        assert_eq!(message_id, "MSG-IMG", "回执取最后一条（媒体条）");
        let sent = api.sent();
        assert_eq!(sent.len(), 2, "卡片一条 + 媒体一条");
        assert_eq!(sent[0].msg_type, Some(msg_type::CARD), "卡片先发");
        assert!(sent[0].card.is_some(), "首条是卡片");
        assert!(sent[0].media.is_none(), "卡片条不占 media 槽");
        assert_eq!(sent[1].msg_type, Some(msg_type::RICH_MEDIA), "媒体后发");
        assert!(sent[1].card.is_none(), "媒体条不是卡片");
        assert_eq!(
            sent[1].media.as_ref().map(|media| media.file_info.as_str()),
            Some("FILE-1")
        );
    }

    // ---- @全员 ----

    #[tokio::test]
    async fn at_everyone_cq_is_rendered_as_tag_and_reserves_quota() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let (_idmap_dir, idmap) = idmap_store();
        let context = context.with_idmap(Arc::clone(&idmap));
        let media = no_media();
        let plan = text_plan(SendTarget::Group(OPENID.to_owned()), "[CQ:at,qq=all] 开会");

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("@全员 应发送成功");

        let sent = api.sent();
        assert_eq!(
            sent[0].content.as_deref(),
            Some("<qqbot-at-everyone /> 开会"),
            "CQ 段必须归一成官方标签，且不留 CQ 残留"
        );
        assert!(
            idmap.read_config(SECTION, OPENID).is_ok(),
            "发送成功应留下当天计数"
        );
    }

    #[tokio::test]
    async fn at_everyone_literal_tag_is_also_accepted() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let (_idmap_dir, idmap) = idmap_store();
        let context = context.with_idmap(Arc::clone(&idmap));
        let media = no_media();
        let plan = text_plan(
            SendTarget::Group(OPENID.to_owned()),
            "<qqbot-at-everyone /> 开会",
        );

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("字面标签应放行");

        assert_eq!(
            api.sent()[0].content.as_deref(),
            Some("<qqbot-at-everyone /> 开会")
        );
        assert!(
            idmap.read_config(SECTION, OPENID).is_ok(),
            "字面标签也要计数"
        );
    }

    #[tokio::test]
    async fn at_everyone_in_private_is_a_request_error() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let (_idmap_dir, idmap) = idmap_store();
        let context = context.with_idmap(Arc::clone(&idmap));
        let media = no_media();
        let plan = text_plan(
            SendTarget::Private(OPENID.to_owned()),
            "<qqbot-at-everyone /> 开会",
        );

        let err = execute(&context, &plan, &deps(&media, None))
            .await
            .expect_err("单聊 @全员 必须拒绝");
        assert!(matches!(err, Error::Protocol(_)), "应映射 1400: {err}");
        assert!(api.sent().is_empty(), "单聊 @全员 不应发送");
    }

    #[tokio::test]
    async fn second_at_everyone_in_same_group_same_day_is_rejected() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let (_idmap_dir, idmap) = idmap_store();
        let context = context.with_idmap(Arc::clone(&idmap));
        let media = no_media();
        let plan = text_plan(SendTarget::Group(OPENID.to_owned()), "[CQ:at,qq=all]开会");

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("首次应放行");
        let err = execute(&context, &plan, &deps(&media, None))
            .await
            .expect_err("同日第二次必须拒绝");

        assert!(matches!(err, Error::Protocol(_)), "超限应映射 1400: {err}");
        assert_eq!(api.sent().len(), 1, "超限的第二次不应发送");
    }

    #[tokio::test]
    async fn whitespace_at_everyone_counts_once_per_day() {
        // 空白写法归一后同样只计 1 次额度：同日第二次（换一种空白写法）回 1400
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let (_idmap_dir, idmap) = idmap_store();
        let context = context.with_idmap(Arc::clone(&idmap));
        let media = no_media();
        let first = text_plan(
            SendTarget::Group(OPENID.to_owned()),
            "[CQ:at, qq = all] 开会",
        );

        execute(&context, &first, &deps(&media, None))
            .await
            .expect("空白写法首次应放行");
        assert_eq!(
            api.sent()[0].content.as_deref(),
            Some("<qqbot-at-everyone /> 开会"),
            "空白写法必须归一为官方标签"
        );

        let second = text_plan(
            SendTarget::Group(OPENID.to_owned()),
            "[CQ:at,qq = ALL] 开会",
        );
        let err = execute(&context, &second, &deps(&media, None))
            .await
            .expect_err("同日第二次必须拒绝");

        assert!(matches!(err, Error::Protocol(_)), "超限应映射 1400: {err}");
        assert_eq!(api.sent().len(), 1, "超限的第二次不应发送");
    }

    #[tokio::test]
    async fn at_everyone_limit_switch_off_allows_repeats_without_counting() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1").with_send("MSG-2"));
        let (_dir, context) =
            context_with_settings(Arc::clone(&api), "  enable_at_everyone_limit: false\n");
        let (_idmap_dir, idmap) = idmap_store();
        let context = context.with_idmap(Arc::clone(&idmap));
        let media = no_media();
        let plan = text_plan(SendTarget::Group(OPENID.to_owned()), "[CQ:at,qq=all]开会");

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("开关关闭时首次应放行");
        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("开关关闭时同日第二次也应放行");

        assert_eq!(api.sent().len(), 2, "开关关闭时不应拦截");
        assert!(
            idmap.read_config(SECTION, OPENID).is_err(),
            "开关关闭时不应写计数"
        );
    }

    #[tokio::test]
    async fn send_failure_rolls_back_the_at_everyone_quota() {
        let api =
            Arc::new(FakeQqApi::default().with_send_error(Error::Network("上游挂了".to_owned())));
        let (_dir, context) = context(Arc::clone(&api));
        let (_idmap_dir, idmap) = idmap_store();
        let context = context.with_idmap(Arc::clone(&idmap));
        let media = no_media();
        let plan = text_plan(SendTarget::Group(OPENID.to_owned()), "[CQ:at,qq=all]开会");

        let err = execute(&context, &plan, &deps(&media, None))
            .await
            .expect_err("上游失败应返回错误");
        assert!(
            matches!(err, Error::Network(_)),
            "发送失败应映射 1500: {err}"
        );

        assert!(
            idmap.read_config(SECTION, OPENID).is_err(),
            "发送失败必须回滚计数"
        );
        crate::at_everyone::reserve(&idmap, OPENID).expect("回滚后应能再次占位");
    }

    #[tokio::test]
    async fn at_everyone_tag_inside_markdown_content_is_reserved_and_kept() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let (_idmap_dir, idmap) = idmap_store();
        let context = context.with_idmap(Arc::clone(&idmap));
        let media = no_media();
        let plan = request(
            SendTarget::Group(OPENID.to_owned()),
            OutboundPlan {
                markdown: vec![
                    MarkdownPayload::from_json(r#"{"content":"[CQ:at,qq=all] 早安"}"#)
                        .expect("markdown 载荷"),
                ],
                ..OutboundPlan::default()
            },
        );

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("markdown @全员 应发送成功");

        let sent = api.sent();
        assert_eq!(sent[0].msg_type, Some(msg_type::MARKDOWN));
        assert_eq!(sent[0].content, None, "markdown 消息的 content 为空");
        assert_eq!(
            sent[0]
                .markdown
                .as_ref()
                .and_then(|markdown| markdown.content.as_deref()),
            Some("<qqbot-at-everyone /> 早安")
        );
        assert!(
            idmap.read_config(SECTION, OPENID).is_ok(),
            "markdown 也要计数"
        );
    }

    #[tokio::test]
    async fn duplicate_at_everyone_tags_collapse_to_one() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let (_idmap_dir, idmap) = idmap_store();
        let context = context.with_idmap(Arc::clone(&idmap));
        let media = no_media();
        // 正文同时带字面标签与 `[CQ:at,qq=all]`，文本侧还有一个：归一后只能留一个
        let mut plan = text_plan(SendTarget::Group(OPENID.to_owned()), "[CQ:at,qq=all] 开会");
        plan.message.plan = markdown_plan("<qqbot-at-everyone /> 议题 [CQ:at,qq=all]");

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("重复标签应归一后发送");

        let content = api.sent()[0]
            .markdown
            .as_ref()
            .and_then(|markdown| markdown.content.clone())
            .expect("markdown 正文");
        assert_eq!(
            content.matches("<qqbot-at-everyone />").count(),
            1,
            "同一份正文只能有一个 @全员 标签: {content}"
        );
        assert_eq!(content.trim_end(), "<qqbot-at-everyone /> 议题");

        // 额度只按一条消息计一次：同群同日第二条必须被限流拒绝
        let err = execute(&context, &plan, &deps(&media, None))
            .await
            .expect_err("同日第二条必须拒绝");
        assert!(matches!(err, Error::Protocol(_)), "超限应映射 1400: {err}");
    }

    #[tokio::test]
    async fn uppercase_cq_at_all_is_treated_as_everyone() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let (_idmap_dir, idmap) = idmap_store();
        let context = context.with_idmap(Arc::clone(&idmap));
        let media = no_media();
        let plan = text_plan(SendTarget::Group(OPENID.to_owned()), "[CQ:at,qq=ALL] 开会");

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("qq=ALL 应归一为 @全员 标签，而不是走 idmap 反查");

        assert_eq!(
            api.sent()[0].content.as_deref(),
            Some("<qqbot-at-everyone /> 开会")
        );
        assert!(
            idmap.read_config(SECTION, OPENID).is_ok(),
            "qq=ALL 同样要占当天额度"
        );
    }

    #[tokio::test]
    async fn at_everyone_with_space_before_comma_is_normalized() {
        // `[CQ:at ,qq=all]` / `[CQ:at , qq = ALL]`：空白形态同样归一为标签并计 1 次额度；
        // `[CQ: at,...]`（`[CQ:` 与 `at` 之间的空白）同批钉桩，防止识别层漏判再次静默丢
        for (text, expected) in [
            ("[CQ:at ,qq=all] 开会", "<qqbot-at-everyone /> 开会"),
            ("[CQ:at , qq = ALL] 开会", "<qqbot-at-everyone /> 开会"),
            ("[CQ: at,qq=all] 开会", "<qqbot-at-everyone /> 开会"),
            ("[CQ: at , qq = ALL] 开会", "<qqbot-at-everyone /> 开会"),
        ] {
            let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
            let (_dir, context) = context(Arc::clone(&api));
            let (_idmap_dir, idmap) = idmap_store();
            let context = context.with_idmap(Arc::clone(&idmap));
            let media = no_media();
            let plan = text_plan(SendTarget::Group(OPENID.to_owned()), text);

            execute(&context, &plan, &deps(&media, None))
                .await
                .expect("空白写法的 @全员 应归一为官方标签");

            assert_eq!(api.sent()[0].content.as_deref(), Some(expected), "{text}");

            // 额度只计一次：同群同日再来一条必须 1400
            let err = execute(&context, &plan, &deps(&media, None))
                .await
                .expect_err("同日第二条必须拒绝");
            assert!(
                matches!(err, Error::Protocol(_)),
                "{text} 的空白写法同样要占额度: {err}"
            );
        }
    }

    // ---- 文本路径的 @用户：缺 qq 原样透传，齐备才反查 ----

    #[tokio::test]
    async fn text_at_without_qq_is_passed_through() {
        // 文本路径缺 qq 不报错：原样透传（与 Go 一致），只有 markdown 才 1400
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        let text = "[CQ:at] 你好 [CQ:at,foo] 世界 [CQ:at,qq=]";
        let plan = text_plan(SendTarget::Group(OPENID.to_owned()), text);

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("文本路径缺 qq 不应报错");

        let sent = api.sent();
        assert_eq!(
            sent[0].content.as_deref(),
            Some(text),
            "缺 qq 的 at 段必须原样保留"
        );
        assert_eq!(sent[0].msg_type, None, "纯文本不应带 msg_type");
    }

    #[tokio::test]
    async fn text_at_with_space_before_comma_still_resolves() {
        // `[CQ:at ,qq=N]` 与紧凑写法同口径：照样反查成 `<@openid>`；
        // 裸 at 段（`[CQ:at ]`）仍按缺 qq 原样透传
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let (_idmap_dir, idmap) = idmap_store();
        let (_, vuin) = idmap
            .retrieve_virtual_value(MEMBER_OPENID)
            .expect("登记身份失败");
        let context = context.with_idmap(Arc::clone(&idmap));
        let media = no_media();
        let plan = text_plan(
            SendTarget::Group(OPENID.to_owned()),
            &format!("[CQ:at ,qq={vuin}] 你好 [CQ:at ] 世界"),
        );

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("文本路径带空白的 at 段不应报错");

        let expected = format!("<@{MEMBER_OPENID}> 你好 [CQ:at ] 世界");
        assert_eq!(
            api.sent()[0].content.as_deref(),
            Some(expected.as_str()),
            "参数齐备的空白写法要反查，裸 at 段（带空白）仍原样透传"
        );
    }

    // ---- AppID ⇄ BotID ----

    /// 正文里的 AppID 换成 BotID，群聊与私聊同一口径（Go 两条 `transformMessageTextAt*`）。
    #[tokio::test]
    async fn text_replaces_app_id_with_bot_id_for_group_and_private() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1").with_send("MSG-2"));
        let (_dir, context) = context_with_settings(Arc::clone(&api), "  app_id: 102001234\n");
        let context = context.with_bot_id("2633000001");
        let media = no_media();

        for target in [
            SendTarget::Group(OPENID.to_owned()),
            SendTarget::Private(OPENID.to_owned()),
        ] {
            let plan = text_plan(target, "机器人 102001234 在线");
            execute(&context, &plan, &deps(&media, None))
                .await
                .expect("文本消息应发送成功");
        }

        let sent = api.sent();
        assert_eq!(sent.len(), 2, "群聊与私聊各一条");
        for body in &sent {
            assert_eq!(body.content.as_deref(), Some("机器人 2633000001 在线"));
        }
    }

    /// CQ 码区间不受替换影响（Go 先占位再 `ReplaceAll`）。
    #[tokio::test]
    async fn text_does_not_replace_app_id_inside_cq_codes() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context_with_settings(Arc::clone(&api), "  app_id: 102001234\n");
        let context = context.with_bot_id("2633000001");
        let media = no_media();
        let plan = text_plan(
            SendTarget::Group(OPENID.to_owned()),
            "102001234 [CQ:at,qq=102001234] 102001234",
        );

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("CQ 段内的数字不应触发替换");

        assert_eq!(
            api.sent()[0].content.as_deref(),
            // `[CQ:at,qq=102001234]` 参数逐字保留；未装配 idmap 时反查失败，码原文透传
            Some("2633000001 [CQ:at,qq=102001234] 2633000001")
        );
    }

    /// `do_not_replace_appid` 打开时正文逐字透传。
    #[tokio::test]
    async fn do_not_replace_appid_keeps_text_verbatim() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context_with_settings(
            Arc::clone(&api),
            "  app_id: 102001234\n  do_not_replace_appid: true\n",
        );
        let context = context.with_bot_id("2633000001");
        let media = no_media();
        let plan = text_plan(
            SendTarget::Group(OPENID.to_owned()),
            "机器人 102001234 在线",
        );

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("开关打开时也应发送成功");

        assert_eq!(
            api.sent()[0].content.as_deref(),
            Some("机器人 102001234 在线")
        );
    }

    /// BotID 缺失或与 AppID 相同时是空操作（Go 用空串守卫，相等替换本就无变化）。
    #[tokio::test]
    async fn missing_or_equal_bot_id_keeps_app_id() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1").with_send("MSG-2"));
        let (_dir, context) = context_with_settings(Arc::clone(&api), "  app_id: 102001234\n");
        let media = no_media();

        for context in [context.clone(), context.with_bot_id("102001234")] {
            let plan = text_plan(
                SendTarget::Group(OPENID.to_owned()),
                "机器人 102001234 在线",
            );
            execute(&context, &plan, &deps(&media, None))
                .await
                .expect("BotID 缺失时也应发送成功");
        }

        let sent = api.sent();
        assert_eq!(sent.len(), 2);
        for body in &sent {
            assert_eq!(body.content.as_deref(), Some("机器人 102001234 在线"));
        }
    }

    #[tokio::test]
    async fn text_at_mention_resolution_failure_keeps_the_code() {
        // 未装配 idmap：数字虚拟号反查失败 → 文本保留原文（只 warn），不像 markdown 那样 1400
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        let text = "[CQ:at,qq=123456] 你好";
        let plan = text_plan(SendTarget::Group(OPENID.to_owned()), text);

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("文本路径反查失败不应报错");

        assert_eq!(
            api.sent()[0].content.as_deref(),
            Some(text),
            "反查失败必须保留 CQ 原文"
        );
    }

    #[tokio::test]
    async fn text_avatar_cq_is_not_treated_as_at() {
        // `[CQ:avatar]` 不是 at 段：文本路径同样原样放行，不误判成缺 qq
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        let text = "[CQ:avatar] 你好";
        let plan = text_plan(SendTarget::Group(OPENID.to_owned()), text);

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("非 at 的 CQ 码应原样放行");

        assert_eq!(
            api.sent()[0].content.as_deref(),
            Some(text),
            "非 at 段不得被改写"
        );
    }

    #[tokio::test]
    async fn text_at_mention_still_resolves_to_openid() {
        // 齐备的 at 段照旧走 idmap 反查 → 旧式 `<@openid>`
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let (_idmap_dir, idmap) = idmap_store();
        let (_, vuin) = idmap
            .retrieve_virtual_value(MEMBER_OPENID)
            .expect("登记身份失败");
        let context = context.with_idmap(Arc::clone(&idmap));
        let media = no_media();
        let plan = text_plan(
            SendTarget::Group(OPENID.to_owned()),
            &format!("[CQ:at,qq={vuin}] 你好"),
        );

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("文本 @用户 应发送成功");

        let expected = format!("<@{MEMBER_OPENID}> 你好");
        assert_eq!(
            api.sent()[0].content.as_deref(),
            Some(expected.as_str()),
            "数字虚拟号仍要反查成 <@openid>"
        );
    }

    #[tokio::test]
    async fn text_at_mention_accepts_whitespace_params() {
        // 键名/取值两侧空白要照样解析（与 Go `cqAtNumRe` 的 `\s*` 同口径）
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let (_idmap_dir, idmap) = idmap_store();
        let (_, vuin) = idmap
            .retrieve_virtual_value(MEMBER_OPENID)
            .expect("登记身份失败");
        let context = context.with_idmap(Arc::clone(&idmap));
        let media = no_media();
        let plan = text_plan(
            SendTarget::Group(OPENID.to_owned()),
            &format!("[CQ:at, qq = {vuin} ] 你好"),
        );

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("空白写法的 at 段也应发送成功");

        let expected = format!("<@{MEMBER_OPENID}> 你好");
        assert_eq!(
            api.sent()[0].content.as_deref(),
            Some(expected.as_str()),
            "空白写法不得把 at 段原样发出去"
        );
    }

    // ---- markdown @用户 / 文本+markdown 的 @全员 迁移 / 卡片冲突 ----

    /// 用 `content` 构造一份 markdown 计划（json 编码由 serde 负责，避免手写转义）。
    fn markdown_plan(content: &str) -> OutboundPlan {
        OutboundPlan {
            markdown: vec![
                MarkdownPayload::from_json(&serde_json::json!({ "content": content }).to_string())
                    .expect("markdown 载荷"),
            ],
            ..OutboundPlan::default()
        }
    }

    #[tokio::test]
    async fn markdown_at_mention_becomes_qqbot_at_user() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let (_idmap_dir, idmap) = idmap_store();
        let (_, vuin) = idmap
            .retrieve_virtual_value(MEMBER_OPENID)
            .expect("登记身份失败");
        let context = context.with_idmap(Arc::clone(&idmap));
        let media = no_media();
        let plan = request(
            SendTarget::Group(OPENID.to_owned()),
            markdown_plan(&format!(
                "[CQ:at,qq={vuin}] 你好 [文档](https://example.com/a)"
            )),
        );

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("markdown @用户 应发送成功");

        let sent = api.sent();
        assert_eq!(sent[0].msg_type, Some(msg_type::MARKDOWN));
        let content = sent[0]
            .markdown
            .as_ref()
            .and_then(|markdown| markdown.content.as_deref())
            .expect("markdown 必须有正文");
        assert_eq!(
            content,
            format!("<qqbot-at-user id=\"{MEMBER_OPENID}\" /> 你好 [文档](https://example.com/a)"),
            "数字虚拟号必须反查成官方 @ 标签，且不破坏 markdown 链接语法"
        );
        assert!(
            !content.contains("[CQ:at,"),
            "markdown 正文不得残留 CQ 码: {content}"
        );
    }

    #[tokio::test]
    async fn markdown_at_mention_accepts_whitespace_params() {
        // markdown 侧同一口径：键名/取值空白仍要渲染成 <qqbot-at-user>
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let (_idmap_dir, idmap) = idmap_store();
        let (_, vuin) = idmap
            .retrieve_virtual_value(MEMBER_OPENID)
            .expect("登记身份失败");
        let context = context.with_idmap(Arc::clone(&idmap));
        let media = no_media();
        let plan = request(
            SendTarget::Group(OPENID.to_owned()),
            markdown_plan(&format!("[CQ:at, qq = {vuin} ] 你好")),
        );

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("markdown 空白写法的 at 段应发送成功");

        let content = api.sent()[0]
            .markdown
            .as_ref()
            .and_then(|markdown| markdown.content.clone())
            .expect("markdown 必须有正文");
        assert_eq!(
            content,
            format!("<qqbot-at-user id=\"{MEMBER_OPENID}\" /> 你好"),
            "空白写法不得残留 CQ 码"
        );
    }

    #[tokio::test]
    async fn markdown_at_mention_resolution_failure_is_a_request_error() {
        // 未装配 idmap：数字虚拟号无法反查 → 1400，不静默留 CQ 码。
        // `[CQ: at,...]`（`[CQ:` 后跳空白形态）必须同样定位到：识别层漏判会让它既不 1400
        // 也不渲染，G2 回归（静默留 CQ 码当文本发出）正是本次补钉的场景
        for content in ["[CQ:at,qq=123456] 你好", "[CQ: at,qq=123456] 你好"] {
            let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
            let (_dir, context) = context(Arc::clone(&api));
            let media = no_media();
            let plan = request(SendTarget::Group(OPENID.to_owned()), markdown_plan(content));

            let err = execute(&context, &plan, &deps(&media, None))
                .await
                .expect_err("反查失败必须拒绝");
            assert!(matches!(err, Error::Protocol(_)), "应映射 1400: {err}");
            assert!(api.sent().is_empty(), "{content} 反查失败不应发送");
        }
    }

    #[tokio::test]
    async fn markdown_at_without_qq_is_a_request_error() {
        // markdown 正文缺 qq 一律 1400：裸 at 段（含 `at` 后带空白的写法）、不成对的参数、
        // 空的 qq 值
        for content in [
            "[CQ:at] 你好",
            "[CQ:at ] 你好",
            "[CQ:at,foo] 你好",
            "[CQ:at ,foo] 你好",
            "[CQ:at,qq=] 你好",
        ] {
            let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
            let (_dir, context) = context(Arc::clone(&api));
            let media = no_media();
            let plan = request(SendTarget::Group(OPENID.to_owned()), markdown_plan(content));

            let err = execute(&context, &plan, &deps(&media, None))
                .await
                .expect_err("markdown 缺 qq 必须拒绝");
            assert!(
                matches!(err, Error::Protocol(_)),
                "{content} 应映射 1400: {err}"
            );
            assert!(api.sent().is_empty(), "{content} 不应发送");
        }
    }

    #[tokio::test]
    async fn markdown_at_with_space_before_comma_still_resolves() {
        // `[CQ:at , qq = N]`：空白不改变渲染结果，markdown 里必须是官方 @ 标签
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let (_idmap_dir, idmap) = idmap_store();
        let (_, vuin) = idmap
            .retrieve_virtual_value(MEMBER_OPENID)
            .expect("登记身份失败");
        let context = context.with_idmap(Arc::clone(&idmap));
        let media = no_media();
        let plan = request(
            SendTarget::Group(OPENID.to_owned()),
            markdown_plan(&format!("[CQ:at , qq = {vuin} ] 你好")),
        );

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("markdown 空白写法的 at 段应发送成功");

        let content = api.sent()[0]
            .markdown
            .as_ref()
            .and_then(|markdown| markdown.content.clone())
            .expect("markdown 必须有正文");
        assert_eq!(
            content,
            format!("<qqbot-at-user id=\"{MEMBER_OPENID}\" /> 你好"),
            "空白写法不得残留 CQ 码"
        );
    }

    #[tokio::test]
    async fn markdown_avatar_cq_is_not_treated_as_at() {
        // 形似的码不是 at 段：不能被误判成缺 qq 而拒掉。`[CQ: atx]` / `[CQ: avatar]` 等
        // 段名前带空白/紧跟空白段名的形态同样不得命中（空白放宽不能扩大误判面）
        for text in ["[CQ:avatar] 你好", "[CQ: atx] 你好", "[CQ: atom,qq=1] 你好"] {
            let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
            let (_dir, context) = context(Arc::clone(&api));
            let media = no_media();
            let plan = request(SendTarget::Group(OPENID.to_owned()), markdown_plan(text));

            execute(&context, &plan, &deps(&media, None))
                .await
                .expect("非 at 的 CQ 码应原样放行");

            let sent = api.sent();
            let content = sent[0]
                .markdown
                .as_ref()
                .and_then(|markdown| markdown.content.as_deref())
                .expect("markdown 必须有正文");
            assert_eq!(content, text, "非 at 段不得被改写");
        }
    }

    #[tokio::test]
    async fn text_at_everyone_migrates_into_markdown_content() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let (_idmap_dir, idmap) = idmap_store();
        let context = context.with_idmap(Arc::clone(&idmap));
        let media = no_media();
        let mut plan = text_plan(SendTarget::Group(OPENID.to_owned()), "[CQ:at,qq=all] 开会");
        plan.message.plan = markdown_plan("# 今天的议题");

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("文本 + markdown 应发送成功");

        let sent = api.sent();
        assert_eq!(sent[0].msg_type, Some(msg_type::MARKDOWN));
        assert_eq!(sent[0].content, None, "markdown 消息的 content 必须为空");
        assert_eq!(
            sent[0]
                .markdown
                .as_ref()
                .and_then(|markdown| markdown.content.as_deref()),
            Some("# 今天的议题 <qqbot-at-everyone />"),
            "文本里的 @全员 必须迁移进 markdown 正文"
        );
        assert!(
            idmap.read_config(SECTION, OPENID).is_ok(),
            "迁移后的标签同样要占当天额度"
        );
    }

    #[test]
    fn migrate_at_everyone_matches_go_semantics() {
        assert_eq!(
            super::migrate_at_everyone("# 议题", "<qqbot-at-everyone /> 开会"),
            "# 议题 <qqbot-at-everyone />"
        );
        assert_eq!(
            super::migrate_at_everyone("# 议题\n", "<qqbot-at-everyone />"),
            "# 议题\n <qqbot-at-everyone />",
            "Go 只做首尾 TrimSpace，正文内部换行保留"
        );
        assert_eq!(
            super::migrate_at_everyone("# 已有 <qqbot-at-everyone />", "<qqbot-at-everyone />"),
            "# 已有 <qqbot-at-everyone />",
            "正文已有标签不得重复追加"
        );
        assert_eq!(
            super::migrate_at_everyone("", "<qqbot-at-everyone />"),
            "",
            "模板 markdown 没有正文，塞标签会产出非法载荷"
        );
        assert_eq!(super::migrate_at_everyone("# 议题", "没有标签"), "# 议题");
    }

    #[tokio::test]
    async fn card_with_at_everyone_is_a_request_error() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        let mut plan = request(SendTarget::Group(OPENID.to_owned()), card_plan());
        plan.message.text = "[CQ:at,qq=all] 开会".to_owned();

        let err = execute(&context, &plan, &deps(&media, None))
            .await
            .expect_err("卡片 + @全员 必须拒绝");
        assert!(matches!(err, Error::Protocol(_)), "应映射 1400: {err}");
        assert!(api.sent().is_empty(), "冲突消息不应发送");
    }

    // ---- R3b：召回 / SSE / 合并转发的执行形状 ----

    #[tokio::test]
    async fn wakeup_plan_forces_is_wakeup_and_drops_reply() {
        let api = Arc::new(FakeQqApi::default().with_send("WAKE-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        let mut plan = request(
            SendTarget::Private(OPENID.to_owned()),
            OutboundPlan {
                // 召回带回复目标：Go 强制 MsgID=""，reply 连反查都不做（互斥语义）
                reply_msg_id: Some(Ob11Id::Num(42)),
                ..OutboundPlan::default()
            },
        );
        plan.wakeup = true;
        plan.message.text = "召回正文".to_owned();

        let id = execute(&context, &plan, &deps(&media, None))
            .await
            .expect("召回发送应成功");
        assert_eq!(id, "WAKE-1");

        let sent = api.sent();
        assert_eq!(sent.len(), 1);
        assert!(sent[0].is_wakeup, "应置 is_wakeup");
        assert_eq!(sent[0].msg_id, None, "召回不带 msg_id（Go 的互斥语义）");
        assert_eq!(sent[0].content.as_deref(), Some("召回正文"));
    }

    // ---- R4a：wakeup 终局 notice 回推（对齐 Go sendWakeupNotice）----

    /// 把总线收到的第一条事件序列化成 JSON 供逐键断言。
    async fn first_event_json(
        events: &mut tokio::sync::broadcast::Receiver<Arc<crate::event_hub::PublishedEvent>>,
    ) -> Value {
        let event = events.recv().await.expect("终局应收到一条 notice");
        // 断言发布时序列化的字节（WS/post_url 实际发出的那份），不是再序列化一遍。
        serde_json::from_str(event.json()).expect("notice 应可序列化")
    }

    /// wakeup 成功：JSON 与 Go `WakeupResponseNotice` 逐键一致，且只推一次。
    #[tokio::test]
    async fn wakeup_success_pushes_c2c_wakeup_resp_notice_once() {
        let api = Arc::new(FakeQqApi::default().with_send("WAKE-OK"));
        let (_dir, context) = context_with_settings(Arc::clone(&api), "  app_id: 102001234\n");
        let media = no_media();
        let hub = EventHub::default();
        let mut events = hub.subscribe();
        let mut plan = text_plan(SendTarget::Private(OPENID.to_owned()), "召回正文");
        plan.wakeup = true;

        execute(&context, &plan, &deps_with_hub(&media, None, Some(&hub)))
            .await
            .expect("召回发送应成功");

        let value = first_event_json(&mut events).await;
        assert_eq!(value["post_type"], "notice");
        assert_eq!(value["notice_type"], "c2c_wakeup_resp");
        assert_eq!(
            value["user_id"],
            serde_json::json!(0),
            "恒数字 0（Go int64）"
        );
        assert_eq!(value["real_user_id"], OPENID, "32 位 OpenID 字符串");
        assert_eq!(value["status"], "success");
        assert_eq!(value["message_id"], "WAKE-OK", "成功带真实上游 ID 字符串");
        assert!(
            value["time"].as_i64().is_some_and(|secs| secs > 0),
            "time 为 unix 秒: {value}"
        );
        assert_eq!(value["self_id"], serde_json::json!(102_001_234_u64));
        assert!(
            value.get("error_msg").is_none(),
            "成功不应带 error_msg（Go omitempty）"
        );
        assert!(events.try_recv().is_err(), "终局只推一次");
    }

    /// 成功但上游 `message_id` 为空串：整键消失（Go `omitempty`），status 仍 success。
    #[tokio::test]
    async fn wakeup_success_with_empty_message_id_omits_key() {
        let api = Arc::new(FakeQqApi::default().with_send(""));
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        let hub = EventHub::default();
        let mut events = hub.subscribe();
        let mut plan = text_plan(SendTarget::Private(OPENID.to_owned()), "召回正文");
        plan.wakeup = true;

        execute(&context, &plan, &deps_with_hub(&media, None, Some(&hub)))
            .await
            .expect("空 message_id 的成功仍应返回成功");

        let value = first_event_json(&mut events).await;
        assert_eq!(value["status"], "success");
        assert!(
            value.get("message_id").is_none(),
            "空 ID 应整键消失（Go omitempty）: {value}"
        );
        assert!(events.try_recv().is_err(), "终局只推一次");
    }

    /// wakeup 上游发送失败：推 failed，带 error_msg、不带 message_id。
    #[tokio::test]
    async fn wakeup_send_failure_pushes_failed_notice() {
        let api =
            Arc::new(FakeQqApi::default().with_send_error(Error::Network("上游挂了".to_owned())));
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        let hub = EventHub::default();
        let mut events = hub.subscribe();
        let mut plan = text_plan(SendTarget::Private(OPENID.to_owned()), "召回正文");
        plan.wakeup = true;

        execute(&context, &plan, &deps_with_hub(&media, None, Some(&hub)))
            .await
            .expect_err("发送失败应报错");

        let value = first_event_json(&mut events).await;
        assert_eq!(value["post_type"], "notice");
        assert_eq!(value["notice_type"], "c2c_wakeup_resp");
        assert_eq!(value["status"], "failed");
        assert!(
            value["error_msg"]
                .as_str()
                .is_some_and(|msg| msg.contains("上游挂了")),
            "error_msg 为错误文本（Go err.Error()）: {value}"
        );
        assert!(
            value.get("message_id").is_none(),
            "失败不应带 message_id（Go omitempty）"
        );
        assert!(events.try_recv().is_err(), "终局只推一次");
    }

    /// wakeup 媒体上传失败（Go :278 出口）：同样推 failed 且只推一次。
    #[tokio::test]
    async fn wakeup_media_upload_failure_pushes_failed_notice() {
        let api = Arc::new(FakeQqApi::with_upload_error(Error::Upstream {
            code: 40034001,
            message: "file too large".to_owned(),
        }));
        let (_dir, context) = context(Arc::clone(&api));
        let media = MediaRuntime::with_transport(Arc::new(FakeTransport::new(b"PNG-BYTES")));
        let hub = EventHub::default();
        let mut events = hub.subscribe();
        let mut plan = request(
            SendTarget::Private(OPENID.to_owned()),
            image_plan(MediaRef::Url("https://example.com/a.png".to_owned())),
        );
        plan.wakeup = true;

        execute(&context, &plan, &deps_with_hub(&media, None, Some(&hub)))
            .await
            .expect_err("上传失败应报错");

        let value = first_event_json(&mut events).await;
        assert_eq!(value["notice_type"], "c2c_wakeup_resp");
        assert_eq!(value["status"], "failed");
        assert!(
            value["error_msg"]
                .as_str()
                .is_some_and(|msg| msg.contains("file too large")),
            "error_msg 应带上游文案: {value}"
        );
        assert_eq!(value["real_user_id"], OPENID);
        assert!(value.get("message_id").is_none());
        assert!(events.try_recv().is_err(), "终局只推一次");
    }

    /// 非 wakeup 发送零影响（回归面）：挂了 hub 也不推。
    #[tokio::test]
    async fn non_wakeup_send_publishes_no_notice() {
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        let hub = EventHub::default();
        let mut events = hub.subscribe();
        let plan = text_plan(SendTarget::Group(OPENID.to_owned()), "普通正文");

        execute(&context, &plan, &deps_with_hub(&media, None, Some(&hub)))
            .await
            .expect("普通发送应成功");

        assert!(events.try_recv().is_err(), "非 wakeup 不应有任何推送");
    }

    /// 收件人还原失败的早退不推（Go 该路径早退也不推，口径一致）。
    #[tokio::test]
    async fn wakeup_target_resolution_failure_pushes_no_notice() {
        let api = Arc::new(FakeQqApi::default());
        let (_dir, context) = context(api);
        let media = no_media();
        let hub = EventHub::default();
        let mut events = hub.subscribe();
        // 数字虚拟号 + 未装配 idmap：还原 OpenID 必失败，终局尚未发生
        let mut plan = text_plan(SendTarget::Private("12345".to_owned()), "召回正文");
        plan.wakeup = true;

        execute(&context, &plan, &deps_with_hub(&media, None, Some(&hub)))
            .await
            .expect_err("虚拟号无法反查应报错");

        assert!(events.try_recv().is_err(), "还原失败早退不推 notice");
    }

    #[tokio::test]
    async fn sse_request_posts_stream_body_to_c2c() {
        let api = Arc::new(FakeQqApi::default().with_send("SSE-1"));
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        let plan = SseRequest {
            target: SendTarget::Private(OPENID.to_owned()),
            message_sse: MessageToCreate {
                msg_type: Some(msg_type::MARKDOWN),
                markdown: Some(Markdown::content("流式正文")),
                msg_seq: Some(10),
                stream: Some(StreamSse {
                    state: 1,
                    index: 0,
                    id: None,
                }),
                ..MessageToCreate::default()
            },
        };

        let id = execute_sse(&context, &plan, &deps(&media, None))
            .await
            .expect("SSE 应直发 C2C 端点");
        assert_eq!(id, "SSE-1");

        let sent = api.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].msg_type, Some(msg_type::MARKDOWN));
        assert_eq!(
            sent[0]
                .markdown
                .as_ref()
                .and_then(|markdown| markdown.content.as_deref()),
            Some("流式正文"),
            "正文原样进 markdown.content（Go 不解析 CQ）"
        );
        let stream = sent[0].stream.as_ref().expect("应带 stream");
        assert_eq!(stream.state, 1);
        assert_eq!(
            sent[0].msg_seq,
            Some(10),
            "msg_seq = index + 10 的防去重原样上送"
        );
    }

    // ---- 被动回复身份：懒消息池 / "2000" / msg_seq（Go send_group_msg.go:207-331） ----

    /// 带 echo 的上下文：`app_id` 显式给出，echo 键要用它拼。
    fn echo_context(api: Arc<FakeQqApi>, settings: &str) -> (TempDir, ActionContext) {
        context_with_settings(api, &format!("  app_id: 10001\n{settings}"))
    }

    #[tokio::test]
    async fn lazy_pool_hit_becomes_the_passive_msg_id() {
        let api = Arc::new(FakeQqApi::default().with_send("OUT-1"));
        let (_dir, context) = echo_context(Arc::clone(&api), "  lazy_message_id: true\n");
        let echo = EchoStore::new();
        echo.add_lazy_message_id_v2(OPENID, MEMBER_OPENID, "POOL-MSG", SystemTime::now());
        let media = no_media();
        let mut plan = text_plan(SendTarget::Group(OPENID.to_owned()), "正文");
        plan.echo_context.user_id = Some(MEMBER_OPENID.to_owned());

        execute(&context, &plan, &deps(&media, Some(&echo)))
            .await
            .expect("懒池命中应发送成功");

        let sent = api.sent();
        assert_eq!(
            sent[0].msg_id.as_deref(),
            Some("POOL-MSG"),
            "群 × 用户懒池的命中值就是 msg_id"
        );
        assert_eq!(sent[0].event_id, None, "msg_id 与 event_id 二选一");
        assert_eq!(sent[0].msg_seq, Some(1), "首条 msg_seq 从 1 起");
    }

    #[tokio::test]
    async fn group_without_user_context_drops_the_cached_msg_id() {
        let api = Arc::new(FakeQqApi::default().with_send("OUT-1"));
        let (_dir, context) = echo_context(Arc::clone(&api), "  lazy_message_id: true\n");
        let echo = EchoStore::new();
        echo.add_lazy_message_id(OPENID, "POOL-MSG", SystemTime::now());
        let media = no_media();
        // 客户端没传 user_id：Go 认为缓存的 msg_id 已过期，清空后按主动推送发
        let plan = text_plan(SendTarget::Group(OPENID.to_owned()), "主动");

        execute(&context, &plan, &deps(&media, Some(&echo)))
            .await
            .expect("应发送成功");

        assert_eq!(api.sent()[0].msg_id, None, "无用户上下文时不带缓存 msg_id");
    }

    #[tokio::test]
    async fn empty_lazy_pool_backfills_event_id_in_both_string_modes() {
        for string_ob11 in [false, true] {
            let api = Arc::new(FakeQqApi::default().with_send("OUT-1"));
            let (_dir, context) = echo_context(
                Arc::clone(&api),
                &format!("  lazy_message_id: true\n  string_ob11: {string_ob11}\n"),
            );
            let (_idmap_dir, idmap) = idmap_store();
            let context = context.with_idmap(Arc::clone(&idmap));
            // 客户端拿虚拟群号发消息，真实群 OpenID 由 idmap 反查
            let virtual_group = idmap
                .retrieve_virtual_value(OPENID)
                .expect("分配虚拟群号")
                .1;
            let echo = EchoStore::new();
            if string_ob11 {
                // string_ob11 打开：直接按真实 OpenID 键查（Go 的 v2 分支）
                echo.add_event_id_v2("10001", OPENID, "EV-STRING");
            } else {
                // 关闭：先按虚拟号键查（Go 的 v1 分支）
                echo.add_event_id("10001", virtual_group, "EV-LEGACY");
            }
            let media = no_media();
            let mut plan = text_plan(SendTarget::Group(virtual_group.to_string()), "主动");
            plan.echo_context.user_id = Some(MEMBER_OPENID.to_owned());

            execute(&context, &plan, &deps(&media, Some(&echo)))
                .await
                .expect("应发送成功");

            let sent = api.sent();
            assert_eq!(sent[0].msg_id, None, "「2000」分支必须清空 msg_id");
            let expected = if string_ob11 { "EV-STRING" } else { "EV-LEGACY" };
            assert_eq!(
                sent[0].event_id.as_deref(),
                Some(expected),
                "string_ob11={string_ob11} 时回填对应键的 event_id"
            );
        }
    }

    #[tokio::test]
    async fn private_empty_lazy_pool_backfills_event_id_by_raw_openid() {
        let api = Arc::new(FakeQqApi::default().with_send("OUT-1"));
        let (_dir, context) = echo_context(Arc::clone(&api), "  lazy_message_id: true\n");
        let (_idmap_dir, idmap) = idmap_store();
        let context = context.with_idmap(Arc::clone(&idmap));
        let virtual_user = idmap.retrieve_virtual_value(OPENID).expect("分配虚拟号").1;
        let echo = EchoStore::new();
        echo.add_event_id_v2("10001", OPENID, "EV-C2C");
        let media = no_media();
        let plan = text_plan(SendTarget::Private(virtual_user.to_string()), "主动");

        execute(&context, &plan, &deps(&media, Some(&echo)))
            .await
            .expect("应发送成功");

        let sent = api.sent();
        assert_eq!(sent[0].msg_id, None, "私聊「2000」也清空 msg_id");
        assert_eq!(
            sent[0].event_id.as_deref(),
            Some("EV-C2C"),
            "私聊按真实 OpenID 键回填 event_id"
        );
    }

    #[tokio::test]
    async fn request_echo_key_is_looked_up_for_the_passive_msg_id() {
        let api = Arc::new(FakeQqApi::default().with_send("OUT-1"));
        let (_dir, context) = echo_context(Arc::clone(&api), "");
        let echo = EchoStore::new();
        echo.add_msg_id_v3("10001", "ECHO-KEY", "MSG-FROM-ECHO");
        let media = no_media();
        let mut plan = text_plan(SendTarget::Private(OPENID.to_owned()), "回复");
        // Go 拿请求 `echo` 原样当键查表（`GetMsgIDByKey(echoStr)`，不再加前缀）
        plan.echo_context.echo_key = Some("10001_ECHO-KEY".to_owned());

        execute(&context, &plan, &deps(&media, Some(&echo)))
            .await
            .expect("应发送成功");

        assert_eq!(
            api.sent()[0].msg_id.as_deref(),
            Some("MSG-FROM-ECHO"),
            "echo 字符串当键查到的就是 msg_id"
        );
    }

    #[tokio::test]
    async fn active_marker_clears_msg_id_and_event_id() {
        let api = Arc::new(FakeQqApi::default().with_send("OUT-1"));
        let (_dir, context) = echo_context(Arc::clone(&api), "");
        let echo = EchoStore::new();
        echo.add_msg_id_v3("10001", "ECHO-KEY", "MSG-FROM-ECHO");
        echo.add_event_id_v2("10001", OPENID, "EV-1");
        let media = no_media();
        let mut plan = text_plan(SendTarget::Private(OPENID.to_owned()), "[CQ:active] 强制主动");
        plan.echo_context.echo_key = Some("10001_ECHO-KEY".to_owned());
        plan.message.plan.active = Some(ActiveKind {
            active: true,
            ..ActiveKind::default()
        });

        execute(&context, &plan, &deps(&media, Some(&echo)))
            .await
            .expect("应发送成功");

        let sent = api.sent();
        assert_eq!(sent[0].msg_id, None, "[CQ:active] 必须清 msg_id");
        assert_eq!(sent[0].event_id, None, "[CQ:active] 必须清 event_id");
    }

    #[tokio::test]
    async fn same_chain_replies_increment_msg_seq_on_one_msg_id() {
        let api = Arc::new(
            FakeQqApi::default()
                .with_send("OUT-1")
                .with_send("OUT-2"),
        );
        let (_dir, context) = echo_context(Arc::clone(&api), "");
        let echo = EchoStore::new();
        echo.add_msg_id_v3("10001", "CHAIN", "SAME-MSG");
        let media = no_media();
        let mut plan = text_plan(SendTarget::Private(OPENID.to_owned()), "第一段");
        plan.echo_context.echo_key = Some("10001_CHAIN".to_owned());

        for _ in 0..2 {
            execute(&context, &plan, &deps(&media, Some(&echo)))
                .await
                .expect("应发送成功");
        }

        let sent = api.sent();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0].msg_id.as_deref(), Some("SAME-MSG"));
        assert_eq!(sent[1].msg_id.as_deref(), Some("SAME-MSG"));
        assert_eq!(sent[0].msg_seq, Some(1), "同一 msg_id 上 seq 递增");
        assert_eq!(
            sent[1].msg_seq,
            Some(2),
            "同链第二段必须换 seq，否则撞 40054005（Issue #19）"
        );
    }

    #[tokio::test]
    async fn media_extras_get_their_own_msg_seq() {
        let api = Arc::new(
            FakeQqApi::default()
                .with_send("MSG-MD")
                .with_send("MSG-IMG"),
        );
        let (_dir, context) = echo_context(Arc::clone(&api), "");
        let echo = EchoStore::new();
        echo.add_msg_id_v3("10001", "CHAIN", "SAME-MSG");
        let media = no_media();
        let mut plan = group_request(OutboundPlan {
            images: vec![MediaRef::Unknown("FILE-1".to_owned())],
            ..markdown_plan("# 标题")
        });
        plan.echo_context.echo_key = Some("10001_CHAIN".to_owned());
        plan.echo_context.user_id = Some(MEMBER_OPENID.to_owned());

        execute(&context, &plan, &deps(&media, Some(&echo)))
            .await
            .expect("markdown + 媒体应发送成功");

        let sent = api.sent();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0].msg_id.as_deref(), Some("SAME-MSG"));
        assert_eq!(
            sent[1].msg_id.as_deref(),
            Some("SAME-MSG"),
            "媒体条与正文条共用同一被动 msg_id"
        );
        assert_eq!(sent[0].msg_seq, Some(1));
        assert_eq!(sent[1].msg_seq, Some(2), "媒体条各自再取一个 seq");
    }

    #[tokio::test]
    async fn wakeup_keeps_msg_id_and_event_id_off_even_with_echo_pooled() {
        let api = Arc::new(FakeQqApi::default().with_send("OUT-1"));
        let (_dir, context) = echo_context(Arc::clone(&api), "  lazy_message_id: true\n");
        let echo = EchoStore::new();
        echo.add_lazy_message_id(OPENID, "POOL-MSG", SystemTime::now());
        echo.add_event_id_v2("10001", OPENID, "EV-1");
        let media = no_media();
        let mut plan = text_plan(SendTarget::Private(OPENID.to_owned()), "召回");
        plan.wakeup = true;

        execute(&context, &plan, &deps(&media, Some(&echo)))
            .await
            .expect("召回应发送成功");

        let sent = api.sent();
        assert!(sent[0].is_wakeup, "召回标记必须落体");
        assert_eq!(sent[0].msg_id, None, "召回与 msg_id 互斥");
        assert_eq!(sent[0].event_id, None, "召回与 event_id 互斥");
    }

    #[tokio::test]
    async fn without_echo_nothing_is_stamped() {
        let api = Arc::new(FakeQqApi::default().with_send("OUT-1"));
        let (_dir, context) = echo_context(Arc::clone(&api), "  lazy_message_id: true\n");
        let media = no_media();
        let mut plan = text_plan(SendTarget::Group(OPENID.to_owned()), "正文");
        plan.echo_context.user_id = Some(MEMBER_OPENID.to_owned());
        plan.echo_context.echo_key = Some("ECHO-KEY".to_owned());

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("未装配 echo 也应正常发送");

        let sent = api.sent();
        assert_eq!(sent[0].msg_id, None);
        assert_eq!(sent[0].event_id, None);
        assert_eq!(sent[0].msg_seq, None, "未装配 echo 时行为零变化");
    }

    // ---- markdown 正文图片 ----

    /// 最小 PNG 文件头：`sniff_image` 只看魔数，`validate_image_upload` 只看体积 / 扩展名。
    const PNG_HEADER: &[u8] = b"\x89PNG\r\n\x1a\n";

    fn write_png(dir: &TempDir, name: &str) -> String {
        let path = dir.path().join(name);
        std::fs::write(&path, PNG_HEADER).expect("写测试图片失败");
        path.to_string_lossy().into_owned()
    }

    #[tokio::test]
    async fn markdown_local_image_becomes_media_reference_and_keeps_size_syntax() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = write_png(&dir, "本地 图.png");
        let api = Arc::new(
            FakeQqApi::default()
                .with_send("MSG-1")
                .with_media("qqbot-media://image/v1/token"),
        );
        let (_cfg, context) = context_with_settings(Arc::clone(&api), "  oss_type: 7\n");
        let media = no_media();
        let plan = group_request(markdown_plan(&format!("![本地 #480px #640px]({path})")));

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("本地图片应替换成素材引用");

        assert_eq!(
            api.sent()[0]
                .markdown
                .as_ref()
                .and_then(|md| md.content.as_deref()),
            Some("![本地 #480px #640px](qqbot-media://image/v1/token)"),
            "只换 URL，alt 与尺寸语法原样保留"
        );
    }

    #[tokio::test]
    async fn markdown_plain_link_and_html_img_are_rewritten() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = write_png(&dir, "a.png");
        let api = Arc::new(
            FakeQqApi::default()
                .with_send("MSG-1")
                .with_media("qqbot-media://image/v1/one")
                .with_media("qqbot-media://image/v1/two"),
        );
        let (_cfg, context) = context_with_settings(Arc::clone(&api), "  oss_type: 7\n");
        let media = no_media();
        let plan = group_request(markdown_plan(&format!(
            "[点我]({path}) <img src=\"{path}\" alt=\"封面\">"
        )));

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("普通链接与 img 标签都要转换");

        let expected = "[点我](qqbot-media://image/v1/one) \
                        <img src=\"qqbot-media://image/v1/two\" alt=\"封面\">";
        assert_eq!(
            api.sent()[0]
                .markdown
                .as_ref()
                .and_then(|md| md.content.as_deref()),
            Some(expected),
            "`!?` 让普通链接也转换（Go 同口径），img 只动 src"
        );
    }

    #[tokio::test]
    async fn remote_inline_and_media_references_are_left_untouched() {
        let api = Arc::new(FakeQqApi::default());
        let (_cfg, context) = context_with_settings(api, "  oss_type: 7\n");
        for raw in [
            "![a](https://example.com/a.png)",
            "![a](http://example.com/a.png)",
            "![a](data:image/png;base64,AAAA)",
            "![a](qqbot-media://image/v1/token)",
        ] {
            assert_eq!(
                resolve_markdown_images(&context, &TEST_HOSTING, raw).await,
                raw
            );
        }
    }

    #[tokio::test]
    async fn unreadable_or_rejected_images_keep_the_original_reference() {
        let dir = tempfile::tempdir().expect("临时目录");
        let missing = dir.path().join("missing.png").to_string_lossy().into_owned();

        let api = Arc::new(FakeQqApi::default());
        let (_cfg, context) = context_with_settings(Arc::clone(&api), "  oss_type: 7\n");
        let raw = format!("![坏图]({missing})");
        assert_eq!(
            resolve_markdown_images(&context, &TEST_HOSTING, &raw).await,
            raw,
            "读失败保留原引用"
        );

        let path = write_png(&dir, "b.png");
        let api = Arc::new(FakeQqApi::default().with_media_error());
        let (_cfg, context) = context_with_settings(api, "  oss_type: 7\n");
        let raw = format!("![坏上传]({path})");
        assert_eq!(
            resolve_markdown_images(&context, &TEST_HOSTING, &raw).await,
            raw,
            "上传失败保留原引用"
        );
    }

    #[tokio::test]
    async fn every_image_in_one_message_is_replaced() {
        let dir = tempfile::tempdir().expect("临时目录");
        let first = write_png(&dir, "1.png");
        let second = write_png(&dir, "2.png");
        let api = Arc::new(
            FakeQqApi::default()
                .with_send("MSG-1")
                .with_media("qqbot-media://image/v1/a")
                .with_media("qqbot-media://image/v1/b"),
        );
        let (_cfg, context) = context_with_settings(Arc::clone(&api), "  oss_type: 7\n");
        let media = no_media();
        let plan = group_request(markdown_plan(&format!(
            "![一]({first}) 和 ![二]({second})"
        )));

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect("多图应逐个替换");

        assert_eq!(
            api.sent()[0]
                .markdown
                .as_ref()
                .and_then(|md| md.content.as_deref()),
            Some("![一](qqbot-media://image/v1/a) 和 ![二](qqbot-media://image/v1/b)")
        );
    }

    #[tokio::test]
    async fn local_image_goes_through_the_configured_hosting_backend() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = write_png(&dir, "a.png");
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_cfg, context) = context_with_settings(Arc::clone(&api), BILIBILI_SETTINGS);
        let media = no_media();
        let transport = Arc::new(FakeHosting::ok());
        let hosting = HostingRuntime::with_transport(transport.clone());
        let plan = group_request(markdown_plan(&format!("![本地]({path})")));

        execute(
            &context,
            &plan,
            &deps_with_hosting(&media, None, None, &hosting),
        )
        .await
        .expect("图床上传成功后应替换成 CDN URL");

        assert_eq!(
            api.sent()[0]
                .markdown
                .as_ref()
                .and_then(|md| md.content.as_deref()),
            Some("![本地](https://cdn.example/1.png)"),
            "oss_type=5 的本地图片换成图床返回的 URL"
        );
        let requests = transport.requests();
        assert_eq!(requests.len(), 1, "一张图一次上传");
        assert_eq!(requests[0].method, UploadMethod::Post);
        assert!(requests[0].body.is_some(), "Bilibili 走 multipart POST");
    }

    #[tokio::test]
    async fn hosting_failure_keeps_the_original_reference() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = write_png(&dir, "a.png");
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_cfg, context) = context_with_settings(Arc::clone(&api), BILIBILI_SETTINGS);
        let media = no_media();
        let transport = Arc::new(FakeHosting::with_status(400));
        let hosting = HostingRuntime::with_transport(transport.clone());
        let raw = format!("![图]({path})");
        let plan = group_request(markdown_plan(&raw));

        execute(
            &context,
            &plan,
            &deps_with_hosting(&media, None, None, &hosting),
        )
        .await
        .expect("上传失败不能阻断发送");

        assert_eq!(
            api.sent()[0]
                .markdown
                .as_ref()
                .and_then(|md| md.content.as_deref()),
            Some(raw.as_str()),
            "上传失败保留原引用"
        );
        assert_eq!(transport.requests().len(), 1, "4xx 不重试");
    }

    #[tokio::test]
    async fn chatglm_keeps_media_store_chain_and_never_calls_hosting() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = write_png(&dir, "a.png");
        let api = Arc::new(
            FakeQqApi::default()
                .with_send("MSG-1")
                .with_media("qqbot-media://image/v1/token"),
        );
        let (_cfg, context) = context_with_settings(Arc::clone(&api), "  oss_type: 7\n");
        let media = no_media();
        let transport = Arc::new(FakeHosting::ok());
        let hosting = HostingRuntime::with_transport(transport.clone());
        let plan = group_request(markdown_plan(&format!("![本地]({path})")));

        execute(
            &context,
            &plan,
            &deps_with_hosting(&media, None, None, &hosting),
        )
        .await
        .expect("oss_type=7 仍走素材库");

        assert_eq!(
            api.sent()[0]
                .markdown
                .as_ref()
                .and_then(|md| md.content.as_deref()),
            Some("![本地](qqbot-media://image/v1/token)"),
            "oss_type=7 零回归：仍是素材库引用"
        );
        assert!(transport.requests().is_empty(), "oss_type=7 不走图床链");
    }

    #[tokio::test]
    async fn builtin_oss_backends_upload_and_rewrite_markdown() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = write_png(&dir, "a.png");
        for (oss_type, prefix) in [
            (1, "https://bucket-125.cos.ap-shanghai.myqcloud.com/"),
            (2, "https://bj.bcebos.com/10001/"),
            (3, "https://a-bucket.oss-cn-hangzhou.aliyuncs.com/"),
        ] {
            let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
            let (_cfg, context) = context_with_settings(Arc::clone(&api), &oss_settings(oss_type));
            let media = no_media();
            let transport = Arc::new(FakeHosting::ok());
            let hosting = HostingRuntime::with_transport(transport.clone());
            let plan = group_request(markdown_plan(&format!("![本地]({path})")));

            execute(
                &context,
                &plan,
                &deps_with_hosting(&media, None, None, &hosting),
            )
            .await
            .expect("内置云 OSS 上传成功后应替换成对象 URL");

            let sent = api.sent();
            let content = sent[0]
                .markdown
                .as_ref()
                .and_then(|md| md.content.as_deref())
                .expect("有 markdown 正文");
            assert!(
                content.starts_with(&format!("![本地]({prefix}")),
                "oss_type={oss_type}: {content}"
            );
            assert!(content.ends_with(".jpg)"), "oss_type={oss_type}: {content}");

            let requests = transport.requests();
            assert_eq!(requests.len(), 1, "oss_type={oss_type}：一张图一次上传、不重试");
            assert_eq!(requests[0].method, UploadMethod::Put);
            assert!(
                requests[0].body.is_some(),
                "oss_type={oss_type} 原始字节直传（不是 multipart）"
            );
        }
    }

    #[tokio::test]
    async fn builtin_oss_failure_keeps_the_original_reference() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = write_png(&dir, "a.png");
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_cfg, context) = context_with_settings(Arc::clone(&api), &oss_settings(1));
        let media = no_media();
        let transport = Arc::new(FakeHosting::with_status(403));
        let hosting = HostingRuntime::with_transport(transport.clone());
        let raw = format!("![图]({path})");
        let plan = group_request(markdown_plan(&raw));

        execute(
            &context,
            &plan,
            &deps_with_hosting(&media, None, None, &hosting),
        )
        .await
        .expect("内置 OSS 失败不能阻断发送");

        assert_eq!(
            api.sent()[0]
                .markdown
                .as_ref()
                .and_then(|md| md.content.as_deref()),
            Some(raw.as_str()),
            "上传失败保留原引用"
        );
        assert_eq!(transport.requests().len(), 1, "4xx 不重试");
    }

    #[tokio::test]
    async fn builtin_oss_audit_switch_uploads_then_audits() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = write_png(&dir, "a.png");
        let transport = Arc::new(FakeHosting::ok());
        let hosting = HostingRuntime::with_transport(transport.clone());
        let api = Arc::new(FakeQqApi::default());
        let settings = format!("{}  t_audit: true\n", oss_settings(1));
        let (_cfg, context) = context_with_settings(api, &settings);
        let raw = format!("![图]({path})");

        assert_eq!(
            resolve_markdown_images(&context, &hosting, &raw).await,
            raw,
            "审核响应不可用（这里回的不是 CI XML）同样保留原引用"
        );
        let requests = transport.requests();
        assert_eq!(requests.len(), 2, "t_audit 打开：先 PUT 再审核");
        assert_eq!(requests[0].method, UploadMethod::Put);
        assert_eq!(requests[1].method, UploadMethod::Get);
        assert!(
            requests[1]
                .url
                .ends_with("?ci-process=sensitive-content-recognition"),
            "{}",
            requests[1].url
        );
    }

    #[tokio::test]
    async fn local_upload_type_zero_keeps_local_paths() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = write_png(&dir, "a.png");
        let transport = Arc::new(FakeHosting::ok());
        let hosting = HostingRuntime::with_transport(transport.clone());
        let api = Arc::new(FakeQqApi::default());
        let (_cfg, context) = context_with_settings(api, "  oss_type: 0\n");
        let raw = format!("![图]({path})");

        assert_eq!(
            resolve_markdown_images(&context, &hosting, &raw).await,
            raw,
            "oss_type=0 依赖 Go 的 /uploadpic 端点，Rust 侧没有 → 保留原引用"
        );
        assert!(transport.requests().is_empty(), "0 不走图床也不走内置 OSS");
    }

    #[tokio::test]
    async fn missing_hosting_credentials_keep_the_original_reference() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = write_png(&dir, "a.png");
        let transport = Arc::new(FakeHosting::ok());
        let hosting = HostingRuntime::with_transport(transport.clone());
        let api = Arc::new(FakeQqApi::default());
        // oss_type=5 但 bilibili 段缺失：后端在发请求前就报 HostingConfig
        let (_cfg, context) = context_with_settings(api, "  oss_type: 5\n");
        let raw = format!("![图]({path})");

        assert_eq!(
            resolve_markdown_images(&context, &hosting, &raw).await,
            raw,
            "缺凭据保留原引用"
        );
        assert!(transport.requests().is_empty(), "缺凭据不发请求");
    }

    #[tokio::test]
    async fn signed_backend_without_config_is_dispatched() {
        // oss_type=9（星野，免配置）：签名请求发出去就证明走的是图床链而不是素材库
        let dir = tempfile::tempdir().expect("临时目录");
        let path = write_png(&dir, "a.png");
        let transport = Arc::new(FakeHosting::with_status(400));
        let hosting = HostingRuntime::with_transport(transport.clone());
        let api = Arc::new(FakeQqApi::default());
        let (_cfg, context) = context_with_settings(api, "  oss_type: 9\n");
        let raw = format!("![图]({path})");

        assert_eq!(
            resolve_markdown_images(&context, &hosting, &raw).await,
            raw,
            "签名失败保留原引用"
        );
        assert_eq!(transport.requests().len(), 1, "星野先取签名");
    }

    #[tokio::test]
    async fn every_markdown_image_goes_through_hosting() {
        let dir = tempfile::tempdir().expect("临时目录");
        let first = write_png(&dir, "1.png");
        let second = write_png(&dir, "2.png");
        let api = Arc::new(FakeQqApi::default().with_send("MSG-1"));
        let (_cfg, context) = context_with_settings(Arc::clone(&api), BILIBILI_SETTINGS);
        let media = no_media();
        let transport = Arc::new(FakeHosting::ok());
        let hosting = HostingRuntime::with_transport(transport.clone());
        let plan = group_request(markdown_plan(&format!("![一]({first}) 和 ![二]({second})")));

        execute(
            &context,
            &plan,
            &deps_with_hosting(&media, None, None, &hosting),
        )
        .await
        .expect("多图应逐个上传替换");

        assert_eq!(
            api.sent()[0]
                .markdown
                .as_ref()
                .and_then(|md| md.content.as_deref()),
            Some("![一](https://cdn.example/1.png) 和 ![二](https://cdn.example/2.png)")
        );
        assert_eq!(transport.requests().len(), 2);
    }

    #[tokio::test]
    async fn forward_loops_every_node_and_reports_failed_index() {
        let api = Arc::new(
            FakeQqApi::default()
                .with_send("F-1")
                .push_send(Err(Error::Upstream {
                    code: 40_054_005,
                    message: "msgseq 去重".to_owned(),
                }))
                .with_send("F-3"),
        );
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        let plan = ForwardRequest {
            // node_index 与计划下标刻意不一致（模拟坏 / 空 node 被跳过，评审 ①）：
            // 计划里是 node 0 / 1 / 4，node 3 规划层解析失败
            forward: vec![
                ForwardPlan {
                    node_index: 0,
                    request: text_plan(SendTarget::Group(OPENID.to_owned()), "第一条"),
                },
                ForwardPlan {
                    node_index: 1,
                    request: text_plan(SendTarget::Group(OPENID.to_owned()), "第二条"),
                },
                ForwardPlan {
                    node_index: 4,
                    request: text_plan(SendTarget::Group(OPENID.to_owned()), "第五条"),
                },
            ],
            parse_failed: vec![ForwardParseFailure {
                node_index: 3,
                error: "段解析失败".to_owned(),
            }],
        };

        let outcome = execute_forward(&context, &plan, &deps(&media, None)).await;

        assert_eq!(
            outcome.message_ids,
            vec!["F-1".to_owned(), "F-3".to_owned()],
            "已发送的不回滚（Go 逐条独立）"
        );
        // 失败清单报**原 node 序号**（发送失败 1 + 规划解析失败 3，按序合并）
        let indices: Vec<usize> = outcome.failed.iter().map(|item| item.index).collect();
        assert_eq!(indices, vec![1, 3]);
        assert!(
            outcome.failed[0].error.contains("msgseq 去重"),
            "实际错误: {}",
            outcome.failed[0].error
        );
        assert_eq!(outcome.failed[1].error, "段解析失败");
        let sent = api.sent();
        assert_eq!(sent.len(), 3, "三条都各打了一次上游");
        assert_eq!(sent[2].content.as_deref(), Some("第五条"), "顺序与计划一致");
    }

    // ---- SSM 补发：入队 / 触发 / 补发语义（Go send_group_msg.go:207-288 / 483-490 / 2662-2697） ----

    /// 往队列里塞一条积压：载荷用真实 [`MessageToCreate`] 序列化，与生产入队同构。
    fn push_ssm(echo: &EchoStore, group: &str, text: &str) {
        let body = MessageToCreate {
            content: Some(text.to_owned()),
            ..MessageToCreate::default()
        };
        echo.push_global_stack(MessageGroupPair {
            payload: Some(serde_json::to_string(&body).expect("消息体应可序列化")),
            ..MessageGroupPair::new(group)
        });
    }

    /// 队列里每条载荷的正文（断言顺序 / 存活情况用）。
    fn ssm_texts(echo: &EchoStore, count: usize) -> Vec<String> {
        echo.pop_global_stack_multi(count)
            .iter()
            .filter_map(|pair| pair.payload.as_deref())
            .filter_map(|payload| serde_json::from_str::<MessageToCreate>(payload).ok())
            .filter_map(|body| body.content)
            .collect()
    }

    /// 补发生效所需的配置：lazy 开启 + 队列条数（`Settings::default()` 的 `ato_p_count` 是 0）。
    const SSM_SETTINGS: &str = "  lazy_message_id: true\n  AMsgRetryAsPMsg_Count: 3\n";

    #[tokio::test]
    async fn group_22009_enqueues_the_failed_body() {
        let api = Arc::new(FakeQqApi::default().with_send_error(Error::Upstream {
            code: 22009,
            message: "频控".to_owned(),
        }));
        let (_dir, context) = context(Arc::clone(&api));
        let echo = EchoStore::new();
        let media = no_media();
        let plan = text_plan(SendTarget::Group(OPENID.to_owned()), "被频控的正文");

        let err = execute(&context, &plan, &deps(&media, Some(&echo)))
            .await
            .expect_err("22009 仍要如实回报失败");

        assert!(matches!(err, Error::Upstream { code: 22009, .. }));
        assert_eq!(echo.global_stack_len(), 1, "失败的群消息应入队");
        let pair = echo.pop_global_stack_multi(1).remove(0);
        assert_eq!(pair.group, OPENID);
        assert!(
            pair.correlation_id.starts_with("ssm-"),
            "关联标识: {}",
            pair.correlation_id
        );
        assert!(pair.enqueue_time.is_some());
        let body: MessageToCreate =
            serde_json::from_str(pair.payload.as_deref().expect("payload 应是消息体 JSON"))
                .expect("payload 应是 MessageToCreate");
        assert_eq!(body.content.as_deref(), Some("被频控的正文"));
    }

    #[tokio::test]
    async fn only_group_22009_enters_the_queue() {
        // 非 22009：不入队
        let api = Arc::new(FakeQqApi::default().with_send_error(Error::Upstream {
            code: 1500,
            message: "别的错".to_owned(),
        }));
        let (_dir, only_group) = context(Arc::clone(&api));
        let echo = EchoStore::new();
        let media = no_media();
        let plan = text_plan(SendTarget::Group(OPENID.to_owned()), "正文");
        execute(&only_group, &plan, &deps(&media, Some(&echo)))
            .await
            .expect_err("其他上游错误同样失败");
        assert_eq!(echo.global_stack_len(), 0, "只有 22009 入队");

        // 私聊 22009：不入队（Go `send_private_msg.go` 只记日志）
        let api = Arc::new(FakeQqApi::default().with_send_error(Error::Upstream {
            code: 22009,
            message: "频控".to_owned(),
        }));
        let (_dir, private_context) = context(Arc::clone(&api));
        let echo = EchoStore::new();
        let plan = text_plan(SendTarget::Private(MEMBER_OPENID.to_owned()), "正文");
        execute(&private_context, &plan, &deps(&media, Some(&echo)))
            .await
            .expect_err("私聊同样失败");
        assert_eq!(echo.global_stack_len(), 0, "私聊不入队");
    }

    #[tokio::test]
    async fn ssm_resent_only_for_the_current_group() {
        let api = Arc::new(
            FakeQqApi::default()
                .with_send("SSM-1")
                .with_send("SSM-2")
                .with_send("MAIN"),
        );
        let (_dir, context) = echo_context(Arc::clone(&api), SSM_SETTINGS);
        let echo = EchoStore::new();
        echo.add_lazy_message_id(OPENID, "POOL-MSG", SystemTime::now());
        push_ssm(&echo, OPENID, "积压一");
        push_ssm(&echo, "OTHER-GROUP-OPENID", "别的群");
        push_ssm(&echo, OPENID, "积压二");
        let media = no_media();
        let plan = text_plan(SendTarget::Group(OPENID.to_owned()), "正文");

        execute(&context, &plan, &deps(&media, Some(&echo)))
            .await
            .expect("应发送成功");

        let sent = api.sent();
        let texts: Vec<Option<&str>> = sent.iter().map(|body| body.content.as_deref()).collect();
        assert_eq!(
            texts,
            vec![Some("积压一"), Some("积压二"), Some("正文")],
            "先补发本群积压，再发本条"
        );
        assert_eq!(echo.global_stack_len(), 1, "异群那条留在队列里");
        assert_eq!(ssm_texts(&echo, 1), vec!["别的群"]);
    }

    #[tokio::test]
    async fn ssm_consumes_increasing_msg_seq() {
        let api = Arc::new(
            FakeQqApi::default()
                .with_send("SSM-1")
                .with_send("SSM-2")
                .with_send("MAIN"),
        );
        let (_dir, context) = echo_context(Arc::clone(&api), SSM_SETTINGS);
        let echo = EchoStore::new();
        echo.add_lazy_message_id_v2(OPENID, MEMBER_OPENID, "POOL-MSG", SystemTime::now());
        push_ssm(&echo, OPENID, "积压一");
        push_ssm(&echo, OPENID, "积压二");
        let media = no_media();
        let mut plan = text_plan(SendTarget::Group(OPENID.to_owned()), "正文");
        plan.echo_context.user_id = Some(MEMBER_OPENID.to_owned());

        execute(&context, &plan, &deps(&media, Some(&echo)))
            .await
            .expect("应发送成功");

        let sent = api.sent();
        let seqs: Vec<Option<u32>> = sent.iter().map(|body| body.msg_seq).collect();
        assert_eq!(
            seqs,
            vec![Some(1), Some(2), Some(3)],
            "补发与主条共用同一 msg_id，seq 逐条递增不重复"
        );
        let ids: Vec<Option<&str>> = sent.iter().map(|body| body.msg_id.as_deref()).collect();
        assert_eq!(
            ids,
            vec![Some("POOL-MSG"), Some("POOL-MSG"), Some("POOL-MSG")]
        );
    }

    #[tokio::test]
    async fn ssm_success_removes_and_22009_failure_requeues() {
        let api = Arc::new(
            FakeQqApi::default()
                .push_send(Err(Error::Upstream {
                    code: 22009,
                    message: "频控".to_owned(),
                }))
                .with_send("SSM-2")
                .with_send("MAIN"),
        );
        let (_dir, context) = echo_context(Arc::clone(&api), SSM_SETTINGS);
        let echo = EchoStore::new();
        echo.add_lazy_message_id(OPENID, "POOL-MSG", SystemTime::now());
        push_ssm(&echo, OPENID, "失败的");
        push_ssm(&echo, OPENID, "成功的");
        let media = no_media();
        let plan = text_plan(SendTarget::Group(OPENID.to_owned()), "正文");

        execute(&context, &plan, &deps(&media, Some(&echo)))
            .await
            .expect("主消息成功");

        assert_eq!(
            echo.global_stack_len(),
            2,
            "成功那条已移除；失败那条留在原地，另有一份重新入队的副本"
        );
        assert_eq!(ssm_texts(&echo, 2), vec!["失败的", "失败的"]);
        let pairs = echo.pop_global_stack_multi(2);
        assert_eq!(
            pairs[0].correlation_id, pairs[1].correlation_id,
            "重新入队保留原关联标识"
        );
    }

    #[tokio::test]
    async fn zero_retry_count_skips_ssm() {
        let api = Arc::new(FakeQqApi::default().with_send("MAIN"));
        let (_dir, context) = echo_context(
            Arc::clone(&api),
            "  lazy_message_id: true\n  AMsgRetryAsPMsg_Count: 0\n",
        );
        let echo = EchoStore::new();
        echo.add_lazy_message_id(OPENID, "POOL-MSG", SystemTime::now());
        push_ssm(&echo, OPENID, "积压一");
        push_ssm(&echo, OPENID, "积压二");
        let media = no_media();
        let plan = text_plan(SendTarget::Group(OPENID.to_owned()), "正文");

        execute(&context, &plan, &deps(&media, Some(&echo)))
            .await
            .expect("应发送成功");

        assert_eq!(api.sent().len(), 1, "只发本条");
        assert_eq!(echo.global_stack_len(), 2, "队列原样保留");
    }

    #[tokio::test]
    async fn ssm_skipped_when_lazy_off_or_active_push() {
        // lazy 关：队列有积压也不补发
        let api = Arc::new(FakeQqApi::default().with_send("MAIN"));
        let (_dir, context) = context(Arc::clone(&api));
        let echo = EchoStore::new();
        echo.add_lazy_message_id(OPENID, "POOL-MSG", SystemTime::now());
        push_ssm(&echo, OPENID, "积压");
        let media = no_media();
        let plan = text_plan(SendTarget::Group(OPENID.to_owned()), "正文");
        execute(&context, &plan, &deps(&media, Some(&echo)))
            .await
            .expect("应发送成功");
        assert_eq!(api.sent().len(), 1, "lazy 关时不补发");
        assert_eq!(echo.global_stack_len(), 1);

        // lazy 开但懒池没有可用值（等价 "2000" 主动推送）：同样不补发
        let api = Arc::new(FakeQqApi::default().with_send("MAIN"));
        let (_dir, context) = echo_context(Arc::clone(&api), SSM_SETTINGS);
        let echo = EchoStore::new();
        push_ssm(&echo, OPENID, "积压");
        execute(&context, &plan, &deps(&media, Some(&echo)))
            .await
            .expect("应发送成功");
        assert_eq!(api.sent().len(), 1, "懒池值为 \"2000\" 时不补发");
        assert_eq!(echo.global_stack_len(), 1);
    }

    #[tokio::test]
    async fn ssm_removes_every_resent_entry() {
        let api = Arc::new(
            FakeQqApi::default()
                .with_send("SSM-1")
                .with_send("SSM-2")
                .with_send("SSM-3")
                .with_send("MAIN"),
        );
        let (_dir, context) = echo_context(Arc::clone(&api), SSM_SETTINGS);
        let echo = EchoStore::new();
        echo.add_lazy_message_id(OPENID, "POOL-MSG", SystemTime::now());
        for text in ["一", "二", "三"] {
            push_ssm(&echo, OPENID, text);
        }
        let media = no_media();
        let plan = text_plan(SendTarget::Group(OPENID.to_owned()), "正文");

        execute(&context, &plan, &deps(&media, Some(&echo)))
            .await
            .expect("应发送成功");

        assert_eq!(api.sent().len(), 4, "三条积压 + 本条");
        assert_eq!(
            echo.global_stack_len(),
            0,
            "三条全成功：倒序删除把每一条都删掉（正序删会漏一条）"
        );
    }

    #[tokio::test]
    async fn without_echo_ssm_is_a_no_op() {
        let api = Arc::new(FakeQqApi::default().with_send_error(Error::Upstream {
            code: 22009,
            message: "频控".to_owned(),
        }));
        let (_dir, context) = context(Arc::clone(&api));
        let media = no_media();
        let plan = text_plan(SendTarget::Group(OPENID.to_owned()), "正文");

        execute(&context, &plan, &deps(&media, None))
            .await
            .expect_err("未装配 echo 时行为与接线前一致");

        assert_eq!(api.sent().len(), 1);
    }
}

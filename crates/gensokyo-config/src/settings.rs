//! `config.yml` 顶层映射。
//!
//! 字段类型与 yaml tag **逐字段对照 Go 真源 `structs/structs.go`**：`Settings` 顶层 184 个
//! tag（第 9~225 行）+ 嵌套类型 13 个 tag（`VisualPrefixConfig` / `ImageHostingCOS` /
//! `ImageHostingBilibili` / `ImageHostingQQChannel`，第 227~261 行），合计 197 个，M1a 起
//! 全量对齐。任何字段漏迁或 tag 写错都会被
//! `tests::fixture_covers_every_settings_field` 的键集比对抓住。
//!
//! 解码宽容度：未知字段忽略、缺失字段取零值（与 Go yaml.v3 一致，因此**不使用**
//! `deny_unknown_fields`）。
//!
//! **有意放宽的 Rust 侧超集**——Go 的 `int` / `int64` / `uint64` 字段额外接受带引号的
//! 数字字符串（`"5"`）与整值浮点（`5.0`），这些输入 Go yaml.v3 会直接报错。Go 侧是
//! `string` 的字段（如 `port`，`config.GetPortValue()` 直接返回该字符串）额外接受裸标量，
//! 并按标量文本归一（`port: 5.0` → `"5.0"`、`port: true` → `"true"`），与 yaml.v3 对
//! string 字段保留原文的行为一致。实现分别是 [`de_i64_lenient`] / [`de_u64_lenient`] /
//! [`de_string_lenient`]；切片字段（`[]int`、`[]string`、`[]int64`、`[]bool`）按元素严格
//! 解析，与 M0 的 `ws_address` 口径一致。

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

/// `config.yml` 顶层配置（Go `structs.Settings`，`structs/structs.go:9-225`）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct Settings {
    // ---- 反向 ws 设置 ----
    /// 反向 WS 地址列表（Go `[]string`，`ws_address`）。
    pub ws_address: Vec<String>,
    /// 反向 WS 令牌列表（Go `[]string`，`ws_token`）。
    pub ws_token: Vec<String>,
    /// 反向 WS 重连次数（Go `int`，`reconnect_times`）。
    #[serde(deserialize_with = "de_i64_lenient")]
    pub reconnect_times: i64,
    /// 心跳间隔秒数（Go `int`，`heart_beat_interval`）。
    #[serde(deserialize_with = "de_i64_lenient")]
    pub heart_beat_interval: i64,
    /// 启动时重连次数（Go `int`，`launch_reconnect_times`）。
    #[serde(deserialize_with = "de_i64_lenient")]
    pub launch_reconnect_times: i64,

    // ---- 基础配置 ----
    /// QQ 开放平台 AppID（Go `uint64`，`app_id`）；带引号写法同样接受。
    #[serde(deserialize_with = "de_u64_lenient")]
    pub app_id: u64,
    /// 机器人 QQ 号（Go `int64`，`uin`）。
    #[serde(deserialize_with = "de_i64_lenient")]
    pub uin: i64,
    /// QQ 开放平台令牌（Go `string`，`token`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub token: String,
    /// QQ 开放平台客户端密钥（Go `string`，`client_secret`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub client_secret: String,
    /// 分片总数（Go `int`，`shard_count`）。
    #[serde(deserialize_with = "de_i64_lenient")]
    pub shard_count: i64,
    /// 当前分片号（Go `int`，`shard_id`）。
    #[serde(deserialize_with = "de_i64_lenient")]
    pub shard_id: i64,
    /// 是否使用 QQ 号而非 AppID 作为机器标识（Go `bool`，`use_uin`）。
    pub use_uin: bool,
    /// 是否隔离各连接的消息 ID 空间（Go `bool`，`idmap_isolation`）。
    pub idmap_isolation: bool,
    /// idmap 旧数据兼容开关（Go `bool`，`idmap_legacy_compat`）。
    pub idmap_legacy_compat: bool,
    /// 分片数量（新字段，Go `int`，`shard_num`）。
    #[serde(deserialize_with = "de_i64_lenient")]
    pub shard_num: i64,

    // ---- 事件订阅类 ----
    /// 事件订阅 intent 列表（Go `[]string`，`text_intent`）。
    pub text_intent: Vec<String>,

    // ---- 转换类 ----
    /// 交互事件转消息（Go `bool`，`global_interaction_to_message`）。
    pub global_interaction_to_message: bool,
    /// 群消息被拒事件转消息（Go `bool`，tag 为 `global_group_msg_rre_to_message`，沿用 Go 的缩写）。
    #[serde(rename = "global_group_msg_rre_to_message")]
    pub global_group_msg_reject_recive_event_to_message: bool,
    /// 群消息被拒时回复的文本（Go `string`，`global_group_msg_reject_message`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub global_group_msg_reject_message: String,
    /// 群消息收到时回复的文本（Go `string`，`global_group_msg_receive_message`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub global_group_msg_receive_message: String,
    /// 单聊开关事件转消息（Go `bool`，tag 为 `global_c2c_msg_switch_to_message`）。
    #[serde(rename = "global_c2c_msg_switch_to_message")]
    pub global_c2c_msg_switch_event_to_message: bool,
    /// 单聊被拒时回复的文本（Go `string`，`global_c2c_msg_reject_message`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub global_c2c_msg_reject_message: String,
    /// 单聊收到时回复的文本（Go `string`，`global_c2c_msg_receive_message`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub global_c2c_msg_receive_message: String,
    /// 是否对 ID 做哈希（Go `bool`，`hash_id`）。
    pub hash_id: bool,
    /// 操作者 ID 类型（Go `string`，`op_userid_type`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub op_userid_type: String,
    /// 消息 ID 缓存存活秒数（Go `int`，`msgid_ttl_seconds`）。
    #[serde(deserialize_with = "de_i64_lenient")]
    pub msgid_ttl_seconds: i64,

    // ---- gensokyo 互联类 ----
    /// 数据目录（Go `string`，`server_dir`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub server_dir: String,
    /// 正向 WS / HTTP 服务端口（Go `string`，`port`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub port: String,
    /// 备用端口（Go `string`，`backup_port`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub backup_port: String,
    /// 是否启用 lotus 互联（Go `bool`，`lotus`）。
    pub lotus: bool,
    /// lotus 互联密码（Go `string`，`lotus_password`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub lotus_password: String,
    /// lotus 是否不带 idmap（Go `bool`，`lotus_without_idmaps`）。
    pub lotus_without_idmaps: bool,
    /// lotus 是否不上传图片（Go `bool`，`lotus_without_uploadpic`）。
    pub lotus_without_uploadpic: bool,
    /// 是否启用 lotus gRPC（Go `bool`，`lotus_grpc`）。
    pub lotus_grpc: bool,
    /// lotus gRPC 端口（Go `int`，`lotus_grpc_port`）。
    #[serde(deserialize_with = "de_i64_lenient")]
    pub lotus_grpc_port: i64,

    // ---- 增强配置 ----
    /// 主人 QQ 号列表（Go `[]string`，`master_id`）。
    pub master_id: Vec<String>,
    /// 语音采样率（Go `int`，tag 为 `record_sampleRate`，保留 Go 的驼峰）。
    #[serde(rename = "record_sampleRate", deserialize_with = "de_i64_lenient")]
    pub record_sample_rate: i64,
    /// 语音比特率（Go `int`，tag 为 `record_bitRate`，保留 Go 的驼峰）。
    #[serde(rename = "record_bitRate", deserialize_with = "de_i64_lenient")]
    pub record_bit_rate: i64,
    /// 群名片与昵称展示方式（Go `string`，tag 为 `card_nick`）。
    #[serde(rename = "card_nick", deserialize_with = "de_string_lenient")]
    pub card_and_nick: String,
    /// 是否自动绑定（Go `bool`，`auto_bind`）。
    pub auto_bind: bool,

    // ---- 发图相关 ----
    /// 图床后端枚举（Go `int`，`oss_type`，4~10；0 表示未启用）。
    #[serde(deserialize_with = "de_i64_lenient")]
    pub oss_type: i64,
    /// 图片大小上限（Go `int`，tag 为 `image_sizelimit`）。
    #[serde(rename = "image_sizelimit", deserialize_with = "de_i64_lenient")]
    pub image_size_limit: i64,
    /// 单条消息图片数量上限（Go `int`，`image_limit`）。
    #[serde(deserialize_with = "de_i64_lenient")]
    pub image_limit: i64,
    /// 是否转存 URL 图片（Go `bool`，`url_pic_transfer`）。
    pub url_pic_transfer: bool,
    /// 上传接口是否走 base64 v2（Go `bool`，tag 为 `uploadpicv2_b64`）。
    #[serde(rename = "uploadpicv2_b64")]
    pub uploadpic_v2_b64: bool,

    // ---- 图床相关（原 image_hosting 嵌套已展平至 Settings 顶层） ----
    /// 腾讯云 COS（Go `ImageHostingCOS`，`cos`）。
    #[serde(default, deserialize_with = "de_default_on_null")]
    pub cos: ImageHostingCOS,
    /// B 站图床（Go `ImageHostingBilibili`，`bilibili`）。
    #[serde(default, deserialize_with = "de_default_on_null")]
    pub bilibili: ImageHostingBilibili,
    /// QQ 频道图床（Go `ImageHostingQQChannel`，`qq_channel`）。
    #[serde(default, deserialize_with = "de_default_on_null")]
    pub qq_channel: ImageHostingQQChannel,
    /// ChatGLM 免费图床（Go `ImageHostingSimple`，`chatglm`）。
    #[serde(default, deserialize_with = "de_default_on_null")]
    pub chatglm: ImageHostingSimple,
    /// Ukaka 免费图床（Go `ImageHostingSimple`，`ukaka`）。
    #[serde(default, deserialize_with = "de_default_on_null")]
    pub ukaka: ImageHostingSimple,
    /// 星野免费图床（Go `ImageHostingSimple`，`xingye`）。
    #[serde(default, deserialize_with = "de_default_on_null")]
    pub xingye: ImageHostingSimple,
    /// Nature 免费图床（Go `ImageHostingSimple`，`nature`）。
    #[serde(default, deserialize_with = "de_default_on_null")]
    pub nature: ImageHostingSimple,

    // ---- 正向 ws 设置 ----
    /// 正向 WS 路径（Go `string`，`ws_server_path`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub ws_server_path: String,
    /// 是否作为 WS 服务端等待下游连接（Go `bool`，`enable_ws_server`）。
    pub enable_ws_server: bool,
    /// 正向 WS 令牌（Go `string`，`ws_server_token`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub ws_server_token: String,

    // ---- ssl 和链接转换类 ----
    /// 是否识别文件（Go `bool`，`identify_file`）。
    pub identify_file: bool,
    /// 需要识别的 AppID 列表（Go `[]int64`，`identify_appids`）。
    pub identify_appids: Vec<i64>,
    /// TLS 证书路径（Go `string`，`crt`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub crt: String,
    /// TLS 私钥路径（Go `string`，`key`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub key: String,
    /// 是否使用自签证书（Go `bool`，`use_self_crt`）。
    pub use_self_crt: bool,
    /// 反向 HTTP 回调路径（Go `string`，`webhook_path`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub webhook_path: String,
    /// 允许回调的来源 IP 段（Go `[]string`，`webhook_prefix_ip`）。
    pub webhook_prefix_ip: Vec<String>,
    /// 是否强制 SSL（Go `bool`，`force_ssl`）。
    pub force_ssl: bool,
    /// SSL 之后的内网转发端口（Go `string`，`http_port_after_ssl`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub http_port_after_ssl: String,

    // ---- 日志类 ----
    /// 开发者日志开关（Go `bool`，`developer_log`）。
    pub developer_log: bool,
    /// 日志级别（Go `int`，`log_level`）。
    #[serde(deserialize_with = "de_i64_lenient")]
    pub log_level: i64,
    /// 是否落盘保存日志（Go `bool`，`save_logs`）。
    pub save_logs: bool,
    /// 日志切分周期（分钟，Go `int`，`log_suffix_per_mins`）。
    #[serde(deserialize_with = "de_i64_lenient")]
    pub log_suffix_per_mins: i64,
    /// 是否启用彩色日志（Go `bool`，`log_color_enabled`）。
    pub log_color_enabled: bool,
    /// 日志保留天数（Go `int`，`log_max_age_days`）。
    #[serde(deserialize_with = "de_i64_lenient")]
    pub log_max_age_days: i64,
    /// 单个日志文件大小上限（MB，Go `int`，`log_max_size_mb`）。
    #[serde(deserialize_with = "de_i64_lenient")]
    pub log_max_size_mb: i64,
    /// 日志文件保留个数（Go `int`，`log_keep_files`）。
    #[serde(deserialize_with = "de_i64_lenient")]
    pub log_keep_files: i64,
    /// 慢事件告警阈值（毫秒，Go `int`，`log_slow_event_threshold_ms`）。
    #[serde(deserialize_with = "de_i64_lenient")]
    pub log_slow_event_threshold_ms: i64,

    // ---- webui 相关 ----
    /// 是否禁用 WebUI（Go `bool`，`disable_webui`）。
    pub disable_webui: bool,
    /// WebUI 用户名（Go `string`，tag 为 `server_user_name`）。
    #[serde(rename = "server_user_name", deserialize_with = "de_string_lenient")]
    pub username: String,
    /// WebUI 密码（Go `string`，tag 为 `server_user_password`）。
    #[serde(
        rename = "server_user_password",
        deserialize_with = "de_string_lenient"
    )]
    pub password: String,

    // ---- 指令魔法类 ----
    /// 是否移除消息前缀（Go `bool`，`remove_prefix`）。
    pub remove_prefix: bool,
    /// 是否移除 @bot（Go `bool`，`remove_at`）。
    pub remove_at: bool,
    /// 是否把 @其他人 转成 CQ 码（Go `bool`，`convert_other_at`）。
    pub convert_other_at: bool,
    /// 全量群消息中是否移除 @bot（Go `bool`，`remove_bot_at_group`）。
    pub remove_bot_at_group: bool,
    /// 群消息出站是否自动加 @bot（Go `bool`，`add_at_group`）。
    pub add_at_group: bool,
    /// 白名单前缀模式（Go `bool`，`white_prefix_mode`）。
    pub white_prefix_mode: bool,
    /// 白名单前缀模式（视觉前缀版，Go `bool`，`v_white_prefix_mode`）。
    pub v_white_prefix_mode: bool,
    /// 白名单前缀列表（Go `[]string`，`white_prefixs`）。
    pub white_prefixs: Vec<String>,
    /// 白名单旁路用户（Go `[]int64`，`white_bypass`）。
    pub white_bypass: Vec<i64>,
    /// 白名单逐个启停（Go `[]bool`，`white_enable`）。
    pub white_enable: Vec<bool>,
    /// 是否反转白名单旁路（Go `bool`，`white_bypass_reverse`）。
    pub white_bypass_reverse: bool,
    /// 非白名单回复文本（Go `string`，tag 为 `No_White_Response`）。
    #[serde(rename = "No_White_Response", deserialize_with = "de_string_lenient")]
    pub no_white_response: String,
    /// 黑名单前缀模式（Go `bool`，`black_prefix_mode`）。
    pub black_prefix_mode: bool,
    /// 黑名单前缀列表（Go `[]string`，`black_prefixs`）。
    pub black_prefixs: Vec<String>,
    /// 别名列表（Go `[]string`，`alias`）。
    pub alias: Vec<String>,
    /// 入口指令列表（Go `[]string`，`enters`）。
    pub enters: Vec<String>,
    /// 入口指令例外（Go `[]string`，`enters_except`）。
    pub enters_except: Vec<String>,
    /// 视觉前缀配置（Go `[]VisualPrefixConfig`，`visual_prefixs`）。
    pub visual_prefixs: Vec<VisualPrefixConfig>,
    /// 自动撤回指令（Go `[]string`，`auto_withdraw`）。
    pub auto_withdraw: Vec<String>,
    /// 自动撤回延迟（秒，Go `int`，`auto_withdraw_time`）。
    #[serde(deserialize_with = "de_i64_lenient")]
    pub auto_withdraw_time: i64,
    /// 视觉前缀旁路（Go `[]string`，`visual_prefixs_bypass`）。
    pub visual_prefixs_bypass: Vec<String>,

    // ---- 开发增强类 ----
    /// access token 目录（Go `string`，tag 为 `develop_access_token_dir`）。
    #[serde(
        rename = "develop_access_token_dir",
        deserialize_with = "de_string_lenient"
    )]
    pub devlop_ac_dir: String,
    /// 开发者机器人 ID（Go `string`，tag 为 `develop_bot_id`）。
    #[serde(rename = "develop_bot_id", deserialize_with = "de_string_lenient")]
    pub dev_bot_id: String,
    /// 沙箱模式（Go `bool`，`sandbox_mode`）。
    pub sandbox_mode: bool,
    /// 开发者消息 ID 模式（Go `bool`，tag 为 `dev_message_id`）。
    #[serde(rename = "dev_message_id")]
    pub dev_messge_id: bool,
    /// 是否发送错误信息（Go `bool`，`send_error`）。
    pub send_error: bool,
    /// 是否保存错误信息（Go `bool`，`save_error`）。
    pub save_error: bool,
    /// 停机公告（Go `string`，`downtime_message`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub downtime_message: String,
    /// 是否在内存中缓存消息 ID（Go `bool`，`memory_msgid`）。
    pub memory_msgid: bool,
    /// 线程回复消息（Go `bool`，`threads_ret_msg`）。
    pub threads_ret_msg: bool,
    /// 是否不回复空消息（Go `bool`，`no_ret_msg`）。
    pub no_ret_msg: bool,

    // ---- 增长营销类 ----
    /// 自我介绍文本列表（Go `[]string`，`self_introduce`）。
    pub self_introduce: Vec<String>,

    // ---- api 修改 ----
    /// 获取群列表的间隔延迟（Go `int`，tag 为 `get_g_list_delay`）。
    #[serde(rename = "get_g_list_delay", deserialize_with = "de_i64_lenient")]
    pub get_group_list_delay: i64,
    /// 合并转发消息条数上限（Go `int`，`forward_msg_limit`）。
    #[serde(deserialize_with = "de_i64_lenient")]
    pub forward_msg_limit: i64,
    /// 自定义机器人名称（Go `string`，`custom_bot_name`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub custom_bot_name: String,
    /// 是否转换 API 里的 ID（Go `bool`，`transform_api_ids`）。
    pub transform_api_ids: bool,
    /// 是否自动上架互动（Go `bool`，`auto_put_interaction`）。
    pub auto_put_interaction: bool,
    /// 上架互动延迟（Go `int`，`put_interaction_delay`）。
    #[serde(deserialize_with = "de_i64_lenient")]
    pub put_interaction_delay: i64,
    /// 上架互动例外（Go `[]string`，`put_interaction_except`）。
    pub put_interaction_except: Vec<String>,

    // ---- onebot 修改 ----
    /// 双向 echo（Go `bool`，`twoway_echo`）。
    pub twoway_echo: bool,
    /// 消息段数组上报（Go `bool`，`array`）。
    pub array: bool,
    /// 原生 OneBot v11 行为（Go `bool`，`native_ob11`）。
    pub native_ob11: bool,
    /// 是否禁用错误通道（Go `bool`，`disable_error_chan`）。
    pub disable_error_chan: bool,
    /// 消息 ID 以字符串上报（Go `bool`，`string_ob11`）。
    pub string_ob11: bool,
    /// action 名称以字符串处理（Go `bool`，`string_action`）。
    pub string_action: bool,
    /// CQ 码解析器模式：`legacy` / `shadow` / `new`（Go `string`，`cq_parse_mode`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub cq_parse_mode: String,
    /// 分层架构接入模式：`legacy` / `shadow` / `new`（Go `string`，`arch_mode`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub arch_mode: String,

    // ---- url 相关 ----
    /// 是否对外暴露真实 IP（Go `bool`，`visible_ip`）。
    pub visible_ip: bool,
    /// URL 是否转成二维码图片（Go `bool`，`url_to_qrimage`）。
    pub url_to_qrimage: bool,
    /// 二维码尺寸（Go `int`，`qr_size`）。
    #[serde(deserialize_with = "de_i64_lenient")]
    pub qr_size: i64,
    /// 是否转换 URL（Go `bool`，`transfer_url`）。
    pub transfer_url: bool,

    // ---- 框架修改 ----
    /// 标题（Go `string`，`title`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub title: String,
    /// frp 端口（Go `string`，`frp_port`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub frp_port: String,
    /// 合并 webhook（Go `string`，`union_webhook`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub union_webhook: String,
    /// 是否合并 ID（Go `bool`，`union_id`）。
    pub union_id: bool,

    // ---- MD 相关 ----
    /// 自定义模板 ID（Go `string`，`custom_template_id`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub custom_template_id: String,
    /// 键盘 ID（Go `string`，`keyboard_id`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub keyboard_id: String,
    /// 是否启用原生 Markdown（Go `bool`，`native_md`）。
    pub native_md: bool,
    /// 换行是否作为块（Go `bool`，`enters_as_block`）。
    pub enters_as_block: bool,
    /// 是否强制校验图片资源（Go `bool`，`force_verify_image_resource`）。
    pub force_verify_image_resource: bool,

    // ---- 发送行为修改 ----
    /// 被动转主动消息复用 message_id（Go `bool`，`lazy_message_id`）。
    pub lazy_message_id: bool,
    /// 随机 msg_seq（Go `bool`，`ramdom_seq`）。
    pub ramdom_seq: bool,
    /// 论坛 Bot 标题（Go `string`，`bot_forum_title`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub bot_forum_title: String,
    /// 被动转主动重试次数（Go `int`，tag 为 `AMsgRetryAsPMsg_Count`）。
    #[serde(rename = "AMsgRetryAsPMsg_Count", deserialize_with = "de_i64_lenient")]
    pub ato_p_count: i64,
    /// 发送延迟（毫秒，Go `int`，`send_delay`）。
    #[serde(deserialize_with = "de_i64_lenient")]
    pub send_delay: i64,
    /// 是否启用替换词（Go `bool`，tag 为 `enableChangeWord`）。
    #[serde(rename = "enableChangeWord")]
    pub enable_change_word: bool,
    /// 默认替换词（Go `string`，tag 为 `defaultChangeWord`）。
    #[serde(rename = "defaultChangeWord", deserialize_with = "de_string_lenient")]
    pub default_change_word: String,

    // ---- 错误临时修复类 ----
    /// 11300 错误修复开关（Go `bool`，`fix_11300`）。
    pub fix_11300: bool,
    /// 仅 HTTP Bot 模式（Go `bool`，`http_only_bot`）。
    pub http_only_bot: bool,
    /// 是否不替换 AppID（Go `bool`，`do_not_replace_appid`）。
    pub do_not_replace_appid: bool,

    // ---- 内置指令 ----
    /// 绑定指令前缀（Go `string`，`bind_prefix`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub bind_prefix: String,
    /// 我的信息指令前缀（Go `string`，`me_prefix`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub me_prefix: String,
    /// 状态指令前缀（Go `string`，`status_prefix`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub status_prefix: String,
    /// 广播指令前缀（Go `string`，`broadcast_prefix`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub broadcast_prefix: String,
    /// 解锁指令前缀（Go `string`，`unlock_prefix`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub unlock_prefix: String,
    /// 链接指令前缀（Go `string`，`link_prefix`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub link_prefix: String,
    /// 是否自动链接（Go `bool`，`auto_link`）。
    pub auto_link: bool,
    /// 音乐指令前缀（Go `string`，`music_prefix`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub music_prefix: String,
    /// 可链接的 Bot 列表（Go `[]string`，`link_bots`）。
    pub link_bots: Vec<String>,
    /// 链接说明文本（Go `string`，`link_text`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub link_text: String,
    /// 链接说明图片（Go `string`，`link_pic`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub link_pic: String,
    /// 链接块行数（Go `int`，`link_lines`）。
    #[serde(deserialize_with = "de_i64_lenient")]
    pub link_lines: i64,
    /// 链接块列数（Go `int`，`link_num`）。
    #[serde(deserialize_with = "de_i64_lenient")]
    pub link_num: i64,

    // ---- HTTP API 配置 ----
    /// 正向 HTTP API 监听地址（Go `string`，`http_address`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub http_address: String,
    /// HTTP API 访问令牌（Go `string`，tag 为 `http_access_token`）。
    #[serde(rename = "http_access_token", deserialize_with = "de_string_lenient")]
    pub access_token: String,
    /// OneBot 上报的版本号（Go `int`，`http_version`）。
    #[serde(deserialize_with = "de_i64_lenient")]
    pub http_version: i64,
    /// HTTP 超时秒数（Go `int`，`http_timeout`）。
    #[serde(deserialize_with = "de_i64_lenient")]
    pub http_timeout: i64,
    /// HTTP POST 上报地址列表（Go `[]string`，`post_url`）。
    pub post_url: Vec<String>,
    /// HTTP POST 上报签名密钥（Go `[]string`，`post_secret`）。
    pub post_secret: Vec<String>,
    /// HTTP POST 最大重试次数（Go `[]int`，`post_max_retries`）。
    pub post_max_retries: Vec<i64>,
    /// HTTP POST 重试间隔（Go `[]int`，`post_retries_interval`）。
    pub post_retries_interval: Vec<i64>,

    // ---- 腾讯云配置 ----
    /// 腾讯云 COS 桶名（Go `string`，tag 为 `t_COS_BUCKETNAME`）。
    #[serde(rename = "t_COS_BUCKETNAME", deserialize_with = "de_string_lenient")]
    pub tencent_bucket_name: String,
    /// 腾讯云 COS 地域（Go `string`，tag 为 `t_COS_REGION`）。
    #[serde(rename = "t_COS_REGION", deserialize_with = "de_string_lenient")]
    pub tencent_bucket_region: String,
    /// 腾讯云 SecretId（Go `string`，tag 为 `t_COS_SECRETID`）。
    #[serde(rename = "t_COS_SECRETID", deserialize_with = "de_string_lenient")]
    pub tencent_cos_secretid: String,
    /// 腾讯云 SecretKey（Go `string`，tag 为 `t_COS_SECRETKEY`）。
    #[serde(rename = "t_COS_SECRETKEY", deserialize_with = "de_string_lenient")]
    pub tencent_secret_key: String,
    /// 腾讯云内容审核（Go `bool`，tag 为 `t_audit`）。
    #[serde(rename = "t_audit")]
    pub tencent_audit: bool,

    // ---- 百度云配置 ----
    /// 百度云 BOS 桶名（Go `string`，tag 为 `b_BOS_BUCKETNAME`）。
    #[serde(rename = "b_BOS_BUCKETNAME", deserialize_with = "de_string_lenient")]
    pub baidu_bos_bucket_name: String,
    /// 百度云 AK（Go `string`，tag 为 `b_BCE_AK`）。
    #[serde(rename = "b_BCE_AK", deserialize_with = "de_string_lenient")]
    pub baidu_bce_ak: String,
    /// 百度云 SK（Go `string`，tag 为 `b_BCE_SK`）。
    #[serde(rename = "b_BCE_SK", deserialize_with = "de_string_lenient")]
    pub baidu_bce_sk: String,
    /// 百度云内容审核（Go `int`，tag 为 `b_audit`）。
    #[serde(rename = "b_audit", deserialize_with = "de_i64_lenient")]
    pub baidu_audit: i64,

    // ---- 阿里云配置 ----
    /// 阿里云 OSS EndPoint（Go `string`，tag 为 `a_OSS_EndPoint`）。
    #[serde(rename = "a_OSS_EndPoint", deserialize_with = "de_string_lenient")]
    pub aliyun_endpoint: String,
    /// 阿里云 OSS 桶名（Go `string`，tag 为 `a_OSS_BucketName`）。
    #[serde(rename = "a_OSS_BucketName", deserialize_with = "de_string_lenient")]
    pub aliyun_bucket_name: String,
    /// 阿里云 AccessKeyId（Go `string`，tag 为 `a_OSS_AccessKeyId`）。
    #[serde(rename = "a_OSS_AccessKeyId", deserialize_with = "de_string_lenient")]
    pub aliyun_access_key_id: String,
    /// 阿里云 AccessKeySecret（Go `string`，tag 为 `a_OSS_AccessKeySecret`）。
    #[serde(
        rename = "a_OSS_AccessKeySecret",
        deserialize_with = "de_string_lenient"
    )]
    pub aliyun_access_key_secret: String,
    /// 阿里云内容审核（Go `bool`，tag 为 `a_audit`）。
    #[serde(rename = "a_audit")]
    pub aliyun_audit: bool,

    // ---- 额外 intent 位探测 ----
    /// 额外探测的 intent 位（Go `[]int`，`extra_intents`）。
    pub extra_intents: Vec<i64>,
    /// 是否自动订阅所有未使用的 intent 位（Go `bool`，`discover_unknown_events`）。
    pub discover_unknown_events: bool,
    /// 是否屏蔽平台不允许的 intent（Go `bool`，`suppress_disallowed_intents`）。
    pub suppress_disallowed_intents: bool,
}

impl Settings {
    /// 与 Go `config.GetPortValue()` 等价：原样返回配置中的端口字符串。
    #[must_use]
    pub fn port_value(&self) -> &str {
        &self.port
    }
}

/// 视觉前缀配置（Go `VisualPrefixConfig`，`structs/structs.go:227-231`）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct VisualPrefixConfig {
    /// 视觉前缀（Go `string`，`prefix`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub prefix: String,
    /// 白名单用户（Go `[]string`，tag 为 `whiteList`）。
    #[serde(rename = "whiteList")]
    pub white_list: Vec<String>,
    /// 非白名单回复文本（Go `string`，tag 为 `No_White_Response`）。
    #[serde(rename = "No_White_Response", deserialize_with = "de_string_lenient")]
    pub no_white_response: String,
}

/// 腾讯云 COS 图床配置（Go `ImageHostingCOS`，`structs/structs.go:235-241`）。
///
/// 仅当 `oss_type=4` 时生效，凭据非空时自动启用。
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct ImageHostingCOS {
    /// SecretId（Go `string`，`secret_id`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub secret_id: String,
    /// SecretKey（Go `string`，`secret_key`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub secret_key: String,
    /// 地域（Go `string`，`region`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub region: String,
    /// 桶名（Go `string`，`bucket`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub bucket: String,
    /// 自定义访问域名（Go `string`，`domain`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub domain: String,
}

/// B 站图床配置（Go `ImageHostingBilibili`，`structs/structs.go:245-249`）。
///
/// 仅当 `oss_type=5` 时生效，凭据非空时自动启用。
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct ImageHostingBilibili {
    /// CSRF token（Go `string`，`csrf_token`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub csrf_token: String,
    /// SESSDATA（Go `string`，`sessdata`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub sessdata: String,
    /// 桶名（Go `string`，`bucket`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub bucket: String,
}

/// QQ 频道图床配置（Go `ImageHostingQQChannel`，`structs/structs.go:254-257`）。
///
/// 仅当 `oss_type=6` 时生效，凭据非空时自动启用。`token` 填写完整 Authorization 值，
/// 如 `"QQBot xxx.yyy"`。
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct ImageHostingQQChannel {
    /// 子频道 ID（Go `string`，`channel_id`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub channel_id: String,
    /// 完整 Authorization 值（Go `string`，`token`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub token: String,
}

/// 开箱即用的免费图床（Go `ImageHostingSimple`，`structs/structs.go:261`）。
///
/// ChatGLM / Ukaka / 星野 / Nature 共用：Go 侧是空结构体，无需任何配置项。
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct ImageHostingSimple {}

/// 接受 YAML 整数、无符号整数、整值浮点或数字字符串（字符串可带下划线，如 `1_000`）。
///
/// Rust 侧超集：Go yaml.v3 对 `int` 字段拒绝 `"5"` 与 `5.0`；这里放宽，但整值浮点必须
/// 小数部分为 0 且落在 `i64` 范围内，绝不静默截断。
fn de_i64_lenient<'de, D: Deserializer<'de>>(deserializer: D) -> Result<i64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Int(i64),
        Uint(u64),
        Float(f64),
        Text(String),
    }

    // 显式 null（如 `log_level:` 空值）在 Go yaml.v3 里等同零值，这里同样归一为 0。
    let Some(raw) = Option::<Raw>::deserialize(deserializer)? else {
        return Ok(0);
    };

    match raw {
        Raw::Int(value) => Ok(value),
        Raw::Uint(value) => {
            i64::try_from(value).map_err(|_| D::Error::custom("整数超出 int64 范围"))
        }
        Raw::Float(value) => float_to_i64(value).map_err(D::Error::custom),
        Raw::Text(text) => strip_numeric_underscores(&text)
            .parse::<i64>()
            .map_err(|err| D::Error::custom(format!("非法整数 {text:?}: {err}"))),
    }
}

/// 接受 YAML 无符号整数、整数、整值浮点或数字字符串（Rust 侧超集，见 [`de_i64_lenient`]）。
fn de_u64_lenient<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Uint(u64),
        Int(i64),
        Float(f64),
        Text(String),
    }

    // 显式 null（如 `app_id:` 空值）→ Go 的零值 0。
    let Some(raw) = Option::<Raw>::deserialize(deserializer)? else {
        return Ok(0);
    };

    match raw {
        Raw::Uint(value) => Ok(value),
        Raw::Int(value) => {
            u64::try_from(value).map_err(|_| D::Error::custom("期望非负整数（Go uint64）"))
        }
        Raw::Float(value) => float_to_u64(value).map_err(D::Error::custom),
        Raw::Text(text) => strip_numeric_underscores(&text)
            .parse::<u64>()
            .map_err(|err| D::Error::custom(format!("非法无符号整数 {text:?}: {err}"))),
    }
}

/// 数字文本预处理：去掉 Go / yaml.v3 允许的下划线分隔符（`1_000` → `1000`）。
///
/// 仅支持下划线：base-0 写法（`0x10` / `0o10` / `0b10`）只有作为 YAML 裸标量时才生效
/// （由解析器直接解成整数）；写成带引号的字符串时本实现不接受，Go 的
/// `strconv.ParseInt(..., 0, 64)` 会接受，属已知差异。
fn strip_numeric_underscores(text: &str) -> String {
    text.trim().replace('_', "")
}

/// 接受 YAML 字符串或裸标量（Go 侧是 `string`，但用户常把数字/布尔写成裸标量）并归一为字符串。
///
/// 归一规则：取 YAML 解析后的**规范化标量文本**——整数按十进制、浮点走 ryu（`5.0` 保留
/// 一位小数）、布尔 `true`/`false`；`null` → `""`（Go 零值）；映射/序列写进 string 字段
/// 一律报错（Go 同样报错）。引号字符串与 `1_000` 这类普通标量原样保留。
///
/// 与 Go yaml.v3 的差异（v3 对 string 字段保留标量原文）：`5.00` → `"5.0"`、
/// `1e3` → `"1000.0"`、`0x10` → `"16"`；普通十进制/布尔标量与 v3 一致。
fn de_string_lenient<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    // 先落到 `Value` 再取文本：只有这样才能保住浮点的标量原文（`5.0` 不能变成 `"5"`）。
    let value = Option::<serde_norway::Value>::deserialize(deserializer)?
        .unwrap_or(serde_norway::Value::Null);
    scalar_to_string(&value).map_err(D::Error::custom)
}

/// YAML 标量 → 规范化后的标量文本（`null` 为零值空串）。
fn scalar_to_string(value: &serde_norway::Value) -> Result<String, String> {
    match value {
        serde_norway::Value::Null => Ok(String::new()),
        serde_norway::Value::Bool(value) => Ok(value.to_string()),
        serde_norway::Value::Number(value) => Ok(value.to_string()),
        serde_norway::Value::String(value) => Ok(value.clone()),
        // `!!str 5` 这类显式标签：取内层标量的文本，与 yaml.v3 一致
        serde_norway::Value::Tagged(tagged) => scalar_to_string(&tagged.value),
        other => Err(format!("期望字符串标量，收到 {other:?}")),
    }
}

/// 嵌套结构体字段：显式 null 归一为默认值（Go yaml.v3 对 `cos:` 空值保留零值结构体）。
fn de_default_on_null<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

/// 整值浮点 → `i64`；非整值/非有限/超范围一律报错（不静默截断）。
fn float_to_i64(value: f64) -> Result<i64, String> {
    if !value.is_finite() || value.fract() != 0.0 {
        return Err(format!("期望整数，收到非整值浮点 {value}"));
    }
    // `i64::MAX as f64` 精确等于 2^63，故上界必须用 `>=`：边界值 2^63 若放行，`as i64`
    // 会静默饱和成 `i64::MAX`。下界 `i64::MIN as f64` 恰为 -2^63（可表示），保留 `<`。
    if value < i64::MIN as f64 || value >= i64::MAX as f64 {
        return Err(format!("浮点整数 {value} 超出 int64 范围"));
    }
    Ok(value as i64)
}

/// 整值浮点 → `u64`；非整值/非有限/负数/超范围一律报错。
fn float_to_u64(value: f64) -> Result<u64, String> {
    if !value.is_finite() || value.fract() != 0.0 {
        return Err(format!("期望整数，收到非整值浮点 {value}"));
    }
    // `u64::MAX as f64` 精确等于 2^64，上界同样必须用 `>=`，否则 2^64 会被饱和成 `u64::MAX`。
    if value < 0.0 || value >= u64::MAX as f64 {
        return Err(format!("浮点整数 {value} 超出 uint64 范围"));
    }
    Ok(value as u64)
}

#[cfg(test)]
mod tests {
    use serde_norway::Value;

    use crate::loader::parse_str;
    use crate::settings::{ImageHostingCOS, ImageHostingSimple, Settings};

    /// 全字段夹具：`Settings` 的 184 个 yaml tag 一个不漏地显式赋值。
    ///
    /// 键集与取值由 `fixture_covers_every_settings_field` 反查，所以这份夹具同时充当
    /// "Go yaml tag → Rust 字段"的覆盖表。
    const FULL_FIXTURE: &str = r##"
ws_address: [ws://127.0.0.1:18080/onebot/v11/ws]
ws_token: [ws-token-a, ws-token-b]
reconnect_times: 7
heart_beat_interval: 30
launch_reconnect_times: 11
app_id: 102001234
uin: 10001
token: bot-token
client_secret: bot-secret
shard_count: 2
shard_id: 1
use_uin: true
idmap_isolation: true
idmap_legacy_compat: true
shard_num: 2
text_intent: [GUILD_MESSAGES, DIRECT_MESSAGE]
global_interaction_to_message: true
global_group_msg_rre_to_message: true
global_group_msg_reject_message: group-reject
global_group_msg_receive_message: group-receive
global_c2c_msg_switch_to_message: true
global_c2c_msg_reject_message: c2c-reject
global_c2c_msg_receive_message: c2c-receive
hash_id: true
op_userid_type: openid
msgid_ttl_seconds: 600
server_dir: ./data
port: "5700"
backup_port: "5701"
lotus: true
lotus_password: lotus-pass
lotus_without_idmaps: true
lotus_without_uploadpic: true
lotus_grpc: true
lotus_grpc_port: 9800
master_id: ["10001", "10002"]
record_sampleRate: 24000
record_bitRate: 32000
card_nick: nick
auto_bind: true
oss_type: 4
image_sizelimit: 1048576
image_limit: 6
url_pic_transfer: true
uploadpicv2_b64: true
cos:
  secret_id: cos-id
  secret_key: cos-key
  region: ap-guangzhou
  bucket: cos-bucket
  domain: cos.example.com
bilibili:
  csrf_token: bili-csrf
  sessdata: bili-sess
  bucket: bili-bucket
qq_channel:
  channel_id: channel-1
  token: QQBot abc.def
chatglm: {}
ukaka: {}
xingye: {}
nature: {}
ws_server_path: /ws
enable_ws_server: true
ws_server_token: ws-server-token
identify_file: true
identify_appids: [102001234, 102005678]
crt: ./crt.pem
key: ./key.pem
use_self_crt: true
webhook_path: /onebot
webhook_prefix_ip: ["127.0.0.1/32", "192.168.1.0/24"]
force_ssl: true
http_port_after_ssl: "5702"
developer_log: true
log_level: 3
save_logs: true
log_suffix_per_mins: 60
log_color_enabled: true
log_max_age_days: 7
log_max_size_mb: 64
log_keep_files: 10
log_slow_event_threshold_ms: 500
disable_webui: true
server_user_name: admin
server_user_password: webui-pass
remove_prefix: true
remove_at: true
convert_other_at: true
remove_bot_at_group: true
add_at_group: true
white_prefix_mode: true
v_white_prefix_mode: true
white_prefixs: ["/", "!"]
white_bypass: [10001, 10002]
white_enable: [true, false, true]
white_bypass_reverse: true
No_White_Response: not-in-whitelist
black_prefix_mode: true
black_prefixs: ["#"]
alias: ["/a", "/b"]
enters: ["/c"]
enters_except: ["/d"]
visual_prefixs:
  - prefix: "!"
    whiteList: ["10001", "10002"]
    No_White_Response: nested-no-white
auto_withdraw: ["/x"]
auto_withdraw_time: 5
visual_prefixs_bypass: ["/y"]
develop_access_token_dir: ./token
develop_bot_id: "102009999"
sandbox_mode: true
dev_message_id: true
send_error: true
save_error: true
downtime_message: bot-offline
memory_msgid: true
threads_ret_msg: true
no_ret_msg: true
self_introduce: ["hello"]
get_g_list_delay: 120
forward_msg_limit: 3
custom_bot_name: Gensokyo
transform_api_ids: true
auto_put_interaction: true
put_interaction_delay: 300
put_interaction_except: ["/skip"]
twoway_echo: true
array: true
native_ob11: true
disable_error_chan: true
string_ob11: true
string_action: true
cq_parse_mode: new
arch_mode: shadow
visible_ip: true
url_to_qrimage: true
qr_size: 256
transfer_url: true
title: Gensokyo-Bot
frp_port: "7500"
union_webhook: /union
union_id: true
custom_template_id: tpl-1
keyboard_id: kb-1
native_md: true
enters_as_block: true
force_verify_image_resource: true
lazy_message_id: true
ramdom_seq: true
bot_forum_title: forum
AMsgRetryAsPMsg_Count: 3
send_delay: 200
enableChangeWord: true
defaultChangeWord: change-me
fix_11300: true
http_only_bot: true
do_not_replace_appid: true
bind_prefix: /bind
me_prefix: /me
status_prefix: /status
broadcast_prefix: /broadcast
unlock_prefix: /unlock
link_prefix: /link
auto_link: true
music_prefix: /music
link_bots: ["10001"]
link_text: link-text
link_pic: link-pic.png
link_lines: 5
link_num: 2
http_address: 127.0.0.1:5700
http_access_token: http-token
http_version: 11
http_timeout: 30
post_url: ["http://127.0.0.1:8080/cb"]
post_secret: ["post-secret"]
post_max_retries: [3, 4]
post_retries_interval: [1000, 2000]
t_COS_BUCKETNAME: t-bucket
t_COS_REGION: ap-shanghai
t_COS_SECRETID: t-id
t_COS_SECRETKEY: t-key
t_audit: true
b_BOS_BUCKETNAME: b-bucket
b_BCE_AK: b-ak
b_BCE_SK: b-sk
b_audit: 1
a_OSS_EndPoint: oss-cn-hangzhou.aliyuncs.com
a_OSS_BucketName: a-bucket
a_OSS_AccessKeyId: a-id
a_OSS_AccessKeySecret: a-secret
a_audit: true
extra_intents: [8, 16]
discover_unknown_events: true
suppress_disallowed_intents: true
"##;

    /// 映射键集合（去序）；非映射输入直接 panic —— 夹具本身写错时要立刻炸。
    fn mapping_keys(value: &Value) -> Vec<String> {
        let mut keys: Vec<String> = value
            .as_mapping()
            .expect("期望 YAML 映射")
            .keys()
            .map(|key| key.as_str().expect("期望字符串键").to_owned())
            .collect();
        keys.sort_unstable();
        keys
    }

    #[test]
    fn parses_go_field_types_from_native_yaml() {
        let yaml = r#"
ws_address:
  - ws://127.0.0.1:8080/onebot/v11/ws
ws_token:
  - token-a
  - token-b
webhook_path: /onebot
app_id: 1234567890123
port: 8080
log_level: 3
msgid_ttl_seconds: 600
oss_type: 4
"#;
        let settings = parse_str(yaml).expect("解析失败");

        // M-1 的 5 个字段：类型与 Go 真源一致
        assert_eq!(settings.ws_token, vec!["token-a", "token-b"]);
        assert_eq!(settings.webhook_path, "/onebot");
        assert_eq!(settings.app_id, 1_234_567_890_123);
        assert_eq!(settings.port_value(), "8080");
        assert_eq!(settings.log_level, 3);
        // 结构体级 default：未出现的字段取零值
        assert_eq!(settings.reconnect_times, 0);
        assert!(settings.server_dir.is_empty());
    }

    #[test]
    fn accepts_quoted_numbers_as_superset_of_go_yaml_v3() {
        // 注意：Go yaml.v3 对 int/uint 字段**拒绝**带引号数字；这里是刻意的 Rust 侧超集
        let yaml = r#"
app_id: "1234567890123"
port: "8080"
log_level: "3"
msgid_ttl_seconds: "600"
oss_type: "4"
reconnect_times: "12"
"#;
        let settings = parse_str(yaml).expect("解析失败");

        assert_eq!(settings.app_id, 1_234_567_890_123);
        assert_eq!(settings.port_value(), "8080");
        assert_eq!(settings.log_level, 3);
        assert_eq!(settings.msgid_ttl_seconds, 600);
        assert_eq!(settings.oss_type, 4);
        assert_eq!(settings.reconnect_times, 12);
    }

    #[test]
    fn accepts_integral_floats_and_bools_as_superset_of_go_yaml_v3() {
        let yaml = r#"
log_level: 5.0
msgid_ttl_seconds: 600.0
oss_type: 4.0
port: 5.0
webhook_path: true
"#;
        let settings = parse_str(yaml).expect("解析失败");

        assert_eq!(settings.log_level, 5);
        assert_eq!(settings.msgid_ttl_seconds, 600);
        assert_eq!(settings.oss_type, 4);
        assert_eq!(
            settings.port_value(),
            "5.0",
            "string 字段保留标量原文，与 yaml.v3 一致"
        );
        assert_eq!(settings.webhook_path, "true", "布尔归一为 Go 文本形式");
    }

    #[test]
    fn string_fields_preserve_scalar_text_like_go_yaml_v3() {
        let yaml = r#"
port: 5.0
title: 5
backup_port: 5.5
frp_port: true
http_port_after_ssl: "5702"
union_webhook:
"#;
        let settings = parse_str(yaml).expect("解析失败");

        // yaml.v3 把标量原文直接写进 string 字段：`5.0` 不得被归一成 `"5"`
        assert_eq!(settings.port_value(), "5.0");
        assert_eq!(settings.title, "5");
        assert_eq!(settings.backup_port, "5.5");
        assert_eq!(settings.frp_port, "true");
        assert_eq!(settings.http_port_after_ssl, "5702");
        assert_eq!(settings.union_webhook, "", "null → Go 零值空串");

        // 映射/序列写进 string 字段必须报错（Go 同样报错，不做静默转换）
        assert!(parse_str("port: [1, 2]").is_err());
        assert!(parse_str("port:\n  nested: 1").is_err());
    }

    #[test]
    fn rejects_values_on_the_saturation_boundary() {
        // 2^63 / 2^64 恰是 f64 可表示值：越界一档必须报错，不能靠 `as` 静默饱和
        assert!(parse_str("log_level: 9223372036854775808").is_err(), "2^63");
        assert!(parse_str("app_id: 18446744073709551616").is_err(), "2^64");
        assert!(
            parse_str("log_level: 9.223372036854776e18").is_err(),
            "2^63 浮点"
        );
        assert!(
            parse_str("app_id: 1.8446744073709552e19").is_err(),
            "2^64 浮点"
        );

        // 边界之内的极值必须原样保留：同时证明 uin 是有符号、app_id 是无符号
        let settings =
            parse_str("uin: -9223372036854775808\napp_id: 18446744073709551615").expect("解析失败");
        assert_eq!(settings.uin, i64::MIN);
        assert_eq!(settings.app_id, u64::MAX);
    }

    #[test]
    fn numeric_fields_accept_underscore_separators() {
        // Go / yaml.v3 允许下划线分组：裸标量与带引号写法都要接受
        let settings = parse_str(
            r#"
log_level: 1_000
app_id: "102_001_234"
msgid_ttl_seconds: -1_000
"#,
        )
        .expect("下划线数字必须解析成功");

        assert_eq!(settings.log_level, 1_000);
        assert_eq!(settings.app_id, 102_001_234);
        assert_eq!(settings.msgid_ttl_seconds, -1_000);

        // 只去掉下划线，不改变其余校验口径：仍然只认十进制
        assert!(parse_str("log_level: 1_0x0").is_err());
        assert!(parse_str("app_id: \"0x10\"").is_err());
    }

    #[test]
    fn utf8_bom_is_ignored() {
        // 纯 BOM / BOM + 注释都必须视为空文档；BOM + 内容要正常解析
        for text in ["\u{feff}", "\u{feff}# 注释\n", "\u{feff}\n\n"] {
            let settings = parse_str(text).expect("纯 BOM 必须解析成功");
            assert_eq!(settings, Settings::default(), "必须是零值: {text:?}");
        }

        let settings = parse_str("\u{feff}log_level: 3").expect("BOM + 内容必须解析成功");
        assert_eq!(settings.log_level, 3);
    }

    #[test]
    fn rejects_non_integral_float_negative_app_id_and_non_numeric_text() {
        // 非整值浮点不得静默截断
        assert!(parse_str("log_level: 5.5").is_err());
        assert!(parse_str("oss_type: -0.5").is_err());
        assert!(parse_str("app_id: 1.5").is_err());
        // 负数进不了 uint64
        assert!(parse_str("app_id: -1").is_err());
        // 非数字串
        assert!(parse_str("log_level: high").is_err());
        assert!(parse_str("app_id: \"abc\"").is_err());
    }

    #[test]
    fn explicit_null_is_zero_value_like_go_yaml_v3() {
        // `port:` / `app_id:` / `log_level:` 显式空值：Go yaml.v3 解码为零值，不得解析失败
        let settings = parse_str("port:\napp_id:\nlog_level:\nwebhook_path:\n").expect("解析失败");

        assert_eq!(settings.port_value(), "");
        assert_eq!(settings.app_id, 0);
        assert_eq!(settings.log_level, 0);
        assert_eq!(settings.webhook_path, "");
    }

    #[test]
    fn parses_full_go_settings_fixture() {
        let settings = parse_str(FULL_FIXTURE).expect("解析失败");

        // 反向 ws / 基础配置
        assert_eq!(
            settings.ws_address,
            vec!["ws://127.0.0.1:18080/onebot/v11/ws"]
        );
        assert_eq!(settings.reconnect_times, 7);
        assert_eq!(settings.heart_beat_interval, 30);
        assert_eq!(settings.launch_reconnect_times, 11);
        assert_eq!(settings.app_id, 102_001_234);
        assert_eq!(settings.uin, 10_001);
        assert_eq!(settings.shard_count, 2);
        assert_eq!(settings.shard_id, 1);
        assert_eq!(settings.shard_num, 2);
        assert!(settings.use_uin && settings.idmap_isolation && settings.idmap_legacy_compat);

        // 特殊大小写 tag：Go 的驼峰与下划线原样保留
        assert_eq!(settings.record_sample_rate, 24_000, "record_sampleRate");
        assert_eq!(settings.record_bit_rate, 32_000, "record_bitRate");
        assert_eq!(settings.ato_p_count, 3, "AMsgRetryAsPMsg_Count");
        assert!(settings.enable_change_word, "enableChangeWord");
        assert_eq!(
            settings.default_change_word, "change-me",
            "defaultChangeWord"
        );
        assert_eq!(
            settings.no_white_response, "not-in-whitelist",
            "No_White_Response"
        );
        assert_eq!(settings.username, "admin", "server_user_name");
        assert_eq!(settings.password, "webui-pass", "server_user_password");
        assert_eq!(settings.image_size_limit, 1_048_576, "image_sizelimit");
        assert_eq!(settings.image_limit, 6, "image_limit");
        assert_eq!(
            settings.devlop_ac_dir, "./token",
            "develop_access_token_dir"
        );
        assert_eq!(settings.dev_bot_id, "102009999", "develop_bot_id");
        assert!(settings.dev_messge_id, "dev_message_id");
        assert_eq!(settings.get_group_list_delay, 120, "get_g_list_delay");
        assert!(
            settings.global_group_msg_reject_recive_event_to_message,
            "rre"
        );
        assert!(settings.v_white_prefix_mode, "v_white_prefix_mode");
        assert!(settings.uploadpic_v2_b64, "uploadpicv2_b64");
        assert!(settings.twoway_echo, "twoway_echo");
        assert!(settings.fix_11300, "fix_11300");

        // 云厂商 tag（t_COS_* / b_* / a_OSS_*）
        assert_eq!(settings.tencent_bucket_name, "t-bucket", "t_COS_BUCKETNAME");
        assert_eq!(
            settings.tencent_bucket_region, "ap-shanghai",
            "t_COS_REGION"
        );
        assert_eq!(settings.tencent_cos_secretid, "t-id", "t_COS_SECRETID");
        assert_eq!(settings.tencent_secret_key, "t-key", "t_COS_SECRETKEY");
        assert!(settings.tencent_audit, "t_audit");
        assert_eq!(
            settings.baidu_bos_bucket_name, "b-bucket",
            "b_BOS_BUCKETNAME"
        );
        assert_eq!(settings.baidu_bce_ak, "b-ak", "b_BCE_AK");
        assert_eq!(settings.baidu_bce_sk, "b-sk", "b_BCE_SK");
        assert_eq!(settings.baidu_audit, 1, "b_audit");
        assert_eq!(
            settings.aliyun_endpoint, "oss-cn-hangzhou.aliyuncs.com",
            "a_OSS_EndPoint"
        );
        assert_eq!(settings.aliyun_bucket_name, "a-bucket", "a_OSS_BucketName");
        assert_eq!(settings.aliyun_access_key_id, "a-id", "a_OSS_AccessKeyId");
        assert_eq!(
            settings.aliyun_access_key_secret, "a-secret",
            "a_OSS_AccessKeySecret"
        );
        assert!(settings.aliyun_audit, "a_audit");

        // 嵌套结构体：cos / bilibili / qq_channel / visual_prefixs
        assert_eq!(settings.cos.secret_id, "cos-id");
        assert_eq!(settings.cos.secret_key, "cos-key");
        assert_eq!(settings.cos.region, "ap-guangzhou");
        assert_eq!(settings.cos.bucket, "cos-bucket");
        assert_eq!(settings.cos.domain, "cos.example.com");
        assert_eq!(settings.bilibili.csrf_token, "bili-csrf");
        assert_eq!(settings.bilibili.sessdata, "bili-sess");
        assert_eq!(settings.bilibili.bucket, "bili-bucket");
        assert_eq!(settings.qq_channel.channel_id, "channel-1");
        assert_eq!(settings.qq_channel.token, "QQBot abc.def");
        assert_eq!(
            settings.chatglm,
            ImageHostingSimple {},
            "空结构体图床照常解码"
        );
        assert_eq!(settings.ukaka, ImageHostingSimple {});
        assert_eq!(settings.xingye, ImageHostingSimple {});
        assert_eq!(settings.nature, ImageHostingSimple {});
        assert_eq!(settings.visual_prefixs.len(), 1);
        assert_eq!(settings.visual_prefixs[0].prefix, "!");
        assert_eq!(
            settings.visual_prefixs[0].white_list,
            vec!["10001", "10002"],
            "whiteList"
        );
        assert_eq!(
            settings.visual_prefixs[0].no_white_response, "nested-no-white",
            "嵌套 No_White_Response"
        );

        // 全部 Vec 字段
        assert_eq!(
            settings.text_intent,
            vec!["GUILD_MESSAGES", "DIRECT_MESSAGE"]
        );
        assert_eq!(settings.master_id, vec!["10001", "10002"]);
        assert_eq!(settings.identify_appids, vec![102_001_234, 102_005_678]);
        assert_eq!(
            settings.webhook_prefix_ip,
            vec!["127.0.0.1/32", "192.168.1.0/24"]
        );
        assert_eq!(settings.white_prefixs, vec!["/", "!"]);
        assert_eq!(settings.white_bypass, vec![10_001, 10_002]);
        assert_eq!(settings.white_enable, vec![true, false, true]);
        assert_eq!(settings.black_prefixs, vec!["#"]);
        assert_eq!(settings.alias, vec!["/a", "/b"]);
        assert_eq!(settings.enters, vec!["/c"]);
        assert_eq!(settings.enters_except, vec!["/d"]);
        assert_eq!(settings.auto_withdraw, vec!["/x"]);
        assert_eq!(settings.visual_prefixs_bypass, vec!["/y"]);
        assert_eq!(settings.self_introduce, vec!["hello"]);
        assert_eq!(settings.put_interaction_except, vec!["/skip"]);
        assert_eq!(settings.link_bots, vec!["10001"]);
        assert_eq!(settings.post_url, vec!["http://127.0.0.1:8080/cb"]);
        assert_eq!(settings.post_secret, vec!["post-secret"]);
        assert_eq!(settings.post_max_retries, vec![3, 4]);
        assert_eq!(settings.post_retries_interval, vec![1000, 2000]);
        assert_eq!(settings.extra_intents, vec![8, 16]);

        // 其余标量抽查（每个 Go 分组至少一个）
        assert_eq!(settings.token, "bot-token");
        assert_eq!(settings.client_secret, "bot-secret");
        assert_eq!(settings.op_userid_type, "openid");
        assert_eq!(settings.msgid_ttl_seconds, 600);
        assert_eq!(settings.server_dir, "./data");
        assert_eq!(settings.port_value(), "5700");
        assert_eq!(settings.backup_port, "5701");
        assert_eq!(settings.lotus_password, "lotus-pass");
        assert_eq!(settings.lotus_grpc_port, 9800);
        assert_eq!(settings.card_and_nick, "nick");
        assert_eq!(settings.oss_type, 4);
        assert_eq!(settings.ws_server_path, "/ws");
        assert_eq!(settings.ws_server_token, "ws-server-token");
        assert_eq!(settings.crt, "./crt.pem");
        assert_eq!(settings.key, "./key.pem");
        assert_eq!(settings.http_port_after_ssl, "5702");
        assert_eq!(settings.log_suffix_per_mins, 60);
        assert_eq!(settings.log_max_age_days, 7);
        assert_eq!(settings.log_max_size_mb, 64);
        assert_eq!(settings.log_keep_files, 10);
        assert_eq!(settings.log_slow_event_threshold_ms, 500);
        assert_eq!(settings.auto_withdraw_time, 5);
        assert_eq!(settings.downtime_message, "bot-offline");
        assert_eq!(settings.forward_msg_limit, 3);
        assert_eq!(settings.custom_bot_name, "Gensokyo");
        assert_eq!(settings.put_interaction_delay, 300);
        assert_eq!(settings.cq_parse_mode, "new");
        assert_eq!(settings.arch_mode, "shadow");
        assert_eq!(settings.qr_size, 256);
        assert_eq!(settings.title, "Gensokyo-Bot");
        assert_eq!(settings.frp_port, "7500");
        assert_eq!(settings.union_webhook, "/union");
        assert_eq!(settings.custom_template_id, "tpl-1");
        assert_eq!(settings.keyboard_id, "kb-1");
        assert_eq!(settings.bot_forum_title, "forum");
        assert_eq!(settings.send_delay, 200);
        assert_eq!(settings.bind_prefix, "/bind");
        assert_eq!(settings.music_prefix, "/music");
        assert_eq!(settings.link_text, "link-text");
        assert_eq!(settings.link_pic, "link-pic.png");
        assert_eq!(settings.link_lines, 5);
        assert_eq!(settings.link_num, 2);
        assert_eq!(settings.http_address, "127.0.0.1:5700");
        assert_eq!(settings.access_token, "http-token");
        assert_eq!(settings.http_version, 11);
        assert_eq!(settings.http_timeout, 30);
        assert_eq!(settings.oss_type, 4);
        assert!(settings.discover_unknown_events && settings.suppress_disallowed_intents);
    }

    #[test]
    fn fixture_covers_every_settings_field() {
        let fixture: Value = serde_norway::from_str(FULL_FIXTURE).expect("夹具必须是合法 YAML");
        let settings = parse_str(FULL_FIXTURE).expect("解析失败");
        let serialized = serde_norway::to_value(&settings).expect("序列化失败");

        // 键集一一对应：漏迁字段 / tag 写错都会让这里不等
        let fixture_keys = mapping_keys(&fixture);
        assert_eq!(fixture_keys.len(), 184, "Go Settings 顶层 yaml tag 总数");
        assert_eq!(
            fixture_keys,
            mapping_keys(&serialized),
            "夹具键集必须与结构体字段一一对应"
        );

        // 嵌套结构体的 tag 数量（Go 侧 5 + 3 + 2 + 3）
        assert_eq!(mapping_keys(fixture.get("cos").expect("cos")).len(), 5);
        assert_eq!(
            mapping_keys(fixture.get("bilibili").expect("bilibili")).len(),
            3
        );
        assert_eq!(
            mapping_keys(fixture.get("qq_channel").expect("qq_channel")).len(),
            2
        );
        let visual_prefix = fixture
            .get("visual_prefixs")
            .and_then(|list| list.get(0))
            .expect("visual_prefixs[0]");
        assert_eq!(mapping_keys(visual_prefix).len(), 3);

        // 全量取值相等：每个字段都必须承载夹具里写的值（含 Vec / 嵌套）
        assert_eq!(serialized, fixture, "往返后每个字段的值都必须与夹具一致");
    }

    #[test]
    fn empty_yaml_is_all_zero_values() {
        // 空文档与纯注释文档：Go yaml.v3 不报错，得到全零值
        for text in ["", "  \n\n", "# 只有注释\n# 第二行\n"] {
            let settings = parse_str(text).expect("空文档必须解析成功");
            assert_eq!(
                settings,
                Settings::default(),
                "空 YAML 必须得到全零值: {text:?}"
            );
        }

        let settings = parse_str("").expect("解析失败");
        assert!(settings.ws_address.is_empty());
        assert!(settings.ws_token.is_empty());
        assert_eq!(settings.app_id, 0);
        assert_eq!(settings.uin, 0);
        assert!(settings.port.is_empty());
        assert!(!settings.use_uin);
        assert!(settings.cos == ImageHostingCOS::default());
        assert!(settings.visual_prefixs.is_empty());

        // 嵌套结构体显式空值同样取零值（Go yaml.v3 语义）
        let settings = parse_str("cos:\nchatglm:\nvisual_prefixs:\n").expect("解析失败");
        assert_eq!(settings.cos, ImageHostingCOS::default());
        assert_eq!(settings.chatglm, ImageHostingSimple {});
        assert!(settings.visual_prefixs.is_empty());
    }

    #[test]
    fn document_markers_and_null_are_all_zero_values() {
        // yaml.v3：只含文档标记（`---` / `...`）、null 标量（`null` / `~`）或注释的文档
        // 解码成零值而不报错；`serde_norway` 对纯 `---` / `null` 文档会报 EOF / 类型错误。
        for text in [
            "---\n",
            "---\n...\n",
            "null\n",
            "~\n",
            "---\nnull\n...\n",
            "# 注释\n---\n# 注释\n...\n",
            "--- # 文档标记\nnull # 空值\n",
        ] {
            let settings = parse_str(text).expect("空文档必须解析成功");
            assert_eq!(
                settings,
                Settings::default(),
                "空文档必须得到零值: {text:?}"
            );
        }
    }

    #[test]
    fn unknown_fields_are_ignored_like_go_yaml_v3() {
        let yaml = r#"
log_level: 2
unknown_top_field: 1
cos:
  secret_id: cos-id
  unknown_cos_field: x
visual_prefixs:
  - prefix: "!"
    unknown_nested: y
"#;
        let settings = parse_str(yaml).expect("未知字段不得导致解析失败");

        assert_eq!(settings.log_level, 2);
        assert_eq!(settings.cos.secret_id, "cos-id");
        assert_eq!(settings.visual_prefixs[0].prefix, "!");
        assert!(settings.visual_prefixs[0].white_list.is_empty());
    }
}

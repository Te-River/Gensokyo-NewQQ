//! `Settings` 的 `Get*()` 等价访问器，供 runtime / actions 层直接调用。
//!
//! 命名与返回类型（M1b 约定）：
//! - 每个字段一个 `get_<字段名>()`（蛇形）；Go `config/config.go` 里名字不同的以 Go 名为准
//!   （`array` → [`Settings::get_array_value`] / `GetArrayValue()`，`port` 另有
//!   [`Settings::get_port_value`] 别名指向 [`Settings::port_value`]）。
//! - 标量按字段类型返回；`String` 字段返回 `&str`，切片字段返回 `&[T]`（等价 Go 返回
//!   slice header，不复制），嵌套结构体返回 `&`。
//! - 全部访问器只读 `&self`、不 `unwrap`、不 panic；非法值按 Go 语义在读取处回退
//!   （见 [`Settings::get_arch_mode`] / [`Settings::get_cq_parse_mode`]）。
//!
//! 覆盖度：`Settings` 顶层 184 个字段 + 嵌套结构体 13 个字段 = 197 个机械访问器，
//! 外加 `port` 的 Go 名别名与 `white_enable` 的 1 基索引辅助函数，合计 199 个 `get_*()`
//! （`port_value()` 沿用 M0 实现）。

use crate::settings::{
    ImageHostingBilibili, ImageHostingCOS, ImageHostingQQChannel, ImageHostingSimple, Settings,
    VisualPrefixConfig,
};

impl Settings {
    // ---- 反向 ws 设置 ----

    #[must_use]
    pub fn get_ws_address(&self) -> &[String] {
        &self.ws_address
    }

    #[must_use]
    pub fn get_ws_token(&self) -> &[String] {
        &self.ws_token
    }

    #[must_use]
    pub fn get_reconnect_times(&self) -> i64 {
        self.reconnect_times
    }

    #[must_use]
    pub fn get_heart_beat_interval(&self) -> i64 {
        self.heart_beat_interval
    }

    #[must_use]
    pub fn get_launch_reconnect_times(&self) -> i64 {
        self.launch_reconnect_times
    }

    // ---- 基础配置 ----

    #[must_use]
    pub fn get_app_id(&self) -> u64 {
        self.app_id
    }

    #[must_use]
    pub fn get_uin(&self) -> i64 {
        self.uin
    }

    #[must_use]
    pub fn get_token(&self) -> &str {
        &self.token
    }

    #[must_use]
    pub fn get_client_secret(&self) -> &str {
        &self.client_secret
    }

    #[must_use]
    pub fn get_shard_count(&self) -> i64 {
        self.shard_count
    }

    #[must_use]
    pub fn get_shard_id(&self) -> i64 {
        self.shard_id
    }

    #[must_use]
    pub fn get_use_uin(&self) -> bool {
        self.use_uin
    }

    #[must_use]
    pub fn get_idmap_isolation(&self) -> bool {
        self.idmap_isolation
    }

    #[must_use]
    pub fn get_idmap_legacy_compat(&self) -> bool {
        self.idmap_legacy_compat
    }

    #[must_use]
    pub fn get_shard_num(&self) -> i64 {
        self.shard_num
    }

    // ---- 事件订阅类 ----

    #[must_use]
    pub fn get_text_intent(&self) -> &[String] {
        &self.text_intent
    }

    // ---- 转换类 ----

    #[must_use]
    pub fn get_global_interaction_to_message(&self) -> bool {
        self.global_interaction_to_message
    }

    #[must_use]
    pub fn get_global_group_msg_reject_recive_event_to_message(&self) -> bool {
        self.global_group_msg_reject_recive_event_to_message
    }

    #[must_use]
    pub fn get_global_group_msg_reject_message(&self) -> &str {
        &self.global_group_msg_reject_message
    }

    #[must_use]
    pub fn get_global_group_msg_receive_message(&self) -> &str {
        &self.global_group_msg_receive_message
    }

    #[must_use]
    pub fn get_global_c2c_msg_switch_event_to_message(&self) -> bool {
        self.global_c2c_msg_switch_event_to_message
    }

    #[must_use]
    pub fn get_global_c2c_msg_reject_message(&self) -> &str {
        &self.global_c2c_msg_reject_message
    }

    #[must_use]
    pub fn get_global_c2c_msg_receive_message(&self) -> &str {
        &self.global_c2c_msg_receive_message
    }

    #[must_use]
    pub fn get_hash_id(&self) -> bool {
        self.hash_id
    }

    #[must_use]
    pub fn get_op_userid_type(&self) -> &str {
        &self.op_userid_type
    }

    #[must_use]
    pub fn get_msgid_ttl_seconds(&self) -> i64 {
        self.msgid_ttl_seconds
    }

    // ---- gensokyo 互联类 ----

    #[must_use]
    pub fn get_server_dir(&self) -> &str {
        &self.server_dir
    }

    #[must_use]
    pub fn get_port(&self) -> &str {
        &self.port
    }

    #[must_use]
    pub fn get_backup_port(&self) -> &str {
        &self.backup_port
    }

    #[must_use]
    pub fn get_lotus(&self) -> bool {
        self.lotus
    }

    #[must_use]
    pub fn get_lotus_password(&self) -> &str {
        &self.lotus_password
    }

    #[must_use]
    pub fn get_lotus_without_idmaps(&self) -> bool {
        self.lotus_without_idmaps
    }

    #[must_use]
    pub fn get_lotus_without_uploadpic(&self) -> bool {
        self.lotus_without_uploadpic
    }

    #[must_use]
    pub fn get_lotus_grpc(&self) -> bool {
        self.lotus_grpc
    }

    #[must_use]
    pub fn get_lotus_grpc_port(&self) -> i64 {
        self.lotus_grpc_port
    }

    // ---- 增强配置 ----

    #[must_use]
    pub fn get_master_id(&self) -> &[String] {
        &self.master_id
    }

    #[must_use]
    pub fn get_record_sample_rate(&self) -> i64 {
        self.record_sample_rate
    }

    #[must_use]
    pub fn get_record_bit_rate(&self) -> i64 {
        self.record_bit_rate
    }

    #[must_use]
    pub fn get_card_and_nick(&self) -> &str {
        &self.card_and_nick
    }

    #[must_use]
    pub fn get_auto_bind(&self) -> bool {
        self.auto_bind
    }

    // ---- 发图相关 ----

    /// 与 Go `config.GetOssType()` 等价：`0`=未启用，`1`=腾讯云，`2`=百度云，`3`=阿里云，
    /// `4~10`=图床后端（COS / Bilibili / QQ 频道 / ChatGLM / Ukaka / 星野 / Nature）。
    ///
    /// 越界值原样返回（Go 侧同样不做校验），调用方自行判定，绝不 panic。
    #[must_use]
    pub fn get_oss_type(&self) -> i64 {
        self.oss_type
    }

    #[must_use]
    pub fn get_image_size_limit(&self) -> i64 {
        self.image_size_limit
    }

    #[must_use]
    pub fn get_image_limit(&self) -> i64 {
        self.image_limit
    }

    #[must_use]
    pub fn get_url_pic_transfer(&self) -> bool {
        self.url_pic_transfer
    }

    #[must_use]
    pub fn get_uploadpic_v2_b64(&self) -> bool {
        self.uploadpic_v2_b64
    }

    // ---- 图床相关 ----

    #[must_use]
    pub fn get_cos(&self) -> &ImageHostingCOS {
        &self.cos
    }

    #[must_use]
    pub fn get_bilibili(&self) -> &ImageHostingBilibili {
        &self.bilibili
    }

    #[must_use]
    pub fn get_qq_channel(&self) -> &ImageHostingQQChannel {
        &self.qq_channel
    }

    #[must_use]
    pub fn get_chatglm(&self) -> &ImageHostingSimple {
        &self.chatglm
    }

    #[must_use]
    pub fn get_ukaka(&self) -> &ImageHostingSimple {
        &self.ukaka
    }

    #[must_use]
    pub fn get_xingye(&self) -> &ImageHostingSimple {
        &self.xingye
    }

    #[must_use]
    pub fn get_nature(&self) -> &ImageHostingSimple {
        &self.nature
    }

    // ---- 正向 ws 设置 ----

    #[must_use]
    pub fn get_ws_server_path(&self) -> &str {
        &self.ws_server_path
    }

    #[must_use]
    pub fn get_enable_ws_server(&self) -> bool {
        self.enable_ws_server
    }

    #[must_use]
    pub fn get_ws_server_token(&self) -> &str {
        &self.ws_server_token
    }

    // ---- ssl 和链接转换类 ----

    #[must_use]
    pub fn get_identify_file(&self) -> bool {
        self.identify_file
    }

    #[must_use]
    pub fn get_identify_appids(&self) -> &[i64] {
        &self.identify_appids
    }

    #[must_use]
    pub fn get_crt(&self) -> &str {
        &self.crt
    }

    #[must_use]
    pub fn get_key(&self) -> &str {
        &self.key
    }

    #[must_use]
    pub fn get_use_self_crt(&self) -> bool {
        self.use_self_crt
    }

    #[must_use]
    pub fn get_webhook_path(&self) -> &str {
        &self.webhook_path
    }

    #[must_use]
    pub fn get_webhook_prefix_ip(&self) -> &[String] {
        &self.webhook_prefix_ip
    }

    #[must_use]
    pub fn get_force_ssl(&self) -> bool {
        self.force_ssl
    }

    #[must_use]
    pub fn get_http_port_after_ssl(&self) -> &str {
        &self.http_port_after_ssl
    }

    // ---- 日志类 ----

    #[must_use]
    pub fn get_developer_log(&self) -> bool {
        self.developer_log
    }

    #[must_use]
    pub fn get_log_level(&self) -> i64 {
        self.log_level
    }

    #[must_use]
    pub fn get_save_logs(&self) -> bool {
        self.save_logs
    }

    #[must_use]
    pub fn get_log_suffix_per_mins(&self) -> i64 {
        self.log_suffix_per_mins
    }

    #[must_use]
    pub fn get_log_color_enabled(&self) -> bool {
        self.log_color_enabled
    }

    #[must_use]
    pub fn get_log_max_age_days(&self) -> i64 {
        self.log_max_age_days
    }

    #[must_use]
    pub fn get_log_max_size_mb(&self) -> i64 {
        self.log_max_size_mb
    }

    #[must_use]
    pub fn get_log_keep_files(&self) -> i64 {
        self.log_keep_files
    }

    #[must_use]
    pub fn get_log_slow_event_threshold_ms(&self) -> i64 {
        self.log_slow_event_threshold_ms
    }

    // ---- webui 相关 ----

    #[must_use]
    pub fn get_disable_webui(&self) -> bool {
        self.disable_webui
    }

    #[must_use]
    pub fn get_username(&self) -> &str {
        &self.username
    }

    #[must_use]
    pub fn get_password(&self) -> &str {
        &self.password
    }

    // ---- 指令魔法类 ----

    #[must_use]
    pub fn get_remove_prefix(&self) -> bool {
        self.remove_prefix
    }

    /// 与 Go `config.GetRemoveAt()` 等价。
    #[must_use]
    pub fn get_remove_at(&self) -> bool {
        self.remove_at
    }

    /// 与 Go `config.GetConvertOtherAt()` 等价。
    #[must_use]
    pub fn get_convert_other_at(&self) -> bool {
        self.convert_other_at
    }

    #[must_use]
    pub fn get_remove_bot_at_group(&self) -> bool {
        self.remove_bot_at_group
    }

    /// 与 Go `config.GetAddAtGroup()` 等价（Go 侧 `instance == nil` 时返回 `true`，
    /// 这里配置对象必然存在，直接取字段值）。
    #[must_use]
    pub fn get_add_at_group(&self) -> bool {
        self.add_at_group
    }

    #[must_use]
    pub fn get_white_prefix_mode(&self) -> bool {
        self.white_prefix_mode
    }

    #[must_use]
    pub fn get_v_white_prefix_mode(&self) -> bool {
        self.v_white_prefix_mode
    }

    #[must_use]
    pub fn get_white_prefixs(&self) -> &[String] {
        &self.white_prefixs
    }

    #[must_use]
    pub fn get_white_bypass(&self) -> &[i64] {
        &self.white_bypass
    }

    #[must_use]
    pub fn get_white_enable(&self) -> &[bool] {
        &self.white_enable
    }

    /// 与 Go `config.GetWhiteEnable(index)` 等价：**1 基索引**（`index = 1` 取第 0 个元素），
    /// 空切片、越界与 `index <= 0` 一律回退 `true`（Go 侧 `white_enable == nil` 同样返回 `true`）。
    ///
    /// 与 [`Settings::get_white_enable`] 的分工：后者返回整个切片，前者按 1 基群序号查表。
    #[must_use]
    pub fn get_white_enable_at(&self, index: i64) -> bool {
        // `index == i64::MIN` 时 `index - 1` 会溢出，用 checked_sub 兜住并直接走回退分支。
        let Some(adjusted) = index.checked_sub(1) else {
            return true;
        };
        if adjusted < 0 {
            return true;
        }
        usize::try_from(adjusted)
            .ok()
            .and_then(|i| self.white_enable.get(i).copied())
            .unwrap_or(true)
    }

    #[must_use]
    pub fn get_white_bypass_reverse(&self) -> bool {
        self.white_bypass_reverse
    }

    #[must_use]
    pub fn get_no_white_response(&self) -> &str {
        &self.no_white_response
    }

    #[must_use]
    pub fn get_black_prefix_mode(&self) -> bool {
        self.black_prefix_mode
    }

    #[must_use]
    pub fn get_black_prefixs(&self) -> &[String] {
        &self.black_prefixs
    }

    #[must_use]
    pub fn get_alias(&self) -> &[String] {
        &self.alias
    }

    #[must_use]
    pub fn get_enters(&self) -> &[String] {
        &self.enters
    }

    #[must_use]
    pub fn get_enters_except(&self) -> &[String] {
        &self.enters_except
    }

    #[must_use]
    pub fn get_visual_prefixs(&self) -> &[VisualPrefixConfig] {
        &self.visual_prefixs
    }

    #[must_use]
    pub fn get_auto_withdraw(&self) -> &[String] {
        &self.auto_withdraw
    }

    #[must_use]
    pub fn get_auto_withdraw_time(&self) -> i64 {
        self.auto_withdraw_time
    }

    #[must_use]
    pub fn get_visual_prefixs_bypass(&self) -> &[String] {
        &self.visual_prefixs_bypass
    }

    // ---- 开发增强类 ----

    #[must_use]
    pub fn get_devlop_ac_dir(&self) -> &str {
        &self.devlop_ac_dir
    }

    #[must_use]
    pub fn get_dev_bot_id(&self) -> &str {
        &self.dev_bot_id
    }

    #[must_use]
    pub fn get_sandbox_mode(&self) -> bool {
        self.sandbox_mode
    }

    #[must_use]
    pub fn get_dev_messge_id(&self) -> bool {
        self.dev_messge_id
    }

    #[must_use]
    pub fn get_send_error(&self) -> bool {
        self.send_error
    }

    #[must_use]
    pub fn get_save_error(&self) -> bool {
        self.save_error
    }

    #[must_use]
    pub fn get_downtime_message(&self) -> &str {
        &self.downtime_message
    }

    #[must_use]
    pub fn get_memory_msgid(&self) -> bool {
        self.memory_msgid
    }

    #[must_use]
    pub fn get_threads_ret_msg(&self) -> bool {
        self.threads_ret_msg
    }

    #[must_use]
    pub fn get_no_ret_msg(&self) -> bool {
        self.no_ret_msg
    }

    // ---- 增长营销类 ----

    #[must_use]
    pub fn get_self_introduce(&self) -> &[String] {
        &self.self_introduce
    }

    // ---- api 修改 ----

    #[must_use]
    pub fn get_get_group_list_delay(&self) -> i64 {
        self.get_group_list_delay
    }

    #[must_use]
    pub fn get_forward_msg_limit(&self) -> i64 {
        self.forward_msg_limit
    }

    #[must_use]
    pub fn get_custom_bot_name(&self) -> &str {
        &self.custom_bot_name
    }

    #[must_use]
    pub fn get_transform_api_ids(&self) -> bool {
        self.transform_api_ids
    }

    #[must_use]
    pub fn get_auto_put_interaction(&self) -> bool {
        self.auto_put_interaction
    }

    #[must_use]
    pub fn get_put_interaction_delay(&self) -> i64 {
        self.put_interaction_delay
    }

    #[must_use]
    pub fn get_put_interaction_except(&self) -> &[String] {
        &self.put_interaction_except
    }

    // ---- onebot 修改 ----

    #[must_use]
    pub fn get_twoway_echo(&self) -> bool {
        self.twoway_echo
    }

    /// 与 Go `config.GetArrayValue()` 等价：消息是否以消息段数组上报。
    #[must_use]
    pub fn get_array_value(&self) -> bool {
        self.array
    }

    #[must_use]
    pub fn get_native_ob11(&self) -> bool {
        self.native_ob11
    }

    #[must_use]
    pub fn get_disable_error_chan(&self) -> bool {
        self.disable_error_chan
    }

    /// 与 Go `config.GetStringOb11()` 等价：消息 ID 是否以字符串上报。
    #[must_use]
    pub fn get_string_ob11(&self) -> bool {
        self.string_ob11
    }

    #[must_use]
    pub fn get_string_action(&self) -> bool {
        self.string_action
    }

    /// 与 Go `config.GetCQParseMode()` 等价：`legacy`/`shadow`/`new` 原样返回，
    /// 空值与非法值（大小写敏感）回退 `legacy`。
    ///
    /// Go 侧首次命中非法值会 warn-once，告警输出留给 runtime 层（M1c），这里只回退值。
    #[must_use]
    pub fn get_cq_parse_mode(&self) -> &str {
        match self.cq_parse_mode.as_str() {
            "legacy" | "shadow" | "new" => &self.cq_parse_mode,
            _ => "legacy",
        }
    }

    /// 与 Go `config.GetArchMode()` 等价：`legacy`/`shadow`/`new` 原样返回，
    /// 空值与非法值（大小写敏感）回退 `new`。
    ///
    /// Go 侧首次命中非法值会 warn-once，告警输出留给 runtime 层（M1c），这里只回退值。
    #[must_use]
    pub fn get_arch_mode(&self) -> &str {
        match self.arch_mode.as_str() {
            "legacy" | "shadow" | "new" => &self.arch_mode,
            _ => "new",
        }
    }

    // ---- url 相关 ----

    #[must_use]
    pub fn get_visible_ip(&self) -> bool {
        self.visible_ip
    }

    #[must_use]
    pub fn get_url_to_qrimage(&self) -> bool {
        self.url_to_qrimage
    }

    #[must_use]
    pub fn get_qr_size(&self) -> i64 {
        self.qr_size
    }

    #[must_use]
    pub fn get_transfer_url(&self) -> bool {
        self.transfer_url
    }

    // ---- 框架修改 ----

    #[must_use]
    pub fn get_title(&self) -> &str {
        &self.title
    }

    #[must_use]
    pub fn get_frp_port(&self) -> &str {
        &self.frp_port
    }

    #[must_use]
    pub fn get_union_webhook(&self) -> &str {
        &self.union_webhook
    }

    #[must_use]
    pub fn get_union_id(&self) -> bool {
        self.union_id
    }

    // ---- MD 相关 ----

    #[must_use]
    pub fn get_custom_template_id(&self) -> &str {
        &self.custom_template_id
    }

    #[must_use]
    pub fn get_keyboard_id(&self) -> &str {
        &self.keyboard_id
    }

    #[must_use]
    pub fn get_native_md(&self) -> bool {
        self.native_md
    }

    #[must_use]
    pub fn get_enters_as_block(&self) -> bool {
        self.enters_as_block
    }

    #[must_use]
    pub fn get_force_verify_image_resource(&self) -> bool {
        self.force_verify_image_resource
    }

    // ---- 发送行为修改 ----

    /// 与 Go `config.GetLazyMessageId()` 等价。
    #[must_use]
    pub fn get_lazy_message_id(&self) -> bool {
        self.lazy_message_id
    }

    #[must_use]
    pub fn get_ramdom_seq(&self) -> bool {
        self.ramdom_seq
    }

    #[must_use]
    pub fn get_bot_forum_title(&self) -> &str {
        &self.bot_forum_title
    }

    #[must_use]
    pub fn get_ato_p_count(&self) -> i64 {
        self.ato_p_count
    }

    #[must_use]
    pub fn get_send_delay(&self) -> i64 {
        self.send_delay
    }

    #[must_use]
    pub fn get_enable_change_word(&self) -> bool {
        self.enable_change_word
    }

    #[must_use]
    pub fn get_default_change_word(&self) -> &str {
        &self.default_change_word
    }

    // ---- 错误临时修复类 ----

    #[must_use]
    pub fn get_fix_11300(&self) -> bool {
        self.fix_11300
    }

    #[must_use]
    pub fn get_http_only_bot(&self) -> bool {
        self.http_only_bot
    }

    #[must_use]
    pub fn get_do_not_replace_appid(&self) -> bool {
        self.do_not_replace_appid
    }

    // ---- 内置指令 ----

    #[must_use]
    pub fn get_bind_prefix(&self) -> &str {
        &self.bind_prefix
    }

    #[must_use]
    pub fn get_me_prefix(&self) -> &str {
        &self.me_prefix
    }

    #[must_use]
    pub fn get_status_prefix(&self) -> &str {
        &self.status_prefix
    }

    #[must_use]
    pub fn get_broadcast_prefix(&self) -> &str {
        &self.broadcast_prefix
    }

    #[must_use]
    pub fn get_unlock_prefix(&self) -> &str {
        &self.unlock_prefix
    }

    #[must_use]
    pub fn get_link_prefix(&self) -> &str {
        &self.link_prefix
    }

    #[must_use]
    pub fn get_auto_link(&self) -> bool {
        self.auto_link
    }

    #[must_use]
    pub fn get_music_prefix(&self) -> &str {
        &self.music_prefix
    }

    #[must_use]
    pub fn get_link_bots(&self) -> &[String] {
        &self.link_bots
    }

    #[must_use]
    pub fn get_link_text(&self) -> &str {
        &self.link_text
    }

    #[must_use]
    pub fn get_link_pic(&self) -> &str {
        &self.link_pic
    }

    #[must_use]
    pub fn get_link_lines(&self) -> i64 {
        self.link_lines
    }

    #[must_use]
    pub fn get_link_num(&self) -> i64 {
        self.link_num
    }

    // ---- HTTP API 配置 ----

    #[must_use]
    pub fn get_http_address(&self) -> &str {
        &self.http_address
    }

    #[must_use]
    pub fn get_access_token(&self) -> &str {
        &self.access_token
    }

    #[must_use]
    pub fn get_http_version(&self) -> i64 {
        self.http_version
    }

    #[must_use]
    pub fn get_http_timeout(&self) -> i64 {
        self.http_timeout
    }

    #[must_use]
    pub fn get_post_url(&self) -> &[String] {
        &self.post_url
    }

    #[must_use]
    pub fn get_post_secret(&self) -> &[String] {
        &self.post_secret
    }

    #[must_use]
    pub fn get_post_max_retries(&self) -> &[i64] {
        &self.post_max_retries
    }

    #[must_use]
    pub fn get_post_retries_interval(&self) -> &[i64] {
        &self.post_retries_interval
    }

    // ---- 腾讯云配置 ----

    #[must_use]
    pub fn get_tencent_bucket_name(&self) -> &str {
        &self.tencent_bucket_name
    }

    #[must_use]
    pub fn get_tencent_bucket_region(&self) -> &str {
        &self.tencent_bucket_region
    }

    #[must_use]
    pub fn get_tencent_cos_secretid(&self) -> &str {
        &self.tencent_cos_secretid
    }

    #[must_use]
    pub fn get_tencent_secret_key(&self) -> &str {
        &self.tencent_secret_key
    }

    #[must_use]
    pub fn get_tencent_audit(&self) -> bool {
        self.tencent_audit
    }

    // ---- 百度云配置 ----

    #[must_use]
    pub fn get_baidu_bos_bucket_name(&self) -> &str {
        &self.baidu_bos_bucket_name
    }

    #[must_use]
    pub fn get_baidu_bce_ak(&self) -> &str {
        &self.baidu_bce_ak
    }

    #[must_use]
    pub fn get_baidu_bce_sk(&self) -> &str {
        &self.baidu_bce_sk
    }

    #[must_use]
    pub fn get_baidu_audit(&self) -> i64 {
        self.baidu_audit
    }

    // ---- 阿里云配置 ----

    #[must_use]
    pub fn get_aliyun_endpoint(&self) -> &str {
        &self.aliyun_endpoint
    }

    #[must_use]
    pub fn get_aliyun_bucket_name(&self) -> &str {
        &self.aliyun_bucket_name
    }

    #[must_use]
    pub fn get_aliyun_access_key_id(&self) -> &str {
        &self.aliyun_access_key_id
    }

    #[must_use]
    pub fn get_aliyun_access_key_secret(&self) -> &str {
        &self.aliyun_access_key_secret
    }

    #[must_use]
    pub fn get_aliyun_audit(&self) -> bool {
        self.aliyun_audit
    }

    // ---- 额外 intent 位探测 ----

    #[must_use]
    pub fn get_extra_intents(&self) -> &[i64] {
        &self.extra_intents
    }

    #[must_use]
    pub fn get_discover_unknown_events(&self) -> bool {
        self.discover_unknown_events
    }

    #[must_use]
    pub fn get_suppress_disallowed_intents(&self) -> bool {
        self.suppress_disallowed_intents
    }

    /// Go `config.GetPortValue()` 的 Rust 名别名，等价于 [`Settings::port_value`]。
    #[must_use]
    pub fn get_port_value(&self) -> &str {
        self.port_value()
    }
}

impl VisualPrefixConfig {
    #[must_use]
    pub fn get_prefix(&self) -> &str {
        &self.prefix
    }

    #[must_use]
    pub fn get_white_list(&self) -> &[String] {
        &self.white_list
    }

    #[must_use]
    pub fn get_no_white_response(&self) -> &str {
        &self.no_white_response
    }
}

impl ImageHostingCOS {
    #[must_use]
    pub fn get_secret_id(&self) -> &str {
        &self.secret_id
    }

    #[must_use]
    pub fn get_secret_key(&self) -> &str {
        &self.secret_key
    }

    #[must_use]
    pub fn get_region(&self) -> &str {
        &self.region
    }

    #[must_use]
    pub fn get_bucket(&self) -> &str {
        &self.bucket
    }

    #[must_use]
    pub fn get_domain(&self) -> &str {
        &self.domain
    }
}

impl ImageHostingBilibili {
    #[must_use]
    pub fn get_csrf_token(&self) -> &str {
        &self.csrf_token
    }

    #[must_use]
    pub fn get_sessdata(&self) -> &str {
        &self.sessdata
    }

    #[must_use]
    pub fn get_bucket(&self) -> &str {
        &self.bucket
    }
}

impl ImageHostingQQChannel {
    #[must_use]
    pub fn get_channel_id(&self) -> &str {
        &self.channel_id
    }

    #[must_use]
    pub fn get_token(&self) -> &str {
        &self.token
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_str;

    #[test]
    fn bool_accessors_mirror_fields() {
        let settings = Settings {
            string_ob11: true,
            array: true,
            native_ob11: true,
            ..Settings::default()
        };

        assert!(settings.get_string_ob11(), "GetStringOb11 等价");
        assert!(
            settings.get_array_value(),
            "GetArrayValue 等价（字段名 array）"
        );
        assert!(settings.get_native_ob11());
        assert!(!settings.get_twoway_echo());
    }

    #[test]
    fn arch_mode_falls_back_to_new() {
        let mut settings = Settings::default();
        assert_eq!(settings.get_arch_mode(), "new", "空值回退 new");

        settings.arch_mode = "legacy".into();
        assert_eq!(settings.get_arch_mode(), "legacy");
        settings.arch_mode = "shadow".into();
        assert_eq!(settings.get_arch_mode(), "shadow");
        settings.arch_mode = "new".into();
        assert_eq!(settings.get_arch_mode(), "new");

        // 非法值与大小写错写同样回退，不 panic
        for value in ["", "Legacy", " new", "unknown"] {
            settings.arch_mode = value.into();
            assert_eq!(settings.get_arch_mode(), "new", "非法值 {value:?}");
        }
    }

    #[test]
    fn cq_parse_mode_falls_back_to_legacy() {
        let mut settings = Settings::default();
        assert_eq!(settings.get_cq_parse_mode(), "legacy", "空值回退 legacy");

        settings.cq_parse_mode = "shadow".into();
        assert_eq!(settings.get_cq_parse_mode(), "shadow");
        settings.cq_parse_mode = "new".into();
        assert_eq!(settings.get_cq_parse_mode(), "new");

        for value in ["", "New", "new ", "srcds"] {
            settings.cq_parse_mode = value.into();
            assert_eq!(settings.get_cq_parse_mode(), "legacy", "非法值 {value:?}");
        }
    }

    #[test]
    fn oss_type_zero_means_disabled_without_panicking() {
        let mut settings = Settings::default();
        assert_eq!(settings.get_oss_type(), 0, "未配置视为未启用");

        settings.oss_type = 4;
        assert_eq!(settings.get_oss_type(), 4);

        // 越界值原样返回，不 panic、不夹逼
        settings.oss_type = 99;
        assert_eq!(settings.get_oss_type(), 99);
    }

    #[test]
    fn port_value_alias_matches_legacy_accessor() {
        let settings = parse_str("port: 5700").expect("解析失败");

        assert_eq!(settings.port_value(), "5700");
        assert_eq!(settings.get_port_value(), settings.port_value());
        assert_eq!(settings.get_port(), "5700");
    }

    #[test]
    fn slice_accessors_borrow_without_copying() {
        let settings = parse_str(
            r#"
ws_address:
  - "ws://127.0.0.1:18080/onebot/v11/ws"
identify_appids: [102001234]
"#,
        )
        .expect("解析失败");

        assert_eq!(settings.get_ws_address().len(), 1);
        assert_eq!(settings.get_identify_appids(), &[102_001_234]);
        assert_eq!(settings.get_token(), "");

        // 返回的是借用而非副本：指针必须与字段内部缓冲区一致
        assert_eq!(
            settings.get_ws_address().as_ptr(),
            settings.ws_address.as_ptr()
        );
    }

    #[test]
    fn scalar_accessors_keep_signed_and_unsigned_width() {
        let settings = Settings {
            uin: -1,
            app_id: u64::MAX,
            log_level: i64::MIN,
            ..Settings::default()
        };

        assert_eq!(settings.get_uin(), -1);
        assert_eq!(settings.get_app_id(), u64::MAX);
        assert_eq!(settings.get_log_level(), i64::MIN);
    }

    #[test]
    fn nested_struct_accessors_borrow() {
        let settings = parse_str(
            r#"
oss_type: 4
cos:
  secret_id: cos-id
  region: ap-guangzhou
  bucket: cos-bucket
visual_prefixs:
  - prefix: "!"
    whiteList: ["10001"]
    No_White_Response: nope
"#,
        )
        .expect("解析失败");

        // 嵌套结构体返回引用，不 Clone
        let cos: &ImageHostingCOS = settings.get_cos();
        assert_eq!(cos.get_secret_id(), "cos-id");
        assert_eq!(cos.get_region(), "ap-guangzhou");
        assert_eq!(cos.get_bucket(), "cos-bucket");
        assert_eq!(
            std::ptr::from_ref(cos),
            std::ptr::from_ref(&settings.cos),
            "必须是同一块内存"
        );

        let prefix = &settings.get_visual_prefixs()[0];
        assert_eq!(prefix.get_prefix(), "!");
        assert_eq!(prefix.get_white_list(), &["10001".to_owned()]);
        assert_eq!(prefix.get_no_white_response(), "nope");

        // 未配置的图床返回全零嵌套结构体
        assert_eq!(settings.get_bilibili().get_bucket(), "");
        assert!(settings.get_ukaka() == &ImageHostingSimple {});
    }

    #[test]
    fn white_enable_at_is_one_based_with_true_fallback() {
        let settings = Settings {
            white_enable: vec![false, true],
            ..Settings::default()
        };

        // 1 基索引：index=1 取第 0 个元素
        assert!(!settings.get_white_enable_at(1));
        assert!(settings.get_white_enable_at(2));

        // index=0、负数、i64::MIN（`index - 1` 会溢出）与越界一律回退 true
        assert!(settings.get_white_enable_at(0));
        assert!(settings.get_white_enable_at(-1));
        assert!(settings.get_white_enable_at(i64::MIN));
        assert!(settings.get_white_enable_at(3));

        // 空切片：任何索引都是 true（Go 侧 nil 切片同样放行）
        let empty = Settings::default();
        assert!(empty.get_white_enable_at(1));
        assert!(empty.get_white_enable_at(0));
    }
}

//! 字段级 diff 与"重启项"表。
//!
//! diff 的键是 **YAML tag**（`serde_norway::to_value` 顶层映射的键），不是 Rust 字段名，
//! 上层（runtime / WebUI）可以直接拿它与 `config.yml` 的写法对照。
//!
//! 重启项表逐字段转写 Go 真源 `config/config.go:38-48` 的 `restartRequiredFields`：
//! Go 侧写的是结构体字段名（`WsAddress` / `Username` / `CQParseMode`），这里换成对应
//! YAML tag；顺序与 Go 完全一致，便于与真源逐行比对。

use serde::de::Error as _;

use crate::settings::Settings;
/// 需要重启进程才生效的 YAML tag（Go `restartRequiredFields`，34 项）。
pub const RESTART_REQUIRED_TAGS: [&str; 34] = [
    "ws_address",
    "ws_token",
    "reconnect_times",
    "heart_beat_interval",
    "launch_reconnect_times",
    "app_id",
    "uin",
    "token",
    "client_secret",
    "shard_count",
    "shard_id",
    "use_uin",
    "text_intent",
    "server_dir",
    "port",
    "backup_port",
    "lotus",
    "lotus_password",
    "lotus_without_idmaps",
    "ws_server_path",
    "enable_ws_server",
    "ws_server_token",
    "identify_file",
    "identify_appids",
    "crt",
    "key",
    "developer_log",
    "log_level",
    "save_logs",
    "disable_webui",
    // Go `Username` / `Password`，YAML tag 为带 server_ 前缀的历史写法
    "server_user_name",
    "server_user_password",
    "title",
    "cq_parse_mode",
];

/// 该 YAML tag 是否属于"重启后生效"的配置项。
pub fn is_restart_required(tag: &str) -> bool {
    RESTART_REQUIRED_TAGS.contains(&tag)
}

/// 逐字段比较两份配置，返回发生变化的 YAML tag（升序、去重）。
///
/// 比较粒度是**顶层字段**：嵌套结构体（`cos` / `bilibili` / …）内部变化整体体现为
/// 该嵌套 tag 变化，与 Go 侧"按结构体字段比较"一致。
pub fn diff_tags(old: &Settings, new: &Settings) -> Result<Vec<String>, serde_norway::Error> {
    let old = serde_norway::to_value(old)?;
    let new = serde_norway::to_value(new)?;

    let Some(old) = old.as_mapping() else {
        return Err(serde_norway::Error::custom(
            "Settings 的序列化结果不是映射，无法做字段级 diff",
        ));
    };
    let Some(new) = new.as_mapping() else {
        return Err(serde_norway::Error::custom(
            "Settings 的序列化结果不是映射，无法做字段级 diff",
        ));
    };

    let mut changed = Vec::new();
    for (key, new_value) in new {
        let Some(tag) = key.as_str() else {
            return Err(serde_norway::Error::custom(
                "Settings 的序列化结果出现非字符串键",
            ));
        };
        // 旧值缺失（字段被删）同样视为变化
        if old.get(tag) != Some(new_value) {
            changed.push(tag.to_owned());
        }
    }
    for key in old.keys() {
        let Some(tag) = key.as_str() else {
            return Err(serde_norway::Error::custom(
                "Settings 的序列化结果出现非字符串键",
            ));
        };
        if !new.contains_key(tag) {
            changed.push(tag.to_owned());
        }
    }

    changed.sort_unstable();
    changed.dedup();
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_table_matches_go_source() {
        // 与 `config/config.go:38-48` 的 34 项逐一对齐（顺序也保持一致）
        assert_eq!(RESTART_REQUIRED_TAGS.len(), 34);
        assert!(is_restart_required("port"));
        assert!(is_restart_required("server_user_name"));
        assert!(is_restart_required("cq_parse_mode"));
        assert!(!is_restart_required("bind_prefix"));
        assert!(!is_restart_required("server_user_passwordx"));
    }

    #[test]
    fn diff_reports_yaml_tags_only() {
        let old = Settings::default();
        let new = Settings {
            port: "5701".into(),
            bind_prefix: "/bind".into(),
            cos: crate::settings::ImageHostingCOS {
                secret_id: "cos-id".into(),
                ..Default::default()
            },
            ..Default::default()
        };

        let changed = diff_tags(&old, &new).expect("diff 不应失败");
        assert_eq!(
            changed,
            vec![
                "bind_prefix".to_owned(),
                "cos".to_owned(),
                "port".to_owned()
            ]
        );
    }

    #[test]
    fn diff_is_empty_for_equal_settings() {
        let settings = Settings::default();
        assert!(
            diff_tags(&settings, &settings)
                .expect("diff 不应失败")
                .is_empty()
        );
    }

    #[test]
    fn every_restart_tag_exists_in_settings() {
        // tag 写错会让重启项永远匹配不上，这里直接对序列化键集做断言
        let serialized = serde_norway::to_value(Settings::default()).expect("序列化失败");
        let keys = serialized.as_mapping().expect("必须是映射");
        for tag in RESTART_REQUIRED_TAGS {
            assert!(
                keys.contains_key(tag),
                "重启项 tag {tag:?} 不在 Settings 的序列化键集里"
            );
        }
    }
}

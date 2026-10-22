//! `config.yml` 顶层映射。
//!
//! 字段类型与 yaml tag **逐字段对照 Go 真源 `structs/structs.go`**（`Settings` 结构体），
//! 不是按"看起来顺手"取名；M1 迁移剩余字段时按同一张表继续补。
//!
//! 解码宽容度：未知字段忽略、缺失字段取零值（与 Go yaml.v3 一致，因此**不使用**
//! `deny_unknown_fields`）。
//!
//! **有意放宽的 Rust 侧超集**——下列输入 Go yaml.v3 会直接报错，这里接受（仅为容错，
//! 不改变 Go 会接受的输入的解码结果）：带引号的数字字符串（`"5"`）、整值浮点（`5.0`）、
//! 以及 `port`（Go 侧是 `string`，`config.GetPortValue()` 直接返回该字符串）位置上的
//! 数字与布尔。

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

/// `config.yml` 顶层配置。
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Settings {
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
    /// 是否作为 WS 服务端等待下游连接（Go `bool`，`enable_ws_server`）。
    pub enable_ws_server: bool,
    /// 正向 WS / HTTP 服务端口（Go `string`，`port`）。
    #[serde(deserialize_with = "de_string_lenient")]
    pub port: String,
    /// 反向 HTTP 回调路径（Go `string`，`webhook_path`）。
    pub webhook_path: String,
    /// QQ 开放平台 AppID（Go `uint64`，`app_id`）；带引号写法同样接受。
    #[serde(deserialize_with = "de_u64_lenient")]
    pub app_id: u64,
    /// QQ 开放平台令牌（Go `string`，`token`）。
    pub token: String,
    /// QQ 开放平台客户端密钥（Go `string`，`client_secret`）。
    pub client_secret: String,
    /// 是否使用 QQ 号而非 AppID 作为机器标识（Go `bool`，`use_uin`）。
    pub use_uin: bool,
    /// 消息 ID 缓存存活秒数（Go `int`，`msgid_ttl_seconds`）。
    #[serde(deserialize_with = "de_i64_lenient")]
    pub msgid_ttl_seconds: i64,
    /// 图床后端枚举（Go `int`，`oss_type`，4~10；0 表示未启用）。
    #[serde(deserialize_with = "de_i64_lenient")]
    pub oss_type: i64,
    /// 日志级别（Go `int`，`log_level`）。
    #[serde(deserialize_with = "de_i64_lenient")]
    pub log_level: i64,
    /// 数据目录（Go `string`，`server_dir`）。
    pub server_dir: String,
}

impl Settings {
    /// 与 Go `config.GetPortValue()` 等价：原样返回配置中的端口字符串。
    #[must_use]
    pub fn port_value(&self) -> &str {
        &self.port
    }
}

/// 接受 YAML 整数、无符号整数、整值浮点或数字字符串。
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
        Raw::Text(text) => text
            .trim()
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
        Raw::Text(text) => text
            .trim()
            .parse::<u64>()
            .map_err(|err| D::Error::custom(format!("非法无符号整数 {text:?}: {err}"))),
    }
}

/// 接受 YAML 字符串、数字或布尔（Go 侧是 `string` 但用户常写成裸标量），统一归一为字符串。
///
/// 归一规则：整值浮点 → 整数串（`5.0` → `"5"`），其余浮点保留原样（`5.5` → `"5.5"`），
/// 布尔 → `"true"` / `"false"`（Go 的文本形式）。
fn de_string_lenient<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Text(String),
        Int(i64),
        Uint(u64),
        Float(f64),
        Bool(bool),
    }

    // 显式 null（如 `port:` 空值）→ 空字符串，与 Go 零值一致（`GetPortValue()` 返回 ""）。
    let Some(raw) = Option::<Raw>::deserialize(deserializer)? else {
        return Ok(String::new());
    };

    match raw {
        Raw::Text(text) => Ok(text),
        Raw::Int(value) => Ok(value.to_string()),
        Raw::Uint(value) => Ok(value.to_string()),
        Raw::Float(value) => Ok(float_to_string(value)),
        Raw::Bool(value) => Ok(value.to_string()),
    }
}

/// 整值浮点 → `i64`；非整值/非有限/超范围一律报错（不静默截断）。
fn float_to_i64(value: f64) -> Result<i64, String> {
    if !value.is_finite() || value.fract() != 0.0 {
        return Err(format!("期望整数，收到非整值浮点 {value}"));
    }
    if value < i64::MIN as f64 || value > i64::MAX as f64 {
        return Err(format!("浮点整数 {value} 超出 int64 范围"));
    }
    Ok(value as i64)
}

/// 整值浮点 → `u64`；非整值/非有限/负数/超范围一律报错。
fn float_to_u64(value: f64) -> Result<u64, String> {
    if !value.is_finite() || value.fract() != 0.0 {
        return Err(format!("期望整数，收到非整值浮点 {value}"));
    }
    if value < 0.0 || value > u64::MAX as f64 {
        return Err(format!("浮点整数 {value} 超出 uint64 范围"));
    }
    Ok(value as u64)
}

/// 浮点 → 字符串：整值且量级安全时输出整数形式（`5.0` → `"5"`），否则保留浮点原样。
fn float_to_string(value: f64) -> String {
    if value.is_finite() && value.fract() == 0.0 && value.abs() < 9.0e15 {
        format!("{}", value as i64)
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use crate::loader::parse_str;

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
        assert_eq!(settings.port_value(), "5", "整值浮点归一为整数串");
        assert_eq!(settings.webhook_path, "true", "布尔归一为 Go 文本形式");
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
}

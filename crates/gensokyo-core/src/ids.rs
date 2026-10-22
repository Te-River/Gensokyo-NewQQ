//! OneBot 侧虚拟 ID（design §3.1）。
//!
//! 虚拟 ID 由 idmap 建立，与 QQ OpenID 双向映射。`string_ob11` 配置决定它在上报时呈现为
//! JSON 数字还是 JSON 字符串，因此**形态由类型携带**：`Ob11Id::Str` 序列化成字符串，
//! `Ob11Id::Num` 序列化成数字——避免 `string_ob11 = true` 时静默发成整数。

use std::fmt;

use serde::{Deserialize, Serialize};

/// OneBot 侧可见的虚拟 ID。
///
/// 线上形态由 `#[serde(untagged)]` 的变体决定（`Num` → 数字，`Str` → 字符串），
/// 与配置 `string_ob11` 一一对应。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Ob11Id {
    /// 数字模式（`string_ob11 = false`）。
    Num(i64),
    /// 字符串模式（`string_ob11 = true`）。
    Str(String),
}

impl Ob11Id {
    /// 主动推送使用的特殊 ID（对应 Go 的 `messageID == "2000"`）。
    pub const ACTIVE: Self = Self::Num(2000);

    /// 构造数字模式 ID。
    #[must_use]
    pub const fn num(value: i64) -> Self {
        Self::Num(value)
    }

    /// 构造字符串模式 ID。
    #[must_use]
    pub fn str(value: impl Into<String>) -> Self {
        Self::Str(value.into())
    }

    /// 取出数值（仅数字模式；字符串模式返回 `None`，不做隐式转换）。
    #[must_use]
    pub const fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Num(value) => Some(*value),
            Self::Str(_) => None,
        }
    }

    /// 取出字符串（仅字符串模式）。
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Num(_) => None,
            Self::Str(value) => Some(value),
        }
    }

    /// 是否为字符串上报模式（`string_ob11 = true`）。
    #[must_use]
    pub const fn is_string_mode(&self) -> bool {
        matches!(self, Self::Str(_))
    }
}

impl fmt::Display for Ob11Id {
    /// 人类可读展示，只输出内部值本身（两种模式看起来一样），
    /// **不承担 JSON 语义**——JSON 形态由 `Serialize` 决定。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Num(value) => write!(f, "{value}"),
            Self::Str(value) => write!(f, "{value}"),
        }
    }
}

impl From<i64> for Ob11Id {
    fn from(value: i64) -> Self {
        Self::Num(value)
    }
}

impl From<String> for Ob11Id {
    fn from(value: String) -> Self {
        Self::Str(value)
    }
}

impl From<&str> for Ob11Id {
    fn from(value: &str) -> Self {
        Self::Str(value.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::Ob11Id;

    #[test]
    fn number_mode_round_trips_as_bare_integer() {
        let id = Ob11Id::num(123456);
        assert_eq!(id.to_string(), "123456");
        assert_eq!(id.as_i64(), Some(123456));
        assert!(!id.is_string_mode());
    }

    #[test]
    fn string_mode_never_degrades_to_integer() {
        let id: Ob11Id = "123456".into();
        assert_eq!(id.to_string(), "123456", "Display 只做人类可读展示");
        assert_eq!(id.as_str(), Some("123456"));
        assert_eq!(id.as_i64(), None, "字符串模式不得隐式转成整数");
        assert!(id.is_string_mode());
    }

    #[test]
    fn active_marker_matches_go_2000() {
        assert_eq!(Ob11Id::ACTIVE, Ob11Id::Num(2000));
    }

    #[test]
    fn serde_untagged_keeps_wire_shape() {
        // 用 serde 数据模型判定类型（YAML 文本的引号风格不承担 JSON 语义）：
        // Num → 数字，Str → 字符串；落到 JSON 时分别就是 number / string。
        let number = serde_norway::to_value(Ob11Id::num(123)).expect("序列化失败");
        assert!(number.is_number(), "数字模式必须是 number: {number:?}");

        let text = serde_norway::to_value(Ob11Id::str("123")).expect("序列化失败");
        assert!(text.is_string(), "字符串模式必须是 string: {text:?}");
    }

    #[test]
    fn serde_untagged_restores_both_modes() {
        let number: Ob11Id = serde_norway::from_str("123").expect("反序列化失败");
        assert_eq!(number, Ob11Id::Num(123));

        let text: Ob11Id = serde_norway::from_str("\"123\"").expect("反序列化失败");
        assert_eq!(text, Ob11Id::Str("123".to_owned()));
        assert!(text.is_string_mode());
    }
}

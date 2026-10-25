//! `params` 的公共取值规则：发送类与查询类 action 共用一套。
//!
//! OneBot 客户端把同一个 ID 发成数字或字符串都合法（Go 侧靠 `interface{}` 断言 + 若干
//! `strconv` 兜底），因此这里统一做「数字 → 十进制文本」的归一，再由上层按需反序列化。
//! 错误信息必须带字段名：协议类型（serde）的报错只说「invalid type」，定位不到字段。

use gensokyo_core::{Error, Result};
use serde_json::{Map, Number, Value};

/// 取 `params` 对象；缺失或非对象时返回带说明的参数错误。
pub(crate) fn object(params: &Value) -> Result<&Map<String, Value>> {
    params
        .as_object()
        .ok_or_else(|| Error::Protocol("params 缺失或不是 JSON 对象".to_owned()))
}

/// 读取 ID 字段：`None` 表示缺失 / `null` / 空串；`0` 非法（对齐 Go 的 `checkZero*`）。
///
/// 返回值是规范化后的 JSON 值（数字 → 整数，字符串原样），供协议类型反序列化。
pub(crate) fn id_field(object: &Map<String, Value>, key: &str) -> Result<Option<Value>> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if text.is_empty() => Ok(None),
        Some(Value::String(text)) => {
            if text == "0" {
                return Err(Error::Protocol(format!("{key} 不能为 0")));
            }
            Ok(Some(Value::String(text.clone())))
        }
        Some(Value::Number(number)) => {
            let normalized = normalized_number(key, number)?;
            if normalized.as_i64() == Some(0) {
                return Err(Error::Protocol(format!("{key} 不能为 0")));
            }
            Ok(Some(Value::Number(normalized)))
        }
        Some(_) => Err(Error::Protocol(format!("{key} 必须是数字或字符串"))),
    }
}

/// 数字 ID → 整数。
///
/// `Number::to_string()` 会把 `1e3` / `1000.0` 渲染成 `"1000.0"`；这里整数优先，浮点仅在
/// 整值且在 `i64` 范围内时折叠（`1e3` → `1000`），非整值 / 越界拒绝。
pub(crate) fn normalized_number(key: &str, number: &Number) -> Result<Number> {
    if let Some(value) = number.as_i64() {
        return Ok(Number::from(value));
    }
    if let Some(value) = number.as_u64() {
        return Ok(Number::from(value));
    }
    let value = number
        .as_f64()
        .ok_or_else(|| Error::Protocol(format!("{key} 不是有效数字")))?;
    let in_range = (i64::MIN as f64..=i64::MAX as f64).contains(&value);
    if value.fract() != 0.0 || !in_range {
        return Err(Error::Protocol(format!("{key} 必须是整数: {number}")));
    }
    Ok(Number::from(value as i64))
}

/// ID 字段 → 十进制文本（数字与字符串同值同形）。
pub(crate) fn id_text(object: &Map<String, Value>, key: &str) -> Result<Option<String>> {
    let Some(value) = id_field(object, key)? else {
        return Ok(None);
    };
    match value {
        Value::String(text) => Ok(Some(text)),
        Value::Number(number) => Ok(Some(number.to_string())),
        other => Err(Error::Protocol(format!(
            "{key} 必须是数字或字符串: {other}"
        ))),
    }
}

/// 必填 ID → 十进制文本。
pub(crate) fn required_id(object: &Map<String, Value>, key: &str) -> Result<String> {
    id_text(object, key)?.ok_or_else(|| Error::Protocol(format!("缺少 {key}")))
}

/// 可选文本字段：缺失 / `null` / 空串都视为未提供；数字按十进制文本收下（`cursor` 有数字形态）。
pub(crate) fn optional_text(object: &Map<String, Value>, key: &str) -> Result<Option<String>> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if text.is_empty() => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(Value::Number(number)) => Ok(Some(normalized_number(key, number)?.to_string())),
        Some(_) => Err(Error::Protocol(format!("{key} 必须是字符串"))),
    }
}

/// 可选整数：数字（含整值浮点）或十进制字符串。
pub(crate) fn optional_i64(object: &Map<String, Value>, key: &str) -> Result<Option<i64>> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if text.is_empty() => Ok(None),
        Some(Value::String(text)) => text
            .parse::<i64>()
            .map(Some)
            .map_err(|_| Error::Protocol(format!("{key} 必须是整数: {text}"))),
        Some(Value::Number(number)) => normalized_number(key, number)?
            .as_i64()
            .map(Some)
            .ok_or_else(|| Error::Protocol(format!("{key} 必须是整数"))),
        Some(_) => Err(Error::Protocol(format!("{key} 必须是数字或字符串"))),
    }
}

/// 可选非负整数（`limit` 等分页参数）：负数按参数错误拒绝，不做静默取绝对值。
pub(crate) fn optional_u32(object: &Map<String, Value>, key: &str) -> Result<Option<u32>> {
    let Some(value) = optional_i64(object, key)? else {
        return Ok(None);
    };
    u32::try_from(value)
        .map(Some)
        .map_err(|_| Error::Protocol(format!("{key} 超出范围: {value}")))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{id_text, object, optional_i64, optional_u32, required_id};

    fn map(value: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
        value.as_object().expect("测试用 params 都是对象").clone()
    }

    #[test]
    fn numeric_and_string_ids_have_one_text_form() {
        for value in [json!(42), json!("42"), json!(42.0), json!(4.2e1)] {
            let params = map(json!({"group_id": value}));
            assert_eq!(required_id(&params, "group_id").unwrap(), "42");
        }
    }

    #[test]
    fn text_form_keeps_openids_verbatim() {
        let openid = "a".repeat(32);
        let params = map(json!({"group_id": openid}));
        assert_eq!(id_text(&params, "group_id").unwrap(), Some(openid));
    }

    #[test]
    fn missing_null_and_empty_ids_are_absent_or_rejected() {
        for params in [
            map(json!({})),
            map(json!({"group_id": null})),
            map(json!({"group_id": ""})),
        ] {
            assert_eq!(id_text(&params, "group_id").unwrap(), None);
            assert!(required_id(&params, "group_id").is_err());
        }
        assert!(required_id(&map(json!({"group_id": 0})), "group_id").is_err());
    }

    #[test]
    fn ids_must_not_be_arbitrary_json() {
        let params = map(json!({"group_id": [1]}));
        assert!(id_text(&params, "group_id").is_err());
        assert!(object(&json!([])).is_err());
    }

    #[test]
    fn limit_accepts_numbers_and_numeric_strings() {
        assert_eq!(
            optional_u32(&map(json!({"limit": 30})), "limit").unwrap(),
            Some(30)
        );
        assert_eq!(
            optional_u32(&map(json!({"limit": "50"})), "limit").unwrap(),
            Some(50)
        );
        assert!(optional_u32(&map(json!({"limit": 1.5})), "limit").is_err());
        assert!(optional_u32(&map(json!({"limit": -1})), "limit").is_err());
    }

    #[test]
    fn duration_accepts_both_forms_and_negative_values() {
        assert_eq!(
            optional_i64(&map(json!({"duration": 60})), "duration").unwrap(),
            Some(60)
        );
        assert_eq!(
            optional_i64(&map(json!({"duration": "60"})), "duration").unwrap(),
            Some(60)
        );
        assert_eq!(
            optional_i64(&map(json!({"duration": -5})), "duration").unwrap(),
            Some(-5)
        );
    }
}

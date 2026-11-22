//! Go `callapi.ParamsContent` 的整包严格解码契约（活体对照 B01/B02/F02/H04）。
//!
//! Go 在收包层对**整个请求**做一次 `json.Unmarshal`（`wsclient/ws.go:253`）：`params` 落进
//! 强类型的 `ParamsContent`，已声明字段遇到类型不符时整包失败，分发层回
//! `1400「参数解析失败: json: cannot unmarshal string into … of type bool」`
//! （`callapi.ReplyUnparseableMessage`），**请求根本进不了 handler**。Rust 的分发入口把
//! `params` 收成 `serde_json::Value`、handler 按字段宽容取值，同一请求会落到 handler 的业务
//! 拒绝（`whole_ban enable:"yes"→100`）甚至真执行上游（`ban duration:"0"` 发出
//! `restrict_chat_setting`）——校验档位与副作用双重不对称。
//!
//! [`check_params`] 在分发器调用 handler **之前**复刻这份契约：逐字段比对 Go 的声明类型，
//! 类型不符返回带 Go 同款定位路径的错误文案；未声明的多余字段维持宽容（Go 的 Alias 内嵌也
//! 只校验已声明字段）。上游调用因此不可能发生。
//!
//! 口径细节（逐条对照 `callapi/callapi.go:103-186`）：
//! - `interface{}` 字段（`message` / `messages` / `menu` / `panel`）任何 JSON 形状都合法。
//! - 五个 ID 字段（`group_id` / `user_id` / `message_id` / `channel_id` / `guild_id`）有
//!   自定义 switch：null / number / string 全收，object / array 整包失败。
//! - `StringList`（`user_ids` / `user_openids` / `group_openids`）：非数组整包失败；元素级
//!   string / number 归一、其余静默跳过（宽容照抄）。`whitelist_users` 是固定 `[]string`
//!   （元素非字符串即失败）、`group_ids` 是 `[]uint64`（元素非数字即失败）。
//! - `context` / `operation` 嵌套结构体：对象形态逐字段递归；非对象 →
//!   `cannot unmarshal … into Go struct field .Alias.params.context of type callapi.Context`。
//! - `duration` / `limit` / `next_index` 是 Go `int`：字符串数字（"0"）在 Go 一律整包
//!   失败，这里同样拒绝——数值语义只认 JSON 数字。

use serde_json::{Map, Value};

/// Go `ParamsContent` 里一个字段的声明类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Decl {
    /// `bool`
    Bool,
    /// `int`
    Int,
    /// `string`
    Str,
    /// `[]string`（`whitelist_users`）
    StringSlice,
    /// `[]uint64`（`group_ids`）
    Uint64Slice,
    /// `StringList`（柔性字符串数组，见模块文档）
    StringList,
    /// `interface{}`：任何形状都合法
    Any,
    /// `interface{}` 但自定义 switch 只接受 null / number / string（五个 ID 字段）
    IdLike,
    /// `Context` 结构体
    Context,
    /// `Operation` 结构体
    Operation,
}

/// 字段名 → Go 声明类型。与 `callapi/callapi.go` 的 `ParamsContent` 逐字段对照，
/// 新增 Go 字段时同步这张表，不要按 action 猜。
const PARAM_FIELDS: &[(&str, Decl)] = &[
    ("botqq", Decl::Str),
    ("channel_id", Decl::IdLike),
    ("guild_id", Decl::IdLike),
    ("group_id", Decl::IdLike),
    ("message_id", Decl::IdLike),
    ("message", Decl::Any),
    ("messages", Decl::Any),
    ("user_id", Decl::IdLike),
    ("duration", Decl::Int),
    ("enable", Decl::Bool),
    ("approve", Decl::Bool),
    ("flag", Decl::Str),
    ("reason", Decl::Str),
    ("add_to_member_blacklist", Decl::Bool),
    ("next_index", Decl::Int),
    ("cursor", Decl::Str),
    ("limit", Decl::Int),
    ("strategy_id", Decl::Str),
    ("group_openids", Decl::StringList),
    ("group_ids", Decl::Uint64Slice),
    ("is_enable", Decl::Str),
    ("expire_at", Decl::Str),
    ("remark", Decl::Str),
    ("op", Decl::Str),
    ("whitelist_users", Decl::StringSlice),
    ("scope", Decl::Str),
    ("target_type", Decl::Str),
    ("panel_id", Decl::Str),
    ("user_ids", Decl::StringList),
    ("user_openids", Decl::StringList),
    ("add_blacklist", Decl::Bool),
    ("menu", Decl::Any),
    ("panel", Decl::Any),
    ("context", Decl::Context),
    ("operation", Decl::Operation),
    ("callback_data", Decl::Str),
];

/// `Context` 结构体的字段表（`callapi.go:169-180`）。
const CONTEXT_FIELDS: &[(&str, Decl)] = &[
    ("avatar", Decl::Str),
    ("font", Decl::Int),
    ("message_id", Decl::Int),
    ("message_seq", Decl::Int),
    ("message_type", Decl::Str),
    ("post_type", Decl::Str),
    ("sub_type", Decl::Str),
    ("time", Decl::Int),
    ("user_id", Decl::Int),
    ("group_id", Decl::Int),
];

/// `Operation` 结构体的字段表（`callapi.go:183-186`）。
const OPERATION_FIELDS: &[(&str, Decl)] = &[("reply", Decl::Str), ("at_sender", Decl::Bool)];

fn decl_of(fields: &[(&str, Decl)], key: &str) -> Option<Decl> {
    fields
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, d)| *d)
}

fn go_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "float64",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// 声明类型对应的 Go 类型名（错误文案里的 `of type …`）。
fn want_name(decl: Decl) -> &'static str {
    match decl {
        Decl::Bool => "bool",
        Decl::Int => "int",
        Decl::Str => "string",
        Decl::StringList | Decl::StringSlice => "[]string",
        Decl::Uint64Slice => "[]uint64",
        Decl::Context => "callapi.Context",
        Decl::Operation => "callapi.Operation",
        Decl::Any | Decl::IdLike => "interface {}",
    }
}

/// 单值是否符合声明类型（容器/嵌套的下钻在调用点处理）。
fn shape_ok(decl: Decl, value: &Value) -> bool {
    match (decl, value) {
        (Decl::Any, _) => true,
        (Decl::IdLike, Value::Number(_) | Value::String(_)) => true,
        (Decl::Bool, Value::Bool(_)) => true,
        // Go 的 int 只吃 JSON 数字（float64 解码路径）；字符串数字 "0" 必炸
        (Decl::Int, Value::Number(_)) => true,
        (Decl::Str, Value::String(_)) => true,
        (Decl::Context | Decl::Operation, Value::Object(_)) => true,
        (Decl::StringList | Decl::StringSlice | Decl::Uint64Slice, Value::Array(_)) => true,
        _ => false,
    }
}

fn mismatch(path: &str, actual: &str, want: &str) -> String {
    format!("cannot unmarshal {actual} into Go struct field {path} of type {want}")
}

/// 校验 `params`；通过返回 `Ok(())`，否则给出 Go `encoding/json` 同构的定位文案
/// （`.Alias.params.<字段>` 路径 + `of type <声明>`）。分发层负责加「参数解析失败: 」前缀。
///
/// `params` 缺失 / `null` / 非对象不在这里管——那是 handler 层既有的
/// 「params 缺失或不是 JSON 对象」口径（Go 的空 params 反序列化为零值结构体，同样不在这一
/// 步失败）。
pub fn check_params(params: Option<&Value>) -> Result<(), String> {
    let Some(Value::Object(object)) = params else {
        return Ok(());
    };
    for (key, value) in object {
        if value.is_null() {
            // Go 对显式 null：interface{} 置 nil、具体类型保持零值，均不报错
            continue;
        }
        let Some(decl) = decl_of(PARAM_FIELDS, key) else {
            continue;
        };
        if !shape_ok(decl, value) {
            return Err(mismatch(
                &format!(".Alias.params.{key}"),
                go_type(value),
                want_name(decl),
            ));
        }
        match decl {
            Decl::Context => {
                check_nested(
                    object.get(key).and_then(Value::as_object),
                    CONTEXT_FIELDS,
                    key,
                )?;
            }
            Decl::Operation => {
                check_nested(
                    object.get(key).and_then(Value::as_object),
                    OPERATION_FIELDS,
                    key,
                )?;
            }
            Decl::StringSlice | Decl::Uint64Slice => {
                if let Some(items) = value.as_array() {
                    for item in items {
                        let ok = match decl {
                            Decl::StringSlice => item.is_string(),
                            _ => item.is_number(),
                        };
                        if !ok {
                            return Err(mismatch(
                                &format!(".Alias.params.{key}"),
                                go_type(item),
                                want_name(decl),
                            ));
                        }
                    }
                }
            }
            // StringList：元素约 string / number 归一、其他靝默迋过（Go 无 default 分支）
            _ => {}
        }
    }
    Ok(())
}

fn check_nested(
    object: Option<&Map<String, Value>>,
    fields: &[(&str, Decl)],
    parent: &str,
) -> Result<(), String> {
    let Some(object) = object else {
        return Ok(());
    };
    for (key, value) in object {
        if value.is_null() {
            continue;
        }
        let Some(decl) = decl_of(fields, key) else {
            continue;
        };
        if !shape_ok(decl, value) {
            return Err(mismatch(
                &format!(".Alias.params.{parent}.{key}"),
                go_type(value),
                want_name(decl),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::check_params;
    use serde_json::{Value, json};

    fn err(params: Value) -> String {
        check_params(Some(&params)).expect_err("应判类型不符")
    }

    /// B01/B02：已声明 bool 字段传字符串 → Go 同款整包文案。
    #[test]
    fn declared_bool_field_rejects_string() {
        assert_eq!(
            err(json!({"group_id": "123", "enable": "yes"})),
            "cannot unmarshal string into Go struct field .Alias.params.enable of type bool"
        );
        assert!(err(json!({"approve": "yes"})).contains(".Alias.params.approve of type bool"));
        assert!(
            err(json!({"add_blacklist": 1})).contains(".Alias.params.add_blacklist of type bool")
        );
        assert!(
            err(json!({"add_to_member_blacklist": "true"}))
                .contains(".Alias.params.add_to_member_blacklist of type bool")
        );
    }

    /// F02/H04：duration/limit 传字符串数字（含 "0"）→ 必拒，绝不放行到上游。
    #[test]
    fn declared_int_field_rejects_numeric_string() {
        assert_eq!(
            err(json!({"group_id": 1, "user_id": 2, "duration": "0"})),
            "cannot unmarshal string into Go struct field .Alias.params.duration of type int"
        );
        assert!(err(json!({"duration": "abc"})).contains(".Alias.params.duration of type int"));
        assert!(err(json!({"limit": "30"})).contains(".Alias.params.limit of type int"));
        assert!(err(json!({"limit": true})).contains(".Alias.params.limit of type int"));
        assert!(err(json!({"next_index": "2"})).contains(".Alias.params.next_index of type int"));
    }

    /// 已声明 string 字段传数字/布尔/数组 → 拒。
    #[test]
    fn declared_string_field_rejects_number_and_bool() {
        assert!(err(json!({"cursor": 5})).contains(".Alias.params.cursor of type string"));
        assert!(
            err(json!({"strategy_id": 7})).contains(".Alias.params.strategy_id of type string")
        );
        assert!(err(json!({"is_enable": true})).contains(".Alias.params.is_enable of type string"));
        assert!(err(json!({"op": 1})).contains(".Alias.params.op of type string"));
        assert!(err(json!({"panel_id": [1]})).contains(".Alias.params.panel_id of type string"));
    }

    /// ID 族：Go 的自定义 switch 只拒 object / array。
    #[test]
    fn id_like_fields_reject_containers_only() {
        assert!(check_params(Some(&json!({"group_id": "openid"}))).is_ok());
        assert!(check_params(Some(&json!({"group_id": 123}))).is_ok());
        assert!(
            err(json!({"group_id": {"a": 1}}))
                .contains(".Alias.params.group_id of type interface {}")
        );
        assert!(err(json!({"user_id": [1]})).contains(".Alias.params.user_id"));
    }

    /// 列表族：StringList 元素宽容；[]string / []uint64 元素严格。
    #[test]
    fn list_fields_follow_go_leniency() {
        assert!(check_params(Some(&json!({"user_ids": ["1", 2, {}, null]}))).is_ok());
        assert!(err(json!({"user_ids": 42})).contains(".Alias.params.user_ids of type []string"));
        assert!(
            err(json!({"whitelist_users": "a"}))
                .contains(".Alias.params.whitelist_users of type []string")
        );
        assert!(
            err(json!({"whitelist_users": [1]}))
                .contains(".Alias.params.whitelist_users of type []string")
        );
        assert!(
            err(json!({"group_ids": ["x"]})).contains(".Alias.params.group_ids of type []uint64")
        );
    }

    /// context / operation 嵌套结构体逐字段下钻。
    #[test]
    fn nested_context_and_operation_are_checked() {
        assert!(
            err(json!({"context": "x"})).contains(".Alias.params.context of type callapi.Context")
        );
        assert!(
            err(json!({"context": {"group_id": "1"}}))
                .contains(".Alias.params.context.group_id of type int")
        );
        assert!(
            err(json!({"operation": {"at_sender": "yes"}}))
                .contains(".Alias.params.operation.at_sender of type bool")
        );
        assert!(
            check_params(Some(&json!({
                "context": {"message_type": "group", "group_id": 1},
                "operation": {"reply": "hi"}
            })))
            .is_ok()
        );
    }

    /// 未声明字段与 interface{} 字段维持宽容（Go Alias 也只校验已声明字段）。
    #[test]
    fn undeclared_and_interface_fields_pass_through() {
        for params in [
            json!({"unknown_key": "whatever", "msg_type": 0}),
            json!({"message": [{"type": "image", "data": {}}], "menu": {"buttons": true}}),
            // auto_escape / no_cache / times 不在 ParamsContent 声明表内：Go 同样宽容
            json!({"auto_escape": "true", "no_cache": 1, "times": "3"}),
        ] {
            assert!(check_params(Some(&params)).is_ok(), "{params}");
        }
        // 显式 null 不报错（Go 零值语义）
        assert!(check_params(Some(&json!({"enable": null, "duration": null}))).is_ok());
        // 缺 params / 非对象：留给 handler 层既有口径
        assert!(check_params(None).is_ok());
        assert!(check_params(Some(&json!("not an object"))).is_ok());
    }

    /// 合法形状全绿（回归护栏：不许误伤正常请求）。
    #[test]
    fn good_shapes_stay_ok() {
        assert!(
            check_params(Some(&json!({
                "group_id": 1, "user_id": "abc", "duration": 60, "enable": true,
                "approve": false, "limit": 20, "cursor": "c", "user_ids": [1, "2"],
                "flag": "f", "reason": "", "reject_add_request": true,
            })))
            .is_ok()
        );
    }
}

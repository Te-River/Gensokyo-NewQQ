//! 互动事件结构（`botgo/dto/interaction.go`，官方「互动事件」`INTERACTION_CREATE`）。
//!
//! 官方把消息按钮 / 快捷菜单回调归在「互动」族：外层载荷带 `id`、`event_id`、`type`、
//! `scene`、`chat_type`、`timestamp` 与场景 openid，按钮真正的回传值在
//! `data.resolved.button_data`。字段与 Go `dto.Interaction` 一一对应，Go 的 `omitempty`
//! 在这里落成 `skip_serializing_if`（入站反序列化两类形态都收）。

use serde::{Deserialize, Serialize};

/// 互动事件（`INTERACTION_CREATE` 的 `d` 负载）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Interaction {
    /// 平台方事件 ID（回应回调时用）。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// 外层 `event_id`。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub event_id: String,
    /// 互动类型：消息按钮 `11`、单聊快捷菜单 `12`。
    #[serde(rename = "type", skip_serializing_if = "is_zero_u32")]
    pub interaction_type: u32,
    /// 事件发生的场景。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub scene: String,
    /// 场景类型：频道 `0`、群聊 `1`、单聊 `2`。
    #[serde(skip_serializing_if = "is_zero_i32")]
    pub chat_type: i32,
    /// 触发时间（RFC3339）。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub timestamp: String,
    /// 频道的 openid。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub guild_id: String,
    /// 文字子频道的 openid。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub channel_id: String,
    /// 单聊按钮触发的用户 openid。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub user_openid: String,
    /// 群的 openid。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub group_openid: String,
    /// 群成员 openid。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub group_member_openid: String,
    /// 互动数据（按钮回传值在 [`InteractionResolved::button_data`]）。
    ///
    /// Go 侧是指针，缺省即 `nil`；这里同语义用 `Option`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<InteractionData>,
    /// 版本，默认 1。
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub version: u32,
    /// 机器人的 appid。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub application_id: String,
}

/// 互动数据。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct InteractionData {
    /// 标题。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// 数据类型（聊天框搜索为 `9`）。
    #[serde(rename = "type", skip_serializing_if = "is_zero_u32")]
    pub interaction_data_type: u32,
    /// 已解析的按钮信息。
    ///
    /// Go 是结构体值（`omitempty` 对结构体不生效），这里同样恒序列化。
    pub resolved: InteractionResolved,
}

/// 已解析的按钮信息。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct InteractionResolved {
    /// 操作按钮的 `data` 字段值（客户端按它分发指令）。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub button_data: String,
    /// 操作按钮的 `id` 字段值。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub button_id: String,
    /// 操作的用户 id。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub user_id: String,
    /// 操作按钮的 feature id。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub feature_id: String,
    /// 操作的消息 id。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub message_id: String,
}

/// Go `omitempty` 对零值整数的语义。
fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

/// Go `omitempty` 对零值整数的语义。
fn is_zero_i32(value: &i32) -> bool {
    *value == 0
}

#[cfg(test)]
mod tests {
    use super::Interaction;
    use serde_json::json;

    #[test]
    fn deserializes_official_interaction_payload() {
        let payload = json!({
            "id": "i-1",
            "event_id": "e-1",
            "type": 11,
            "scene": "group",
            "chat_type": 1,
            "timestamp": "2026-10-31T10:00:00+08:00",
            "group_openid": "G",
            "group_member_openid": "M",
            "data": {
                "name": "按钮",
                "type": 9,
                "resolved": {
                    "button_data": "/menu",
                    "button_id": "b-1",
                    "user_id": "U",
                    "feature_id": "f-1",
                    "message_id": "m-1"
                }
            },
            "version": 1,
            "application_id": "app"
        });
        let interaction: Interaction = serde_json::from_value(payload).expect("互动事件应可解析");

        assert_eq!(interaction.id, "i-1");
        assert_eq!(interaction.event_id, "e-1");
        assert_eq!(interaction.interaction_type, 11);
        assert_eq!(interaction.chat_type, 1);
        assert_eq!(interaction.group_member_openid, "M");
        let resolved = &interaction.data.as_ref().expect("data 应存在").resolved;
        assert_eq!(resolved.button_data, "/menu");
        assert_eq!(resolved.button_id, "b-1");
        assert_eq!(interaction.version, 1);
    }

    #[test]
    fn missing_optional_fields_fall_back_to_default() {
        let interaction: Interaction =
            serde_json::from_value(json!({"group_openid": "G"})).expect("缺字段不应报错");

        assert_eq!(
            interaction,
            Interaction {
                group_openid: "G".to_owned(),
                ..Interaction::default()
            }
        );
    }

    #[test]
    fn serializes_empty_fields_away_like_go_omitempty() {
        let value = serde_json::to_value(Interaction::default()).expect("应可序列化");

        assert_eq!(value, json!({}));
    }
}

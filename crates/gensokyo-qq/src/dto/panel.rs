//! 指令面板 / 自定义菜单 DTO（对齐 `botgo/dto/panel.go` 与 `menu.go`）。
//!
//! 请求侧结构体带 `deny_unknown_fields`：Go 的 `decodeStrictJSON` 对客户端透传对象做严格
//! 解码，拼错的字段名（如 `sub_menus`）必须报错而不是静默丢弃——`PUT` 是覆盖式写入，
//! 一个被丢掉的字段等于把线上已有的面板 / 菜单配置清空。
//!
//! 响应侧把 `panel` / `menu` 留在 [`Value`] 里原样透传：官方响应只增不减，严格结构体只会
//! 让客户端在官方加字段时解析失败。

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 面板（`items` ≤ 10，`remark` ≤ 155；不展示字段用 `skip_serializing_if` 对齐 `omitempty`）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Panel {
    /// 面板元素列表。
    pub items: Vec<PanelItem>,
    /// 备注。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub remark: String,
    /// 面板版本（仅响应携带；请求里通常省略）。
    #[serde(skip_serializing_if = "is_zero_i64")]
    pub version: i64,
}

/// 面板元素（`name` ≤ 14 字符，`desc` ≤ 80 字符，`type` 为 `command` / `link`）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PanelItem {
    /// 元素名。
    pub name: String,
    /// 元素描述。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub desc: String,
    /// 元素类型：`command` / `link`。
    #[serde(rename = "type")]
    pub item_type: String,
    /// 是否仅管理员可用。
    #[serde(skip_serializing_if = "is_false")]
    pub only_admin: bool,
    /// 跳转链接（仅 `type=link` 有效）。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub link: String,
}

/// 面板记录（`GET /v2/panels` 的 `records` 条目）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelRecord {
    /// 面板 ID。
    pub panel_id: String,
    /// 生效场景：`c2c` / `group` / `channel` / `dm`。
    pub scope: String,
    /// 关联对象形态：`all` / `specific`。
    pub target_type: String,
    /// 面板内容（原样透传）。
    pub panel: Value,
    /// 创建时间（RFC3339）。
    pub created_at: String,
    /// 更新时间（RFC3339）。
    pub updated_at: String,
    /// 面板版本。
    pub version: i64,
}

/// 单个面板详情（`GET /v2/panels/{panel_id}`）：记录全字段 + 关联对象。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelRecordDetail {
    /// 面板 ID。
    pub panel_id: String,
    /// 生效场景。
    pub scope: String,
    /// 关联对象形态。
    pub target_type: String,
    /// 面板内容（原样透传）。
    pub panel: Value,
    /// 创建时间（RFC3339）。
    pub created_at: String,
    /// 更新时间（RFC3339）。
    pub updated_at: String,
    /// 面板版本。
    pub version: i64,
    /// 关联用户 OpenID（`target_type=specific` 时返回）。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub user_openids: Vec<String>,
    /// 关联群 OpenID（`target_type=specific` 时返回）。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub group_openids: Vec<String>,
}

/// 面板列表（`next_cursor` 空串表示末页）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelList {
    /// 面板记录。
    pub records: Vec<PanelRecord>,
    /// 下一页游标。
    pub next_cursor: String,
    /// 是否已到末页。
    pub is_end: bool,
}

/// 创建面板请求（`POST /v2/panels`）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CreatePanelRequest {
    /// 生效场景（必填）。
    pub scope: String,
    /// 关联对象形态：`all` / `specific`（`c2c` / `group` 支持 `specific`）。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub target_type: String,
    /// 关联用户 OpenID（≤ 20，仅 `c2c` + `specific`）。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub user_openids: Vec<String>,
    /// 关联群 OpenID（≤ 20，仅 `group` + `specific`）。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub group_openids: Vec<String>,
    /// 面板内容（必填）。
    pub panel: Panel,
}

/// 创建面板响应。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CreatePanelResponse {
    /// 新建面板 ID。
    pub panel_id: String,
}

/// 更新面板请求（`PUT /v2/panels/{panel_id}`，覆盖元素与备注，不影响关联对象）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UpdatePanelRequest {
    /// 面板内容。
    pub panel: Panel,
}

/// 更新面板 / 更新菜单的版本响应。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct VersionResponse {
    /// 新版本号。
    pub version: i64,
}

/// 增删面板关联对象请求（`PUT /v2/panels/{panel_id}/target`，各 ≤ 20）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PanelTargetRequest {
    /// `add` / `del`。
    pub op: String,
    /// 用户 OpenID 列表。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub user_openids: Vec<String>,
    /// 群 OpenID 列表。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub group_openids: Vec<String>,
}

/// 自定义菜单（`items` ≤ 10；仅 C2C 场景全局生效）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Menu {
    /// 一级菜单项。
    pub items: Vec<MenuItem>,
}

/// 一级菜单项（`name` ≤ 10 字符，汉字算 2）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MenuItem {
    /// 菜单名。
    pub name: String,
    /// `switch` / `send_message` / `link` / `menu`。
    #[serde(rename = "type")]
    pub item_type: String,
    /// 子菜单项（仅 `type=menu`，≤ 5，不支持再嵌套）。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sub_menu_items: Vec<SubMenuItem>,
    /// 发送文本（`type=send_message`）。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub send_message: String,
    /// 跳转链接（`type=link`，必须 `https://`）。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub link: String,
    /// 开关配置（`type=switch`）。
    #[serde(rename = "switch", skip_serializing_if = "Option::is_none")]
    pub switch: Option<MenuSwitch>,
}

/// 开关型菜单项配置。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MenuSwitch {
    /// 开关 ID。
    pub switch_id: String,
    /// 默认是否开启。
    #[serde(skip_serializing_if = "is_false")]
    pub default: bool,
}

/// 子菜单项（`name` ≤ 14 字符，`type` 为 `send_message` / `link`）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SubMenuItem {
    /// 子菜单名。
    pub name: String,
    /// `send_message` / `link`。
    #[serde(rename = "type")]
    pub item_type: String,
    /// 发送文本。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub send_message: String,
    /// 跳转链接。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub link: String,
}

/// 自定义菜单响应（未设置过时 `menu` 为 `null`）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MenuResponse {
    /// 菜单版本。
    pub version: i64,
    /// 菜单内容（原样透传）。
    pub menu: Value,
}

/// 设置自定义菜单请求（`PUT /v2/menu`，覆盖式）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PutMenuRequest {
    /// 菜单内容。
    pub menu: Menu,
}

/// `i64` 是否为 0（`omitempty` 语义）。
fn is_zero_i64(value: &i64) -> bool {
    *value == 0
}

/// `bool` 是否为 `false`（`omitempty` 语义）。
fn is_false(value: &bool) -> bool {
    !*value
}

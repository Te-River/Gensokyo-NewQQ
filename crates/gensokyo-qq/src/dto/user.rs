//! 用户结构（`botgo/dto/user.go` 的最小子集 + 官方 2026-11-16 的用户资料）。

use serde::{Deserialize, Serialize};

/// `GET /users/@me` 响应：只保留机器人自身 ID。
///
/// Go 侧同一份数据是 `dto.User`（`main.go:254` 的 `api.Me`），真正被消费的只有
/// `me.ID`（`main.go:304` → `handlers.BotID`），其余字段（`username` / `avatar` /
/// `bot`）不参与任何逻辑，因此这里不建模，响应里出现也照样忽略。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct BotUser {
    /// 机器人真实 ID（AppID→BotID 文本替换的目标）。
    pub id: String,
}

/// `GET /v2/users/{user_openid}/info` 响应（官方 2026-11-16 新增）。
///
/// 三个字段都可能缺省：`allow_proactive_msg` 缺省时按 `false`（Go 的零值），
/// `username` / `avatar` 缺省按空串。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserInfo {
    /// 用户昵称。
    #[serde(default)]
    pub username: String,
    /// 是否允许机器人主动发消息。
    #[serde(default)]
    pub allow_proactive_msg: bool,
    /// 头像直链。
    #[serde(default)]
    pub avatar: String,
}

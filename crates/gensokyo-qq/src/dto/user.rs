//! 机器人身份结构（`botgo/dto/user.go` 的最小子集）。

use serde::Deserialize;

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

//! [`OutboundPlan`]：Go 版 `foundItems` 的类型化替代（design §3.3）。
//!
//! Go 用 `map[string][]string` + "循环跳过控制型 key、循环内再回读 reply" 的隐式约定；
//! 这里用字段与枚举把同一语义显式表达，key 映射保持 1:1。
//!
//! M0 只定型容器与媒体引用；markdown / keyboard / card / input_notify / stream 仍以原始
//! JSON 字符串承载（Go 侧同样是 base64 或 JSON 字符串），类型化 payload 在 M1 落地。

use std::path::PathBuf;

/// 媒体来源。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaRef {
    /// 本地文件。
    Local(PathBuf),
    /// 远程 URL。
    Url(String),
    /// base64 内容。
    Base64(String),
    /// 无法识别、需回退处理的内容（Go 的 `unknown_image` 等）。
    Unknown(String),
}

/// 主动推送标记（对应 Go 的 `active` / `active_type` / `active_sub_type` / `wakeup`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveKind {
    /// 主动推送大类。
    pub kind: String,
    /// 子类型。
    pub sub_type: Option<String>,
    /// 唤醒消息 ID（`send_private_msg_wakeup`）。
    pub wakeup: Option<String>,
}

/// 文件引用：`file_name` 与文件本身绑定，避免 Go 里"循环跳过再回读"的隐式约定。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRef {
    /// 文件来源。
    pub media: MediaRef,
    /// 显示文件名。
    pub file_name: Option<String>,
}

/// 一条出站消息的完整意图。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OutboundPlan {
    /// 文本主体。
    pub text: String,
    /// 回复目标消息 ID。
    pub reply_msg_id: Option<String>,
    /// 主动推送标记。
    pub active: Option<ActiveKind>,
    /// Markdown 消息（原始 JSON 字符串）。
    pub markdown: Vec<String>,
    /// 按钮 / 键盘（原始 JSON 字符串）。
    pub keyboard: Vec<String>,
    /// 图片。
    pub images: Vec<MediaRef>,
    /// 语音。
    pub records: Vec<MediaRef>,
    /// 视频。
    pub videos: Vec<MediaRef>,
    /// 文件。
    pub files: Vec<FileRef>,
    /// QQ 音乐分享 ID 列表。
    pub qqmusic: Vec<String>,
    /// 图文卡片（原始 JSON 字符串）。
    pub card: Vec<String>,
    /// 输入状态（原始 JSON 字符串）。
    pub input_notify: Option<String>,
    /// 流式消息（原始 JSON 字符串）。
    pub stream: Option<String>,
    /// 无法识别的媒体，按到达顺序回退发送。
    pub unknown: Vec<MediaRef>,
}

impl OutboundPlan {
    /// 是否没有任何可发送内容。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
            && self.markdown.is_empty()
            && self.keyboard.is_empty()
            && self.images.is_empty()
            && self.records.is_empty()
            && self.videos.is_empty()
            && self.files.is_empty()
            && self.qqmusic.is_empty()
            && self.card.is_empty()
            && self.input_notify.is_none()
            && self.stream.is_none()
            && self.unknown.is_empty()
    }
}

//! QQ 侧 DTO。
//!
//! - [`message`]：发送 / 上传 / 响应结构（`botgo/dto/message_create.go`）。
//! - [`group`]：群管理结构（`botgo/dto/group.go`、`group_member.go`）。

pub mod group;
pub mod message;

pub use group::{
    AutoApprovedInfo, BotInGroupState, GlobalMuteRule, GroupInfo, GroupMember, GroupMemberList,
    JoinRequest, JoinRequestList, MemberMuteState, MuteRecurringRule, MuteScheduleRule,
    RestrictChatSetting, ReviewQa, VerifyInfo,
};
pub use message::{
    FileUpload, Markdown, MarkdownParams, Media, MediaResponse, MessageReference, MessageResponse,
    MessageToCreate, file_type, msg_type,
};

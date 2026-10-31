//! QQ 侧 DTO。
//!
//! - [`message`]：发送 / 上传 / 响应结构（`botgo/dto/message_create.go`）。
//! - [`group`]：群管理结构（`botgo/dto/group.go`、`group_member.go`）。
//! - [`media_image`]：素材管理结构（官方 2026-10 素材管理端点族）。

pub mod group;
pub mod interaction;
pub mod media_image;
pub mod message;

pub use group::{
    AutoApprovedInfo, BotInGroupState, GlobalMuteRule, GroupInfo, GroupMember, GroupMemberList,
    JoinRequest, JoinRequestList, MemberMuteState, MuteRecurringRule, MuteScheduleRule,
    RestrictChatSetting, ReviewQa, VerifyInfo,
};
pub use media_image::{
    MAX_MEDIA_IMAGE_BYTES, MediaImage, MediaImageCompleteRequest, MediaImageCompleteResponse,
    MediaImageList, MediaImagePrepareRequest, MediaImagePrepareResponse, SUPPORTED_CONTENT_TYPES,
};
pub use message::{
    FileUpload, Markdown, MarkdownParams, Media, MediaResponse, MessageReference, MessageResponse,
    MessageToCreate, file_type, msg_type,
};

//! QQ 侧 DTO。
//!
//! - [`message`]：发送 / 上传 / 响应结构（`botgo/dto/message_create.go`）。
//! - [`group`]：群管理结构（`botgo/dto/group.go`、`group_member.go`）。
//! - [`panel`]：指令面板 / 自定义菜单（`botgo/dto/panel.go`、`menu.go`）。
//! - [`media_image`]：素材管理结构（官方 2026-10 素材管理端点族）。
//! - [`user`]：机器人自身信息（`botgo/dto/user.go`，`GET /users/@me`）。

pub mod group;
pub mod interaction;
pub mod media_image;
pub mod message;
pub mod panel;
pub mod user;

pub use group::{
    ApprovalJoinRequest, AutoApprovedInfo, BatchRemoveMembersRequest, BatchRemoveMembersResponse,
    BlacklistUser, BotInGroupState, CreateJoinApprovalStrategyRequest,
    CreateJoinApprovalStrategyResponse, GlobalMuteRule, GroupAction, GroupInfo, GroupMember,
    GroupMemberList, JoinApprovalStrategy, JoinApprovalStrategyList, JoinRequest, JoinRequestList,
    MemberBlacklistList, MemberBlacklistRequest, MemberBlacklistResponse, MemberMuteState,
    MuteRecurringRule, MuteScheduleRule, RestrictChatSetting, ReviewQa,
    UpdateJoinApprovalStrategyRequest, UpdateJoinApprovalStrategyResponse, VerifyInfo,
    WhitelistUsersRequest, WhitelistUsersResponse,
};
pub use media_image::{
    MAX_MEDIA_IMAGE_BYTES, MediaImage, MediaImageCompleteRequest, MediaImageCompleteResponse,
    MediaImageList, MediaImagePrepareRequest, MediaImagePrepareResponse, SUPPORTED_CONTENT_TYPES,
};
pub use message::{
    FileUpload, FileUploadMergeRequest, FileUploadPart, FileUploadPartFinishRequest,
    FileUploadPrepareRequest, FileUploadPrepareResponse, GenerateUrlLink, GenerateUrlLinkRequest,
    InputNotify, Markdown, MarkdownParams, Media, MediaResponse, MessageReference, MessageResponse,
    MessageToCreate, StreamSse, file_type, msg_type,
};
pub use panel::{
    CreatePanelRequest, CreatePanelResponse, Menu, MenuItem, MenuResponse, MenuSwitch, Panel,
    PanelItem, PanelList, PanelRecord, PanelRecordDetail, PanelTargetRequest, PutMenuRequest,
    SubMenuItem, UpdatePanelRequest, VersionResponse,
};
pub use user::BotUser;

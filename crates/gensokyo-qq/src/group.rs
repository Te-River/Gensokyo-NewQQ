//! 群管理 API 子集（对应 `botgo` 的 `GroupAPI`）。
//!
//! 端点取自 `botgo/openapi/v2/resource.go`；未实现的群管理端点（审批、自动审批策略、
//! 黑名单、批量移出、菜单、指令面板等）见 crate 级文档。

use crate::dto::group::{
    BotInGroupState, GroupInfo, GroupMember, GroupMemberList, JoinRequestList, RestrictChatSetting,
};
use crate::error::Result;
use crate::http::{HttpTransport, encode_path_segment};
use crate::openapi::OpenApiClient;

impl<T: HttpTransport> OpenApiClient<T> {
    /// 获取群基本信息（`GET /v2/groups/{group_id}/info`）。
    pub async fn get_group_info(&self, group_openid: &str) -> Result<GroupInfo> {
        let path = format!("/v2/groups/{}/info", encode_path_segment(group_openid));
        self.get_json(&path).await
    }

    /// 获取机器人群内状态（`GET /v2/groups/{group_id}/bot_state`）。
    pub async fn get_bot_state(&self, group_openid: &str) -> Result<BotInGroupState> {
        let path = format!("/v2/groups/{}/bot_state", encode_path_segment(group_openid));
        self.get_json(&path).await
    }

    /// 入群申请列表（`GET /v2/groups/{group_id}/join_request_list`）。
    ///
    /// `cursor` 为空串视为首页；`limit` 为 `None` 或 0 时不传，官方默认 20、上限 50。
    pub async fn get_join_request_list(
        &self,
        group_openid: &str,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<JoinRequestList> {
        let path = with_paging(
            format!(
                "/v2/groups/{}/join_request_list",
                encode_path_segment(group_openid)
            ),
            cursor,
            limit,
        );
        self.get_json(&path).await
    }

    /// 设置群成员禁言（`POST /v2/groups/{group_id}/restrict_chat_setting`）。
    ///
    /// 官方 POST 请求体只接受 `members`（按成员 `op` 增量生效）。
    pub async fn set_restrict_chat_setting(
        &self,
        group_openid: &str,
        setting: &RestrictChatSetting,
    ) -> Result<()> {
        let path = format!(
            "/v2/groups/{}/restrict_chat_setting",
            encode_path_segment(group_openid)
        );
        self.post_json_void(&path, setting).await
    }

    /// 群成员列表（`GET /v2/groups/{group_id}/members`），游标分页。
    ///
    /// `cursor` 为 `None` / 空串取首页；响应里的 `next_cursor` 为末页时是空串。
    pub async fn get_group_members(
        &self,
        group_openid: &str,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<GroupMemberList> {
        let path = with_paging(
            format!("/v2/groups/{}/members", encode_path_segment(group_openid)),
            cursor,
            limit,
        );
        self.get_json(&path).await
    }

    /// 单个群成员信息（`GET /v2/groups/{group_id}/members/{member_openid}`）。
    pub async fn get_member_info(
        &self,
        group_openid: &str,
        member_openid: &str,
    ) -> Result<GroupMember> {
        let path = format!(
            "/v2/groups/{}/members/{}",
            encode_path_segment(group_openid),
            encode_path_segment(member_openid)
        );
        self.get_json(&path).await
    }
}

/// 追加 `cursor` / `limit` 查询参数（`cursor` 空串与 `limit = 0` 都不传）。
fn with_paging(mut path: String, cursor: Option<&str>, limit: Option<u32>) -> String {
    let mut query: Vec<String> = Vec::new();
    if let Some(cursor) = cursor.filter(|cursor| !cursor.is_empty()) {
        query.push(format!("cursor={}", encode_path_segment(cursor)));
    }
    if let Some(limit) = limit.filter(|limit| *limit > 0) {
        query.push(format!("limit={limit}"));
    }
    if !query.is_empty() {
        path.push('?');
        path.push_str(&query.join("&"));
    }
    path
}

#[cfg(test)]
mod tests {
    use super::with_paging;

    #[test]
    fn members_path_forwards_cursor_and_limit() {
        assert_eq!(
            with_paging("/v2/groups/G/members".to_owned(), Some("nc"), Some(30)),
            "/v2/groups/G/members?cursor=nc&limit=30"
        );
    }

    #[test]
    fn first_page_omits_empty_cursor_and_zero_limit() {
        assert_eq!(
            with_paging("/v2/groups/G/members".to_owned(), None, Some(30)),
            "/v2/groups/G/members?limit=30"
        );
        assert_eq!(
            with_paging("/v2/groups/G/members".to_owned(), Some(""), Some(0)),
            "/v2/groups/G/members"
        );
    }
}

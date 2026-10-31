//! 素材管理 action：`upload_media_image` / `get_media_image_list` / `delete_media_image`。
//!
//! # 覆盖范围
//!
//! | action | Go 对应 | 上游端点 |
//! |--------|---------|----------|
//! | `upload_media_image` | `handlers/media_images.go` | `POST /v2/media-images/prepare` → `PUT <presigned_url>` → `POST /v2/media-images/complete` |
//! | `get_media_image_list` | 同上 | `GET /v2/media-images?cursor=` |
//! | `delete_media_image` | 同上 | `DELETE /v2/media-images/{image_id}/delete` |
//!
//! # `file` 形态
//!
//! 与 Go 侧同口径：`base64://`、`data:<mime>;base64,`、`file://`、本地路径。**`http(s)://` 一律
//! 拒绝**——prepare 要求提交内容真实的 md5 / sha1，远端下载既违背「本地字节」语义，也会让官方
//! 校验在 complete 阶段炸成 40035003。
//!
//! # retcode
//!
//! - 参数 / 校验 / 本地读取失败 → `1400`（[`crate::handler::RETCODE_BAD_REQUEST`]）；
//! - 上游业务错误（官方 40035001~40035010）→ `1401`，`message` 保留 `code=<原码>` 文本，不吞成
//!   `1500`：客户端要能区分「格式不支持」「超配额」「图片已删除」；
//! - 传输层失败 / 端点未落地 → `1500`。

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use gensokyo_core::{Error, Result};
use gensokyo_protocol::action::{ActionRequest, ActionResponse};
use gensokyo_protocol::{Handler, HandlerFuture};
use gensokyo_qq::dto::media_image::{
    MediaImage, MediaImageCompleteRequest, MediaImagePrepareRequest,
};
use gensokyo_qq::media_image::{
    content_type_for_extension, extension_for_content_type, extension_of, md5_hex, sha1_hex,
    validate_image_upload,
};

use crate::params;
use crate::qqapi::ActionContext;
use crate::util::{bad_request, failed, ok_data, params_object, upstream_error};

/// 素材管理上游错误（官方 40035001~40035010）的 OneBot retcode。
///
/// 与查询类 action 的 `1500` 不同：素材管理的错误码是「可决策」的（换格式、删图、等额度），
/// 客户端需要看见原码。
pub const RETCODE_MEDIA_UPSTREAM: i32 = 1401;

/// `upload_media_image` 的响应数据（字段与 Go `MediaImageUpload` 对齐）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadMediaImageData {
    /// 图片 ID（查询 / 删除用）。
    pub image_id: String,
    /// 素材引用 `qqbot-media://image/v1/<token>`，可原样写进 Markdown。
    pub url: String,
    /// 有效期（秒），官方 7_776_000 = 90 天滑动窗口。
    pub ttl: u32,
    /// COS 预签名短时效链接；**禁止持久化**。
    pub preview_url: String,
    /// 实际使用的文件名（缺省时由路径或 `content_type` 推导）。
    pub file_name: String,
    /// 预签名 URL 有效期（秒）。
    pub expires_in: u32,
}

/// `get_media_image_list` 的响应数据。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaImageListData {
    /// 素材列表，按上传时间倒序。
    pub images: Vec<MediaImage>,
    /// 下一页游标；空串表示已到末页。
    pub next_cursor: String,
    /// 已上传素材总数（与 100 张配额同口径）。
    pub total: u32,
    /// `next_cursor` 非空即为 `true`（OneBot 客户端惯用的布尔翻页信号）。
    pub next: bool,
}

/// `delete_media_image` 的响应数据（官方成功回空 JSON 对象）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteMediaImageData {}

/// 一次上传的本地准备结果。
struct UploadPlan {
    /// 文件字节（PUT 直传用）。
    bytes: Vec<u8>,
    /// prepare 请求体（含本地计算的 md5 / sha1）。
    prepare: MediaImagePrepareRequest,
}

/// 解析 `params` 并读出字节、归一 `file_name` / `content_type`、算摘要。
///
/// 校验顺序刻意是「读字节 → 体积 / 格式 / 扩展名 → 摘要」：超限与格式错误在算摘要之前就返回，
/// 10MB 的非法文件不必白算一遍哈希。
fn plan_upload(object: &Map<String, Value>) -> Result<UploadPlan> {
    let file = params::optional_text(object, "file")?
        .ok_or_else(|| Error::Protocol("缺少 file".to_owned()))?;
    let source = read_upload_source(&file)?;
    let content_type = resolve_content_type(object, &source)?;
    let file_name = resolve_file_name(object, &source, &content_type)?;
    validate_image_upload(&file_name, &content_type, source.bytes.len() as u64)
        .map_err(local_error)?;

    let prepare = MediaImagePrepareRequest {
        file_size: source.bytes.len() as u64,
        md5: md5_hex(&source.bytes),
        sha1: sha1_hex(&source.bytes),
        file_name,
        content_type,
    };
    Ok(UploadPlan {
        bytes: source.bytes,
        prepare,
    })
}

/// `file` 的字节来源。
struct UploadSource {
    /// 文件内容。
    bytes: Vec<u8>,
    /// `data:` URI 里声明的 MIME（其余形态为 `None`）。
    content_type_hint: Option<String>,
    /// 本地路径的末段文件名（其余形态为 `None`）。
    file_name_hint: Option<String>,
}

/// 按 OneBot 的四种 `file` 形态取字节；`http(s)://` 明确拒绝。
fn read_upload_source(file: &str) -> Result<UploadSource> {
    if file.starts_with("http://") || file.starts_with("https://") {
        return Err(Error::Protocol(
            "file 不接受 http(s) URL：素材上传需要本地可计算 md5/sha1 的内容".to_owned(),
        ));
    }
    if let Some(payload) = file.strip_prefix("base64://") {
        let bytes =
            decode_base64(payload).ok_or_else(|| Error::Protocol("base64 内容非法".to_owned()))?;
        return Ok(UploadSource {
            bytes,
            content_type_hint: None,
            file_name_hint: None,
        });
    }
    if let Some(rest) = file.strip_prefix("data:") {
        let (meta, payload) = rest
            .split_once(',')
            .ok_or_else(|| Error::Protocol("data URI 缺少逗号分隔".to_owned()))?;
        let mime = meta
            .strip_suffix(";base64")
            .ok_or_else(|| Error::Protocol("data URI 只支持 base64 编码".to_owned()))?;
        let bytes =
            decode_base64(payload).ok_or_else(|| Error::Protocol("base64 内容非法".to_owned()))?;
        return Ok(UploadSource {
            bytes,
            content_type_hint: Some(mime.trim().to_ascii_lowercase())
                .filter(|mime| !mime.is_empty()),
            file_name_hint: None,
        });
    }

    let path = normalize_local_path(file);
    let bytes = std::fs::read(&path)
        .map_err(|err| Error::Protocol(format!("读取本地文件失败(1404): {path}: {err}")))?;
    let file_name_hint = path
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .map(str::to_owned);
    Ok(UploadSource {
        bytes,
        content_type_hint: None,
        file_name_hint,
    })
}

/// `file:///C:/a.png` → `C:/a.png`；POSIX 的 `/home/a.png` 保持原样。
fn normalize_local_path(file: &str) -> String {
    let path = file.strip_prefix("file://").map_or(file, |rest| rest);
    let bytes = path.as_bytes();
    let is_drive_letter =
        bytes.len() >= 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b':';
    if is_drive_letter {
        path[1..].to_owned()
    } else {
        path.to_owned()
    }
}

/// `content_type` 归一：显式参数 > `data:` URI 的 MIME > 文件名扩展名。
fn resolve_content_type(object: &Map<String, Value>, source: &UploadSource) -> Result<String> {
    if let Some(raw) = params::optional_text(object, "content_type")? {
        return Ok(raw.trim().to_ascii_lowercase());
    }
    if let Some(hint) = &source.content_type_hint {
        return Ok(hint.clone());
    }
    let file_name = params::optional_text(object, "file_name")?;
    let name = file_name.as_deref().or(source.file_name_hint.as_deref());
    let extension = name
        .and_then(extension_of)
        .ok_or_else(|| Error::Protocol("缺少 content_type 且无法从文件名推导".to_owned()))?;
    content_type_for_extension(extension)
        .map(str::to_owned)
        .ok_or_else(|| Error::Protocol(format!("不支持的图片扩展名: .{extension}")))
}

/// `file_name` 归一：显式参数 > 本地路径末段 > 按 `content_type` 拼默认名。
fn resolve_file_name(
    object: &Map<String, Value>,
    source: &UploadSource,
    content_type: &str,
) -> Result<String> {
    if let Some(raw) = params::optional_text(object, "file_name")? {
        return Ok(raw);
    }
    if let Some(hint) = &source.file_name_hint {
        return Ok(hint.clone());
    }
    let extension = extension_for_content_type(content_type).unwrap_or("png");
    Ok(format!("upload.{extension}"))
}

/// `gensokyo-qq` 的本地校验错误 → 参数错误（`1400`）；其余仍按上游 / 传输处理。
fn local_error(err: gensokyo_qq::Error) -> Error {
    match err {
        gensokyo_qq::Error::InvalidMessage(message)
        | gensokyo_qq::Error::InvalidResponse(message) => Error::Protocol(message),
        other => Error::Network(other.to_string()),
    }
}

/// 上游业务错误 → `1401`（保留官方原码），其余错误沿用 `1500`。
fn media_error(request: &ActionRequest<Value>, err: &Error) -> ActionResponse<Value> {
    match err {
        Error::Upstream { .. } => failed(request, RETCODE_MEDIA_UPSTREAM, &err.to_string()),
        _ => upstream_error(request, err),
    }
}

/// `upload_media_image`：prepare → PUT 直传 → complete。
#[derive(Debug, Clone)]
pub struct UploadMediaImageHandler {
    context: Arc<ActionContext>,
}

impl UploadMediaImageHandler {
    /// 构造（共享 [`ActionContext`]）。
    #[must_use]
    pub fn new(context: Arc<ActionContext>) -> Self {
        Self { context }
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        let object = match params_object(request) {
            Ok(object) => object,
            Err(err) => return Ok(bad_request(request, &err)),
        };
        let plan = match plan_upload(object) {
            Ok(plan) => plan,
            Err(err) => return Ok(bad_request(request, &err)),
        };

        let api = self.context.api();
        let prepared = match api.prepare_media_image(plan.prepare.clone()).await {
            Ok(prepared) => prepared,
            Err(err) => return Ok(media_error(request, &err)),
        };
        let md5 = match api
            .put_presigned_image(
                &prepared.presigned_url,
                &plan.prepare.content_type,
                plan.bytes,
            )
            .await
        {
            Ok(md5) => md5,
            Err(err) => return Ok(media_error(request, &err)),
        };
        let completed = match api
            .complete_media_image(MediaImageCompleteRequest {
                upload_id: prepared.upload_id,
                md5,
            })
            .await
        {
            Ok(completed) => completed,
            Err(err) => return Ok(media_error(request, &err)),
        };

        let data = UploadMediaImageData {
            image_id: completed.image_id,
            url: completed.url,
            ttl: completed.ttl,
            preview_url: completed.preview_url,
            file_name: plan.prepare.file_name,
            expires_in: prepared.expires_in,
        };
        ok_data(request, &data)
    }
}

impl Handler for UploadMediaImageHandler {
    fn name(&self) -> &'static str {
        "upload_media_image"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `get_media_image_list`：`cursor` 可选，空串视为首页。
#[derive(Debug, Clone)]
pub struct GetMediaImageListHandler {
    context: Arc<ActionContext>,
}

impl GetMediaImageListHandler {
    /// 构造（共享 [`ActionContext`]）。
    #[must_use]
    pub fn new(context: Arc<ActionContext>) -> Self {
        Self { context }
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        let object = match params_object(request) {
            Ok(object) => object,
            Err(err) => return Ok(bad_request(request, &err)),
        };
        let cursor = match params::optional_text(object, "cursor") {
            Ok(cursor) => cursor,
            Err(err) => return Ok(bad_request(request, &err)),
        };
        match self
            .context
            .api()
            .list_media_images(cursor.as_deref())
            .await
        {
            Ok(list) => {
                let data = MediaImageListData {
                    next: !list.next_cursor.is_empty(),
                    images: list.images,
                    next_cursor: list.next_cursor,
                    total: list.total,
                };
                ok_data(request, &data)
            }
            Err(err) => Ok(media_error(request, &err)),
        }
    }
}

impl Handler for GetMediaImageListHandler {
    fn name(&self) -> &'static str {
        "get_media_image_list"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `delete_media_image`：`image_id` 必填。
#[derive(Debug, Clone)]
pub struct DeleteMediaImageHandler {
    context: Arc<ActionContext>,
}

impl DeleteMediaImageHandler {
    /// 构造（共享 [`ActionContext`]）。
    #[must_use]
    pub fn new(context: Arc<ActionContext>) -> Self {
        Self { context }
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        let object = match params_object(request) {
            Ok(object) => object,
            Err(err) => return Ok(bad_request(request, &err)),
        };
        let image_id = match params::optional_text(object, "image_id") {
            Ok(Some(image_id)) => image_id,
            Ok(None) => {
                return Ok(bad_request(
                    request,
                    &Error::Protocol("缺少 image_id".to_owned()),
                ));
            }
            Err(err) => return Ok(bad_request(request, &err)),
        };
        match self.context.api().delete_media_image(&image_id).await {
            Ok(()) => ok_data(request, &DeleteMediaImageData::default()),
            Err(err) => Ok(media_error(request, &err)),
        }
    }
}

impl Handler for DeleteMediaImageHandler {
    fn name(&self) -> &'static str {
        "delete_media_image"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// 宽松 base64 解码：忽略空白与填充，同时接受标准与 URL-safe alphabet。
///
/// `gensokyo_protocol::base64` 是 `pub(crate)`，本包不改 protocol，因此自带这一份；只用于
/// OneBot 客户端给的 `base64://` / `data:` 载荷。
fn decode_base64(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() / 4 * 3);
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for byte in input.bytes() {
        if byte.is_ascii_whitespace() || byte == b'=' {
            continue;
        }
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::{Value, json};

    use super::{
        DeleteMediaImageHandler, GetMediaImageListHandler, RETCODE_MEDIA_UPSTREAM,
        UploadMediaImageHandler, decode_base64, normalize_local_path,
    };
    use crate::handler::RETCODE_BAD_REQUEST;
    use crate::qqapi::{ActionContext, QqApi, QqFuture};
    use crate::testing::{block_on, config, failed, ok_data, request};
    use crate::util::RETCODE_UPSTREAM;
    use gensokyo_core::{Error, Result};
    use gensokyo_protocol::Handler;
    use gensokyo_protocol::Status;
    use gensokyo_qq::dto::media_image::{
        MediaImage, MediaImageCompleteRequest, MediaImageCompleteResponse, MediaImageList,
        MediaImagePrepareRequest, MediaImagePrepareResponse,
    };

    /// 只实现素材管理、其余端点报未实现的替身。
    #[derive(Debug, Default)]
    struct MediaFake {
        prepare:
            std::sync::Mutex<Option<(MediaImagePrepareRequest, Result<MediaImagePrepareResponse>)>>,
        put: std::sync::Mutex<Option<(String, String, Result<String>)>>,
        complete: std::sync::Mutex<
            Option<(
                MediaImageCompleteRequest,
                Result<MediaImageCompleteResponse>,
            )>,
        >,
        list: std::sync::Mutex<Option<(Option<String>, Result<MediaImageList>)>>,
        delete: std::sync::Mutex<Option<(String, Result<()>)>>,
    }

    fn unsupported<T>(method: &str) -> Result<T> {
        Err(Error::Unsupported(format!("MediaFake 未提供 {method}")))
    }

    impl QqApi for MediaFake {
        fn get_group_info(&self, _group_openid: &str) -> QqFuture<'_, gensokyo_qq::GroupInfo> {
            Box::pin(async { unsupported("get_group_info") })
        }

        fn get_group_member_info(
            &self,
            _group_openid: &str,
            _member_openid: &str,
        ) -> QqFuture<'_, gensokyo_qq::GroupMember> {
            Box::pin(async { unsupported("get_group_member_info") })
        }

        fn get_group_member_list(
            &self,
            _group_openid: &str,
            _limit: u32,
        ) -> QqFuture<'_, gensokyo_qq::GroupMemberList> {
            Box::pin(async { unsupported("get_group_member_list") })
        }

        fn set_group_ban(
            &self,
            _group_openid: &str,
            _member_openid: &str,
            _duration_secs: i64,
        ) -> QqFuture<'_, ()> {
            Box::pin(async { unsupported("set_group_ban") })
        }

        fn get_group_bot_state(
            &self,
            _group_openid: &str,
        ) -> QqFuture<'_, gensokyo_qq::BotInGroupState> {
            Box::pin(async { unsupported("get_group_bot_state") })
        }

        fn get_group_join_request_list(
            &self,
            _group_openid: &str,
            _cursor: Option<&str>,
            _limit: Option<u32>,
        ) -> QqFuture<'_, gensokyo_qq::JoinRequestList> {
            Box::pin(async { unsupported("get_group_join_request_list") })
        }

        fn send_group_message(
            &self,
            _group_openid: String,
            _msg: gensokyo_qq::dto::MessageToCreate,
        ) -> QqFuture<'_, gensokyo_qq::dto::MessageResponse> {
            Box::pin(async { unsupported("send_group_message") })
        }

        fn send_c2c_message(
            &self,
            _user_openid: String,
            _msg: gensokyo_qq::dto::MessageToCreate,
        ) -> QqFuture<'_, gensokyo_qq::dto::MessageResponse> {
            Box::pin(async { unsupported("send_c2c_message") })
        }

        fn prepare_media_image(
            &self,
            request: MediaImagePrepareRequest,
        ) -> QqFuture<'_, MediaImagePrepareResponse> {
            let slot = self.prepare.lock().expect("槽未中毒").take();
            Box::pin(async move {
                let (recorded, result) = slot.expect("测试未配置 prepare 结果");
                assert_eq!(recorded, request, "prepare 请求体应与本地计划一致");
                result
            })
        }

        fn put_presigned_image(
            &self,
            presigned_url: &str,
            content_type: &str,
            bytes: Vec<u8>,
        ) -> QqFuture<'_, String> {
            let slot = self.put.lock().expect("槽未中毒").take();
            let presigned_url = presigned_url.to_owned();
            let content_type = content_type.to_owned();
            Box::pin(async move {
                let (expected_url, expected_type, result) = slot.expect("测试未配置 put 结果");
                assert_eq!(expected_url, presigned_url);
                assert_eq!(expected_type, content_type);
                assert!(!bytes.is_empty(), "直传字节不应为空");
                result
            })
        }

        fn complete_media_image(
            &self,
            request: MediaImageCompleteRequest,
        ) -> QqFuture<'_, MediaImageCompleteResponse> {
            let slot = self.complete.lock().expect("槽未中毒").take();
            Box::pin(async move {
                let (recorded, result) = slot.expect("测试未配置 complete 结果");
                assert_eq!(recorded, request, "complete 应收到去引号后的 md5");
                result
            })
        }

        fn list_media_images(&self, cursor: Option<&str>) -> QqFuture<'_, MediaImageList> {
            let slot = self.list.lock().expect("槽未中毒").take();
            let cursor = cursor.map(str::to_owned);
            Box::pin(async move {
                let (recorded, result) = slot.expect("测试未配置 list 结果");
                assert_eq!(recorded, cursor);
                result
            })
        }

        fn delete_media_image(&self, image_id: &str) -> QqFuture<'_, ()> {
            let slot = self.delete.lock().expect("槽未中毒").take();
            let image_id = image_id.to_owned();
            Box::pin(async move {
                let (recorded, result) = slot.expect("测试未配置 delete 结果");
                assert_eq!(recorded, image_id);
                result
            })
        }
    }

    fn context(fake: Arc<MediaFake>) -> Arc<ActionContext> {
        Arc::new(ActionContext::new(fake, config()))
    }

    const PNG_1PX: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

    #[test]
    fn upload_runs_prepare_put_complete_in_order() {
        let fake = Arc::new(MediaFake::default());
        let file = format!("base64://{PNG_1PX}");
        let bytes = decode_base64(PNG_1PX).expect("测试向量应可解码");
        let md5 = gensokyo_qq::media_image::md5_hex(&bytes);
        let sha1 = gensokyo_qq::media_image::sha1_hex(&bytes);
        let prepare_request = MediaImagePrepareRequest {
            file_name: "upload.png".to_owned(),
            file_size: bytes.len() as u64,
            content_type: "image/png".to_owned(),
            md5,
            sha1,
        };
        *fake.prepare.lock().expect("槽未中毒") = Some((
            prepare_request,
            Ok(MediaImagePrepareResponse {
                upload_id: "up-1".to_owned(),
                presigned_url: "https://cos.example.com/x".to_owned(),
                expires_in: 3600,
            }),
        ));
        *fake.put.lock().expect("槽未中毒") = Some((
            "https://cos.example.com/x".to_owned(),
            "image/png".to_owned(),
            Ok("etag-md5".to_owned()),
        ));
        *fake.complete.lock().expect("槽未中毒") = Some((
            MediaImageCompleteRequest {
                upload_id: "up-1".to_owned(),
                md5: "etag-md5".to_owned(),
            },
            Ok(MediaImageCompleteResponse {
                image_id: "img-1".to_owned(),
                url: "qqbot-media://image/v1/token".to_owned(),
                ttl: 7_776_000,
                preview_url: "https://cos.example.com/p".to_owned(),
            }),
        ));

        let handler = UploadMediaImageHandler::new(context(Arc::clone(&fake)));
        let response = block_on(handler.handle(&request(
            "upload_media_image",
            json!({"file": file, "content_type": "image/png"}),
        )))
        .expect("本地替身不应失败");
        let data = ok_data(response);
        assert_eq!(data["image_id"], json!("img-1"));
        assert_eq!(data["url"], json!("qqbot-media://image/v1/token"));
        assert_eq!(data["ttl"], json!(7_776_000));
        assert_eq!(data["preview_url"], json!("https://cos.example.com/p"));
        assert_eq!(data["file_name"], json!("upload.png"));
        assert_eq!(data["expires_in"], json!(3600));
        assert_eq!(handler.name(), "upload_media_image");
    }

    #[test]
    fn oversized_file_fails_with_1400_before_any_network() {
        let path =
            std::env::temp_dir().join(format!("gensokyo-media-image-{}.png", std::process::id()));
        std::fs::write(&path, vec![0_u8; 10_000_001]).expect("写临时文件失败");
        let fake = Arc::new(MediaFake::default());
        let handler = UploadMediaImageHandler::new(context(fake));
        let response = block_on(handler.handle(&request(
            "upload_media_image",
            json!({"file": path.to_string_lossy()}),
        )))
        .expect("校验失败也应是 Ok(失败响应)");
        let _ = std::fs::remove_file(&path);
        let (status, retcode, data) = failed(response);
        assert_eq!(status, Status::Failed);
        assert_eq!(retcode, RETCODE_BAD_REQUEST);
        assert_eq!(data, None);
    }

    #[test]
    fn invalid_file_forms_are_rejected() {
        let fake = Arc::new(MediaFake::default());
        let handler = UploadMediaImageHandler::new(context(fake));
        for params in [
            json!({}),
            json!({"file": ""}),
            json!({"file": "https://example.com/a.png"}),
            json!({"file": format!("base64://{PNG_1PX}"), "content_type": "image/bmp"}),
            json!({"file": format!("base64://{PNG_1PX}"), "content_type": "image/png", "file_name": "a.jpg"}),
            json!({"file": format!("base64://{PNG_1PX}"), "content_type": "image/png", "file_name": "noext"}),
        ] {
            let response = block_on(handler.handle(&request("upload_media_image", params.clone())))
                .expect("参数错误也应是 Ok(失败响应)");
            let (_, retcode, _) = failed(response);
            assert_eq!(retcode, RETCODE_BAD_REQUEST, "{params}");
        }
    }

    #[test]
    fn complete_error_40035003_maps_to_1401_with_original_code() {
        let fake = Arc::new(MediaFake::default());
        let bytes = decode_base64(PNG_1PX).expect("测试向量应可解码");
        *fake.prepare.lock().expect("槽未中毒") = Some((
            MediaImagePrepareRequest {
                file_name: "upload.png".to_owned(),
                file_size: bytes.len() as u64,
                content_type: "image/png".to_owned(),
                md5: gensokyo_qq::media_image::md5_hex(&bytes),
                sha1: gensokyo_qq::media_image::sha1_hex(&bytes),
            },
            Ok(MediaImagePrepareResponse {
                upload_id: "up-1".to_owned(),
                presigned_url: "https://cos.example.com/x".to_owned(),
                expires_in: 3600,
            }),
        ));
        *fake.put.lock().expect("槽未中毒") = Some((
            "https://cos.example.com/x".to_owned(),
            "image/png".to_owned(),
            Ok("etag-md5".to_owned()),
        ));
        *fake.complete.lock().expect("槽未中毒") = Some((
            MediaImageCompleteRequest {
                upload_id: "up-1".to_owned(),
                md5: "etag-md5".to_owned(),
            },
            Err(Error::Upstream {
                code: 40_035_003,
                message: "上传任务不存在或已过期".to_owned(),
            }),
        ));

        let handler = UploadMediaImageHandler::new(context(fake));
        let response = block_on(handler.handle(&request(
            "upload_media_image",
            json!({"file": format!("base64://{PNG_1PX}"), "content_type": "image/png"}),
        )))
        .expect("上游错误也应是 Ok(失败响应)");
        let (status, retcode, data) = failed(response);
        assert_eq!(status, Status::Failed);
        assert_eq!(retcode, RETCODE_MEDIA_UPSTREAM, "不得吞成 1500");
        assert_eq!(data, None);
    }

    #[test]
    fn delete_error_40035008_maps_to_1401() {
        let fake = Arc::new(MediaFake::default());
        *fake.delete.lock().expect("槽未中毒") = Some((
            "img-404".to_owned(),
            Err(Error::Upstream {
                code: 40_035_008,
                message: "图片不存在或已被删除".to_owned(),
            }),
        ));
        let handler = DeleteMediaImageHandler::new(context(fake));
        let response = block_on(handler.handle(&request(
            "delete_media_image",
            json!({"image_id": "img-404"}),
        )))
        .expect("上游错误也应是 Ok(失败响应)");
        let (_, retcode, _) = failed(response);
        assert_eq!(retcode, RETCODE_MEDIA_UPSTREAM);

        let missing = DeleteMediaImageHandler::new(context(Arc::new(MediaFake::default())));
        let response = block_on(missing.handle(&request("delete_media_image", json!({}))))
            .expect("缺参也应是 Ok(失败响应)");
        let (_, retcode, _) = failed(response);
        assert_eq!(retcode, RETCODE_BAD_REQUEST);
    }

    #[test]
    fn empty_list_does_not_panic_and_delete_returns_empty_data() {
        let fake = Arc::new(MediaFake::default());
        *fake.list.lock().expect("槽未中毒") = Some((
            None,
            Ok(MediaImageList {
                images: vec![MediaImage {
                    image_id: "img-1".to_owned(),
                    ..MediaImage::default()
                }],
                next_cursor: String::new(),
                total: 1,
            }),
        ));
        let handler = GetMediaImageListHandler::new(context(Arc::clone(&fake)));
        let response = block_on(handler.handle(&request("get_media_image_list", json!({}))))
            .expect("列表不应失败");
        let data = ok_data(response);
        assert_eq!(data["total"], json!(1));
        assert_eq!(data["next_cursor"], json!(""));
        assert_eq!(data["next"], json!(false));
        assert_eq!(data["images"][0]["image_id"], json!("img-1"));

        *fake.delete.lock().expect("槽未中毒") = Some(("img-1".to_owned(), Ok(())));
        let handler = DeleteMediaImageHandler::new(context(fake));
        let response =
            block_on(handler.handle(&request("delete_media_image", json!({"image_id": "img-1"}))))
                .expect("删除不应失败");
        let data = ok_data(response);
        assert_eq!(data, Value::Object(Default::default()));
    }

    #[test]
    fn transport_errors_keep_the_1500_bucket() {
        let fake = Arc::new(MediaFake::default());
        *fake.list.lock().expect("槽未中毒") =
            Some((None, Err(Error::Network("连接重置".to_owned()))));
        let handler = GetMediaImageListHandler::new(context(fake));
        let response = block_on(handler.handle(&request("get_media_image_list", json!({}))))
            .expect("传输错误也应是 Ok(失败响应)");
        let (_, retcode, _) = failed(response);
        assert_eq!(retcode, RETCODE_UPSTREAM);
    }

    #[test]
    fn file_uri_is_normalized_and_base64_is_lenient() {
        assert_eq!(normalize_local_path("file:///C:/tmp/a.png"), "C:/tmp/a.png");
        assert_eq!(normalize_local_path("/home/a.png"), "/home/a.png");
        assert_eq!(decode_base64("YQ==").as_deref(), Some(&b"a"[..]));
        assert_eq!(
            decode_base64("_-8=").expect("URL-safe alphabet 也应接受"),
            vec![0xff, 0xef]
        );
        assert_eq!(decode_base64("Y!"), None);
    }
}

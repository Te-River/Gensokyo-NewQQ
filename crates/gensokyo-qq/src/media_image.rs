//! 素材管理端点族（官方 2026-10）：prepare → PUT 直传 COS → complete，外加列表与删除。
//!
//! # 链路
//!
//! ```text
//! [本地字节] md5 / sha1 / size / content_type → POST /v2/media-images/prepare
//!    → upload_id（4 小时） + presigned_url + expires_in
//! [字节] → PUT presigned_url（Content-Type 必须与 prepare 一致，**不带 Authorization**）→ COS
//!    → 响应头 ETag（去引号后等于 md5）
//! POST /v2/media-images/complete 传 upload_id 与 md5
//!    → image_id + url（qqbot-media://image/v1/<token>）+ ttl 7776000 + preview_url
//! ```
//!
//! 列表走 `GET /v2/media-images?cursor=`，删除走 `DELETE /v2/media-images/{image_id}/delete`
//! （成功后引用链接立即失效，已发送的消息不受影响）。
//!
//! # 为什么 PUT / DELETE 不走 [`HttpTransport`]
//!
//! - **PUT**：预签名 URL 属 COS 域名，带 `Authorization: QQBot <token>` 会被当成越权请求
//!   （官方原文：Content-Type 与 prepare 不一致直接 403）。`botgo` 同样把 PUT 排除在 SDK 之外，
//!   这里照搬：用 reqwest 裸 PUT，60 秒超时。
//! - **DELETE**：`HttpMethod` 只有 GET / POST（`http.rs` 不在 P-R1 的文件所有权内），因此删除
//!   也用 reqwest 直连，但鉴权头与 [`OpenApiClient`] 的 `authorized` 完全一致，错误同样经
//!   [`ApiError::from_response`] 归一（业务码 40035008 等原样保留）。
//!
//! # 本地校验
//!
//! 官方限制表：单张 ≤ 10MB、png / jpeg / gif / webp、扩展名与 `content_type` 一致。校验失败一律
//! [`Error::InvalidMessage`]（handler 映射成 `retcode = 1400`，不发网络请求）。

use std::time::Duration;

use crate::dto::media_image::{
    MediaImageCompleteRequest, MediaImageCompleteResponse, MediaImageList,
    MediaImagePrepareRequest, MediaImagePrepareResponse, SUPPORTED_CONTENT_TYPES,
};
use crate::error::{ApiError, Error, Result};
use crate::http::{
    DEFAULT_CONNECT_TIMEOUT, DEFAULT_REQUEST_TIMEOUT, HttpTransport, encode_path_segment,
};
use crate::openapi::OpenApiClient;

pub use crate::dto::media_image::MAX_MEDIA_IMAGE_BYTES;

/// 预签名直传的超时（官方未规定；10MB 上行在弱网下需要比 OpenAPI 的 10 秒更宽）。
pub const PRESIGNED_UPLOAD_TIMEOUT: Duration = Duration::from_secs(60);

/// 鉴权 scheme（与 `openapi.rs` 的 `AUTH_SCHEME` 同值；那里是私有的）。
const AUTH_SCHEME: &str = "QQBot";

/// MD5 期望长度（32 位十六进制）。
const MD5_HEX_LEN: usize = 32;
/// SHA1 期望长度（40 位十六进制）。
const SHA1_HEX_LEN: usize = 40;

impl<T: HttpTransport> OpenApiClient<T> {
    /// 申请上传（`POST /v2/media-images/prepare`）。
    pub async fn prepare_media_image(
        &self,
        request: &MediaImagePrepareRequest,
    ) -> Result<MediaImagePrepareResponse> {
        validate_media_image_request(request)?;
        self.post_json("/v2/media-images/prepare", request).await
    }

    /// 完成上传（`POST /v2/media-images/complete`）。
    pub async fn complete_media_image(
        &self,
        request: &MediaImageCompleteRequest,
    ) -> Result<MediaImageCompleteResponse> {
        self.post_json("/v2/media-images/complete", request).await
    }

    /// 素材列表（`GET /v2/media-images?cursor=`）；`cursor` 为空表示首页。
    pub async fn list_media_images(&self, cursor: Option<&str>) -> Result<MediaImageList> {
        let mut path = String::from("/v2/media-images");
        if let Some(cursor) = cursor.filter(|cursor| !cursor.is_empty()) {
            path.push_str("?cursor=");
            path.push_str(&encode_path_segment(cursor));
        }
        self.get_json(&path).await
    }

    /// 删除素材（`DELETE /v2/media-images/{image_id}/delete`）。
    ///
    /// 成功后释放数量配额、引用链接立即失效；删除不存在的 `image_id` 上游回 40035008。
    pub async fn delete_media_image(&self, image_id: &str) -> Result<()> {
        let path = format!("/v2/media-images/{}/delete", encode_path_segment(image_id));
        let token = self.token_manager().token().await?;
        let request = api_client()?
            .delete(format!("{}{path}", self.base_url()))
            .header("Authorization", format!("{AUTH_SCHEME} {token}"))
            .header("X-Union-Appid", self.token_manager().config().app_id())
            .header("Accept", "application/json")
            .build()
            .map_err(|err| Error::Transport(format!("构造素材删除请求失败: {err}")))?;
        let response = api_client()?
            .execute(request)
            .await
            .map_err(|err| Error::Transport(format!("素材删除失败: {err}")))?;
        let status = response.status().as_u16();
        if (200..300).contains(&status) {
            return Ok(());
        }
        let body = response
            .bytes()
            .await
            .map_err(|err| Error::Transport(format!("读取素材删除响应失败: {err}")))?;
        Err(ApiError::from_response(status, &body, None).into())
    }

    /// 直传 COS（`PUT <presigned_url>`），返回**去引号后的 md5**，供 complete 使用。
    ///
    /// Content-Type 强制取 prepare 声明值；不带 Authorization。ETag 缺失（或为空）时回退用本地
    /// 计算的 md5 并 `warn`——COS 正常会回，缺失只可能是代理吃掉了响应头。
    pub async fn put_presigned_image(
        &self,
        presigned_url: &str,
        content_type: &str,
        bytes: &[u8],
    ) -> Result<String> {
        let local_md5 = md5_hex(bytes);
        let client = presigned_client()?;
        let request = build_presigned_request(presigned_url, content_type, bytes)?;
        let response = client
            .execute(request)
            .await
            .map_err(|err| Error::Transport(format!("预签名直传失败: {err}")))?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            let body = response
                .bytes()
                .await
                .map_err(|err| Error::Transport(format!("读取直传响应失败: {err}")))?;
            // COS 的 403 就是「Content-Type 与声明不一致」的官方信号，原样上抛。
            return Err(ApiError::from_response(status, &body, None).into());
        }
        let etag = response
            .headers()
            .get("etag")
            .and_then(|value| value.to_str().ok())
            .map(strip_etag_quotes)
            .filter(|etag| !etag.is_empty());
        match etag {
            Some(etag) => Ok(etag.to_owned()),
            None => {
                tracing::warn!(url = presigned_url, "直传响应缺少 ETag，回退本地计算的 md5");
                Ok(local_md5)
            }
        }
    }
}

/// 构造预签名 PUT 请求（纯函数，便于断言「不带 Authorization」与 Content-Type）。
///
/// 不经过 [`HttpTransport`]：`HttpMethod` 只有 GET / POST，且直传 COS 不能带 OpenAPI 的鉴权头。
///
/// # Errors
/// URL 非法或 `content_type` 不是合法请求头值时返回 [`Error::Transport`]。
pub(crate) fn build_presigned_request(
    presigned_url: &str,
    content_type: &str,
    bytes: &[u8],
) -> Result<reqwest::Request> {
    let url = reqwest::Url::parse(presigned_url)
        .map_err(|err| Error::Transport(format!("预签名 URL 非法: {err}")))?;
    let value = reqwest::header::HeaderValue::from_str(content_type)
        .map_err(|err| Error::Transport(format!("content_type 不是合法请求头: {err}")))?;
    let mut request = reqwest::Request::new(reqwest::Method::PUT, url);
    request
        .headers_mut()
        .insert(reqwest::header::CONTENT_TYPE, value);
    *request.body_mut() = Some(reqwest::Body::from(bytes.to_vec()));
    Ok(request)
}

/// 预签名直传用的 reqwest 客户端：只用连接超时 + 60 秒总超时，不带任何鉴权。
fn presigned_client() -> Result<reqwest::Client> {
    ensure_tls_provider()?;
    reqwest::Client::builder()
        .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
        .timeout(PRESIGNED_UPLOAD_TIMEOUT)
        .build()
        .map_err(|err| Error::Transport(format!("构建预签名直传客户端失败: {err}")))
}

/// 素材删除用的 reqwest 客户端（超时与 OpenAPI 请求一致）。
fn api_client() -> Result<reqwest::Client> {
    ensure_tls_provider()?;
    reqwest::Client::builder()
        .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
        .timeout(DEFAULT_REQUEST_TIMEOUT)
        .build()
        .map_err(|err| Error::Transport(format!("构建素材管理客户端失败: {err}")))
}

/// reqwest 在 `rustls-no-provider` 下缺 provider 会直接 panic；先查一次，缺就回错误。
fn ensure_tls_provider() -> Result<()> {
    if rustls::crypto::CryptoProvider::get_default().is_some() {
        return Ok(());
    }
    Err(Error::Tls(
        "未安装 rustls CryptoProvider；二进制需先安装，素材直传才能建 TLS 连接".to_owned(),
    ))
}

/// ETag → md5：去掉可选 `W/` 前缀与两端引号，顺带 trim。
#[must_use]
pub fn strip_etag_quotes(raw: &str) -> &str {
    let trimmed = raw.trim();
    let trimmed = trimmed.strip_prefix("W/").unwrap_or(trimmed);
    trimmed.trim_matches('"')
}

/// prepare 请求体的完整校验（形态 + 限制表）。
///
/// # Errors
/// 空文件、超 10MB、`content_type` 不支持、扩展名与 `content_type` 不一致、md5 / sha1 非十六进制
/// 或长度不对，均返回 [`Error::InvalidMessage`]。
pub fn validate_media_image_request(request: &MediaImagePrepareRequest) -> Result<()> {
    validate_image_upload(&request.file_name, &request.content_type, request.file_size)?;
    if !is_lower_hex(&request.md5, MD5_HEX_LEN) {
        return Err(Error::InvalidMessage(format!(
            "md5 必须是 {MD5_HEX_LEN} 位小写十六进制: {}",
            request.md5
        )));
    }
    if !is_lower_hex(&request.sha1, SHA1_HEX_LEN) {
        return Err(Error::InvalidMessage(format!(
            "sha1 必须是 {SHA1_HEX_LEN} 位小写十六进制: {}",
            request.sha1
        )));
    }
    Ok(())
}

/// 上传前的本地校验（不含摘要）：体积、`content_type`、扩展名一致性。
///
/// # Errors
/// 同 [`validate_media_image_request`] 的形态部分。
pub fn validate_image_upload(file_name: &str, content_type: &str, file_size: u64) -> Result<()> {
    if file_size == 0 {
        return Err(Error::InvalidMessage("图片内容为空".to_owned()));
    }
    if file_size > MAX_MEDIA_IMAGE_BYTES {
        return Err(Error::InvalidMessage(format!(
            "图片超过大小上限: {file_size} 字节 > {MAX_MEDIA_IMAGE_BYTES} 字节（10MB）"
        )));
    }
    if !SUPPORTED_CONTENT_TYPES.contains(&content_type) {
        return Err(Error::InvalidMessage(format!(
            "不支持的 content_type: {content_type}（只接受 {}）",
            SUPPORTED_CONTENT_TYPES.join(" / ")
        )));
    }
    let extension = extension_of(file_name)
        .ok_or_else(|| Error::InvalidMessage(format!("file_name 缺少扩展名: {file_name}")))?;
    let expected = content_type_for_extension(extension).ok_or_else(|| {
        Error::InvalidMessage(format!(
            "不支持的图片扩展名: .{extension}（只接受 png / jpg / jpeg / gif / webp）"
        ))
    })?;
    if expected != content_type {
        return Err(Error::InvalidMessage(format!(
            "扩展名与 content_type 不一致: .{extension} 应为 {expected}，实际 {content_type}"
        )));
    }
    Ok(())
}

/// 取文件名的扩展名（不含点，保留原大小写；只看最后一个路径段）。
#[must_use]
pub fn extension_of(file_name: &str) -> Option<&str> {
    let name = file_name.rsplit(['/', '\\']).next()?;
    match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() && !extension.is_empty() => Some(extension),
        _ => None,
    }
}

/// 扩展名 → 官方 `content_type`（大小写不敏感）。
#[must_use]
pub fn content_type_for_extension(extension: &str) -> Option<&'static str> {
    match extension.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

/// `content_type` → 缺省扩展名（action 层在没给 `file_name` 时拼默认名）。
#[must_use]
pub fn extension_for_content_type(content_type: &str) -> Option<&'static str> {
    match content_type {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/gif" => Some("gif"),
        "image/webp" => Some("webp"),
        _ => None,
    }
}

/// 是否为指定长度的小写十六进制（官方要求 32 / 40 位十六进制，上游按原串比对）。
fn is_lower_hex(raw: &str, len: usize) -> bool {
    raw.len() == len
        && raw
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// MD5（RFC 1321），输出 32 位小写十六进制。
///
/// workspace 没有 md5 / sha1 依赖（也不允许为本包新增依赖），官方又要求 prepare 带上内容的
/// 真实摘要，因此在这里自带实现；只服务于 upload 链路，不做流式 / 大文件优化。
#[must_use]
pub fn md5_hex(input: &[u8]) -> String {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    const K: [u32; 64] = [
        0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613,
        0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193,
        0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d,
        0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
        0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122,
        0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa,
        0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244,
        0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
        0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
        0xeb86d391,
    ];

    let mut state: [u32; 4] = [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];
    let mut message = input.to_vec();
    let bit_len = (input.len() as u64).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_le_bytes());

    for chunk in message.as_chunks::<64>().0 {
        let mut words = [0_u32; 16];
        for (index, word) in words.iter_mut().enumerate() {
            let offset = index * 4;
            *word = u32::from_le_bytes([
                chunk[offset],
                chunk[offset + 1],
                chunk[offset + 2],
                chunk[offset + 3],
            ]);
        }
        let [mut a, mut b, mut c, mut d] = state;
        for round in 0..64 {
            let (mixed, word_index) = match round / 16 {
                0 => ((b & c) | (!b & d), round),
                1 => ((d & b) | (!d & c), (5 * round + 1) % 16),
                2 => (b ^ c ^ d, (3 * round + 5) % 16),
                _ => (c ^ (b | !d), (7 * round) % 16),
            };
            let rotated = a
                .wrapping_add(mixed)
                .wrapping_add(K[round])
                .wrapping_add(words[word_index])
                .rotate_left(S[round]);
            let previous_d = d;
            d = c;
            c = b;
            b = b.wrapping_add(rotated);
            a = previous_d;
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
    }

    let mut bytes = [0_u8; 16];
    for (index, word) in state.iter().enumerate() {
        bytes[index * 4..index * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    hex(&bytes)
}

/// SHA1（RFC 3174），输出 40 位小写十六进制。
#[must_use]
pub fn sha1_hex(input: &[u8]) -> String {
    let mut state: [u32; 5] = [
        0x6745_2301,
        0xefcd_ab89,
        0x98ba_dcfe,
        0x1032_5476,
        0xc3d2_e1f0,
    ];
    let mut message = input.to_vec();
    let bit_len = (input.len() as u64).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in message.as_chunks::<64>().0 {
        let mut words = [0_u32; 80];
        for (index, word) in words.iter_mut().take(16).enumerate() {
            let offset = index * 4;
            *word = u32::from_be_bytes([
                chunk[offset],
                chunk[offset + 1],
                chunk[offset + 2],
                chunk[offset + 3],
            ]);
        }
        for index in 16..80 {
            words[index] =
                (words[index - 3] ^ words[index - 8] ^ words[index - 14] ^ words[index - 16])
                    .rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = state;
        for (index, word) in words.iter().enumerate() {
            let (mixed, constant) = match index / 20 {
                0 => ((b & c) | (!b & d), 0x5a82_7999),
                1 => (b ^ c ^ d, 0x6ed9_eba1),
                2 => ((b & c) | (b & d) | (c & d), 0x8f1b_bcdc),
                _ => (b ^ c ^ d, 0xca62_c1d6),
            };
            let rotated = a
                .rotate_left(5)
                .wrapping_add(mixed)
                .wrapping_add(e)
                .wrapping_add(constant)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = rotated;
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
    }

    let mut bytes = [0_u8; 20];
    for (index, word) in state.iter().enumerate() {
        bytes[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    hex(&bytes)
}

/// 字节 → 小写十六进制。
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_MEDIA_IMAGE_BYTES, build_presigned_request, content_type_for_extension, extension_of,
        md5_hex, sha1_hex, strip_etag_quotes, validate_image_upload, validate_media_image_request,
    };
    use crate::dto::media_image::MediaImagePrepareRequest;

    fn prepare(file_name: &str, content_type: &str, file_size: u64) -> MediaImagePrepareRequest {
        MediaImagePrepareRequest {
            file_name: file_name.to_owned(),
            file_size,
            content_type: content_type.to_owned(),
            md5: "0".repeat(32),
            sha1: "0".repeat(40),
        }
    }

    #[test]
    fn md5_matches_rfc1321_vectors() {
        assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(md5_hex(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            md5_hex(b"message digest"),
            "f96b697d7cb7938d525a2f31aaf161d0"
        );
        assert_eq!(
            md5_hex(b"abcdefghijklmnopqrstuvwxyz"),
            "c3fcd3d76192e4007dfb496cca67e13b"
        );
    }

    #[test]
    fn sha1_matches_rfc3174_vectors() {
        assert_eq!(sha1_hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            sha1_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
    }

    #[test]
    fn size_limit_is_exactly_ten_million_decimal() {
        assert!(validate_image_upload("a.png", "image/png", MAX_MEDIA_IMAGE_BYTES).is_ok());
        let error = validate_image_upload("a.png", "image/png", MAX_MEDIA_IMAGE_BYTES + 1)
            .expect_err("10,000,001 字节必须拒绝");
        assert!(error.to_string().contains("大小上限"), "{error}");
        assert!(
            validate_image_upload("a.png", "image/png", 0).is_err(),
            "空文件拒绝"
        );
    }

    #[test]
    fn content_type_and_extension_must_agree() {
        for (name, content_type) in [
            ("a.png", "image/png"),
            ("a.jpg", "image/jpeg"),
            ("a.jpeg", "image/jpeg"),
            ("a.JPEG", "image/jpeg"),
            ("a.gif", "image/gif"),
            ("a.webp", "image/webp"),
            ("C:\\tmp\\dir.d\\a.png", "image/png"),
        ] {
            assert!(
                validate_image_upload(name, content_type, 1024).is_ok(),
                "{name} + {content_type}"
            );
        }
        for (name, content_type) in [
            ("a.bmp", "image/png"),
            ("a.png", "image/jpeg"),
            ("a.png", "image/heic"),
            ("a", "image/png"),
            ("a.", "image/png"),
        ] {
            assert!(
                validate_image_upload(name, content_type, 1024).is_err(),
                "{name} + {content_type} 应拒绝"
            );
        }
        assert_eq!(extension_of("a.png"), Some("png"));
        assert_eq!(extension_of("no-extension"), None);
        assert_eq!(content_type_for_extension("JPG"), Some("image/jpeg"));
    }

    #[test]
    fn digest_shape_is_checked_before_network() {
        assert!(validate_media_image_request(&prepare("a.png", "image/png", 1024)).is_ok());
        let mut uppercase = prepare("a.png", "image/png", 1024);
        uppercase.md5 = "A".repeat(32);
        assert!(
            validate_media_image_request(&uppercase).is_err(),
            "大写 md5 拒绝"
        );
        let mut short = prepare("a.png", "image/png", 1024);
        short.sha1 = "0".repeat(39);
        assert!(
            validate_media_image_request(&short).is_err(),
            "39 位 sha1 拒绝"
        );
    }

    #[test]
    fn etag_quotes_are_stripped_for_complete() {
        assert_eq!(
            strip_etag_quotes("\"d41d8cd98f00b204e9800998ecf8427e\""),
            "d41d8cd98f00b204e9800998ecf8427e"
        );
        assert_eq!(strip_etag_quotes(" \"abc\" "), "abc");
        assert_eq!(strip_etag_quotes("W/\"abc\""), "abc");
        assert_eq!(strip_etag_quotes("abc"), "abc");
        assert_eq!(strip_etag_quotes(""), "");
    }

    #[test]
    fn presigned_request_uses_put_without_authorization() {
        let request = build_presigned_request(
            "https://cos.example.com/upload?sign=x",
            "image/png",
            b"png-bytes",
        )
        .expect("应能构造 PUT 请求");
        assert_eq!(request.method(), reqwest::Method::PUT);
        assert_eq!(
            request.url().as_str(),
            "https://cos.example.com/upload?sign=x"
        );
        assert_eq!(
            request
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("image/png")
        );
        assert!(
            request
                .headers()
                .get(reqwest::header::AUTHORIZATION)
                .is_none(),
            "直传 COS 不得携带 Authorization"
        );
        assert!(request.body().is_some(), "图片字节必须随 PUT 发出");
    }
}

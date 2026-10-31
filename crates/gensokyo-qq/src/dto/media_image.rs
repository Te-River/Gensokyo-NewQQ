//! 素材管理 DTO（官方 2026-10 素材管理端点族，`/server-inter/message/material-management`）。
//!
//! 字段名与官方表逐字一致：`file_name` / `file_size` / `content_type` / `md5` / `sha1`
//! （prepare），`upload_id` / `md5`（complete）。这些名字也是 Go 侧 DTO 的 json tag，
//! 两侧同名同形，便于对照排障。

use serde::{Deserialize, Serialize};

/// 单张图片大小上限（字节）。
///
/// 官方「限制说明」写 10MB；这里取**十进制** 10_000_000（与 5MB 直传政策同口径的保守取法）。
/// 上游若按二进制口径或更宽，十进制只会多拒 4.8% 的边缘文件，不会误放。
pub const MAX_MEDIA_IMAGE_BYTES: u64 = 10_000_000;

/// 官方支持的 `content_type` 全集。
pub const SUPPORTED_CONTENT_TYPES: &[&str] =
    &["image/png", "image/jpeg", "image/gif", "image/webp"];

/// `POST /v2/media-images/prepare` 请求体。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaImagePrepareRequest {
    /// 文件名（含扩展名，如 `kline.png`）。
    pub file_name: String,
    /// 文件大小（字节）。
    pub file_size: u64,
    /// `image/png` / `image/jpeg` / `image/gif` / `image/webp`。
    pub content_type: String,
    /// 文件内容 MD5，32 位小写十六进制。
    pub md5: String,
    /// 文件内容 SHA1，40 位小写十六进制。
    pub sha1: String,
}

/// `POST /v2/media-images/prepare` 响应。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MediaImagePrepareResponse {
    /// 上传任务 ID，**4 小时**有效；过期需重新 prepare。
    pub upload_id: String,
    /// 直传 COS 的预签名 URL（非 OpenAPI 域名，PUT 时不得携带 Authorization）。
    pub presigned_url: String,
    /// 预签名 URL 有效期（秒）。
    pub expires_in: u32,
}

/// `POST /v2/media-images/complete` 请求体。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MediaImageCompleteRequest {
    /// prepare 返回的上传任务 ID。
    pub upload_id: String,
    /// PUT 响应头 `ETag` 去掉引号后的值（等于文件 MD5）。
    pub md5: String,
}

/// `POST /v2/media-images/complete` 响应。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MediaImageCompleteResponse {
    /// 图片 ID（查询 / 删除用）。
    pub image_id: String,
    /// 素材引用：`qqbot-media://image/v1/<token>`，可原样写进 Markdown。
    pub url: String,
    /// 有效期（秒）：官方 7_776_000 = 90 天滑动窗口。
    pub ttl: u32,
    /// COS 预签名短时效链接；**禁止持久化**。
    pub preview_url: String,
}

/// 单张已上传素材（`GET /v2/media-images` 的 `images[]` 元素）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MediaImage {
    /// 图片 ID。
    pub image_id: String,
    /// 素材引用 `qqbot-media://image/v1/<token>`。
    pub url: String,
    /// 上传时的文件名。
    pub file_name: String,
    /// COS 预签名短时效链接（禁止持久化）。
    pub preview_url: String,
    /// 创建时间（Unix 秒）。
    pub created_at: i64,
    /// 失效时间（Unix 秒；每次被消息引用后顺延 90 天）。
    pub expires_at: i64,
}

/// `GET /v2/media-images?cursor=` 响应。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MediaImageList {
    /// 素材列表，按上传时间倒序。
    pub images: Vec<MediaImage>,
    /// 下一页游标；空串表示已到末页。
    pub next_cursor: String,
    /// 当前机器人已上传的素材总数（与 100 张配额同口径）。
    pub total: u32,
}

#[cfg(test)]
mod tests {
    use super::MediaImagePrepareRequest;
    use serde_json::json;

    #[test]
    fn prepare_request_uses_official_field_names() {
        let request = MediaImagePrepareRequest {
            file_name: "kline.png".to_owned(),
            file_size: 1024,
            content_type: "image/png".to_owned(),
            md5: "a".repeat(32),
            sha1: "b".repeat(40),
        };
        let value = serde_json::to_value(&request).expect("prepare 请求体应可序列化");
        assert_eq!(
            value,
            json!({
                "file_name": "kline.png",
                "file_size": 1024,
                "content_type": "image/png",
                "md5": "a".repeat(32),
                "sha1": "b".repeat(40),
            })
        );
        let back: MediaImagePrepareRequest =
            serde_json::from_value(value).expect("同名同形应可反序列化");
        assert_eq!(back, request);
    }
}

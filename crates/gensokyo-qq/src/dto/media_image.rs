//! 素材管理 DTO（官方 2026-10 素材管理端点族，`/server-inter/message/material-management`）。
//!
//! 字段名与官方表逐字一致：`file_name` / `file_size` / `content_type` / `md5` / `sha1`
//! （prepare），`upload_id` / `md5`（complete）。这些名字也是 Go 侧 DTO 的 json tag，
//! 两侧同名同形，便于对照排障。

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

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
    #[serde(deserialize_with = "deserialize_lenient_int")]
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
    #[serde(deserialize_with = "deserialize_lenient_int")]
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
    #[serde(deserialize_with = "deserialize_lenient_int")]
    pub created_at: i64,
    /// 失效时间（Unix 秒；每次被消息引用后顺延 90 天）。
    #[serde(deserialize_with = "deserialize_lenient_int")]
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
    #[serde(deserialize_with = "deserialize_lenient_int")]
    pub total: u32,
}

/// 素材管理数值字段的宽容反序列化：官方实测把 `total` / `created_at` 等数值以**字符串**下发
/// （Go 侧线上日志：`json: cannot unmarshal string into Go struct field MediaImageList.total of type int`），
/// 同时接受 JSON number 与数字字符串；布尔、null、对象、非数字字符串仍明确报错，不静默吞值。
/// 字段**缺失**时容器级 `serde(default)` 照常兜底回 0，缺省行为不因宽容化改变。
fn deserialize_lenient_int<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    let value = Value::deserialize(deserializer)?;
    let text = match &value {
        Value::Number(number) => number.to_string(),
        Value::String(string) => string.clone(),
        other => {
            return Err(serde::de::Error::custom(format!(
                "素材数值字段应为数字或数字字符串，收到 {other}"
            )));
        }
    };
    text.parse::<T>().map_err(|error| {
        serde::de::Error::custom(format!("素材数值字段无法解析为整数，收到 {text}: {error}"))
    })
}

#[cfg(test)]
mod tests {
    use super::{
        MediaImageCompleteResponse, MediaImageList, MediaImagePrepareRequest,
        MediaImagePrepareResponse,
    };
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

    /// 官方线上实发形态：`total` / `created_at` / `expires_at` 均为字符串数字。
    #[test]
    fn list_accepts_string_numbers() {
        let list: MediaImageList = serde_json::from_value(json!({
            "images": [{
                "image_id": "img_1",
                "url": "qqbot-media://image/v1/token",
                "file_name": "kline.png",
                "preview_url": "https://cos.example/short-lived",
                "created_at": "1792051409",
                "expires_at": "1799798609",
            }],
            "next_cursor": "cursor_2",
            "total": "100",
        }))
        .expect("素材列表数值字段应接受字符串数字形态");
        assert_eq!(list.total, 100);
        assert_eq!(list.images[0].created_at, 1_792_051_409);
        assert_eq!(list.images[0].expires_at, 1_799_798_609);
    }

    /// number 形态照旧（回归：宽容化不得破坏原有严格数值解析）。
    #[test]
    fn list_accepts_plain_numbers() {
        let list: MediaImageList = serde_json::from_value(json!({
            "images": [{ "created_at": 1_792_051_409_i64, "expires_at": 1_799_798_609_i64 }],
            "next_cursor": "",
            "total": 7,
        }))
        .expect("数字形态应保持原有解析能力");
        assert_eq!(list.total, 7);
        assert_eq!(list.images[0].created_at, 1_792_051_409);
    }

    /// 混排：同一响应里 `total` 是串、`created_at` 是数。
    #[test]
    fn list_accepts_mixed_forms() {
        let list: MediaImageList = serde_json::from_value(json!({
            "images": [{ "created_at": 1_792_051_409_i64, "expires_at": "1799798609" }],
            "total": "100",
        }))
        .expect("混排形态应同时接受");
        assert_eq!(list.total, 100);
        assert_eq!(list.images[0].created_at, 1_792_051_409);
        assert_eq!(list.images[0].expires_at, 1_799_798_609);
    }

    /// 非法形态（布尔 / 对象 / null / 非数字串 / 非整值串）仍明确报错，错误文案带字段语义。
    #[test]
    fn list_rejects_bad_total_shapes() {
        for bad in [
            json!("abc"),
            json!(true),
            json!({}),
            json!(null),
            json!("12.5"),
            json!(-1),
        ] {
            let result: Result<MediaImageList, _> = serde_json::from_value(json!({ "total": bad }));
            let error = result.expect_err(&format!("非法 total 形态 {bad} 应报错"));
            assert!(
                error.to_string().contains("素材数值字段"),
                "错误文案应带字段语义: {error}"
            );
        }
    }

    /// 字段缺失仍走容器级 default 回 0（宽容化不改缺省行为）。
    #[test]
    fn list_missing_fields_keep_default() {
        let list: MediaImageList = serde_json::from_value(json!({})).expect("缺失字段应走 default");
        assert_eq!(list.total, 0);
        assert!(list.images.is_empty());
        let image: MediaImageList =
            serde_json::from_value(json!({ "images": [{}] })).expect("元素缺失字段应走 default");
        assert_eq!(image.images[0].created_at, 0);
        assert_eq!(image.images[0].expires_at, 0);
    }

    /// 上传链同为官方数值家：prepare 的 `expires_in` 与 complete 的 `ttl` 同雷同修。
    #[test]
    fn prepare_and_complete_accept_string_numbers() {
        let prepared: MediaImagePrepareResponse = serde_json::from_value(json!({
            "upload_id": "up_1",
            "presigned_url": "https://cos.example/put",
            "expires_in": "3600",
        }))
        .expect("expires_in 应接受字符串数字");
        assert_eq!(prepared.expires_in, 3600);
        let completed: MediaImageCompleteResponse = serde_json::from_value(json!({
            "image_id": "img_1",
            "url": "qqbot-media://image/v1/token",
            "ttl": "7776000",
            "preview_url": "https://cos.example/short",
        }))
        .expect("ttl 应接受字符串数字");
        assert_eq!(completed.ttl, 7_776_000);
    }
}

//! `/uploadpic` / `/uploadpicv2` / `/uploadrecord` / `/getid`（Go `server/uploadpic.go` +
//! `server/getIDHandler.go`）。
//!
//! # 挂载位置
//!
//! Go 把三条路由挂在主服务器 `r`（`main.go:508` / `:523-524`，端口 `port`，lotus 时
//! `backup_port`）。Rust 的主 listener 是正向 WS 端口（[`crate::boot`] 里的 `ws`），所以
//! 这三条路由并进该 listener；`enable_ws_server=false` 时主 listener 不存在，才落到 HTTP API
//! 端口（Go 的 `r` 始终存在，这是最接近的等价物）。
//!
//! # 鉴权（与 Go 逐条对齐）
//!
//! - `/uploadpic*`、`/uploadrecord`：`UploadAuthMiddleware`——`http_access_token` 非空时，`Authorization` 去掉
//!   首处 `"Bearer "` 后的值或 `?access_token=` 必须相等，否则 **403** `{"error":"鉴权失败"}`。
//! - `/getid`：`IDMapAuthMiddleware`——有 `lotus_password` 时校验
//!   `HMAC-SHA256(password, path[?sorted k=v&…]&timestamp=<ts>)`（`token` / `timestamp`
//!   不参与签名，query 参数按键排序、同键保留请求顺序），时间窗口 ±300s；缺 token /
//!   缺 timestamp / 非法 timestamp / 过期 / 签名不符分别回 401 的对应文案。无密码时只允许
//!   TCP 对端为 `127.0.0.1` / `::1`（gin 默认不信任代理，`ClientIP()` 就是对端地址），
//!   否则 **403** `{"error":"仅允许本地访问"}`。
//!
//! # 与 Go 的差异
//!
//! - `oss_type=0`（本地上传）：Go 把 base64 回环 POST 给自己的 `/uploadpic`；这里直接走同一段
//!   保存 / 拼 URL 逻辑（省掉一跳），成功响应完全一致。
//! - `oss_type=0` 且 `lotus=true && !lotus_without_uploadpic` 时 Go 转发给远端 lotus 图床；
//!   Rust 未移植 lotus 出站，返回 500 并说明原因（不伪造成功）。
//! - `getid` 的 14（Go `FindSubKeysByIdPro`）按正表 key 前缀扫子键——Go 读的是已退役的 bbolt
//!   `ids` 桶，redb 没有该桶，现代 schema 下与 Go 一样恒空，回 200 `{"keys":null}`（Go 的
//!   nil 切片序列化成 `null`）；复合键类型 8~11 与重绑定 5/12 由本地 `IdMapStore` 组合实现
//!   （与 gRPC 服务端同一套语义），其余类型与 Go 同码同形状。Go 的 `switch` 无 `default`，
//!   未知 type 返回 **200 空响应体**，此处照抄。
//! - `getid` 18/19（Go `ForceUnbindID` / `ListAllIdentities`）：18 按 `normalize_identity` 删正表
//!   条目，因此解绑后正反查都查不到（Go 删的是 `uin:<raw>`，命中不了真实 key，正表会残留）；
//!   19 的 `real_id` 取反表里的原始身份串（Go 取归一化 key），空库回 `[]`（Go 的 nil 切片是
//!   `null`）。18 的错误按 Go 一律回 500（Go 该分支没有 404）。
//! - `getid` 的挂载与 gRPC 接管联动：Go 在 lotus gRPC 接管 idmap 时不注册该路由
//!   （`main.go:507`）。是否接管由 [`crate::boot`] 按「服务端是否真的起来」决定——起不来时
//!   仍挂本地路由（Go 那里会 `Fatalf`），避免远端 lotus 调用整条消失（见 [`router`]）。
//! - `/uploadrecord`：Go 的 handler 只落 `./channel_temp/<md5>.silk` 再拼 `server_dir:port`，
//!   不走 `images.UploadBase64RecordToServer`——那条链在 Go 里**没有调用方**
//!   （`oss.UploadAndAuditRecord*` 同样是死代码），因此 `oss_type` 的任何取值都不影响语音，
//!   这里逐行照抄（语音地址也**不**套 `frp_port` / `stun`，见 [`record_url`]）。
//! - `/uploadpicv2` 的宽高：Go 对 0~3 按 URL 再下载一次取尺寸；这里统一读内存里的文件头
//!   （`gensokyo_media::image_size`），少一次网络往返，结果对同一张图一致。
//! - 图床短签名外链的 302→直链解析属 P7 的口径（`gensokyo_media::hosting` 未做），v2 复用
//!   同一条上传链，因此同样不做二次解析。

use std::collections::{BTreeMap, HashMap};
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::extract::{
    ConnectInfo, DefaultBodyLimit, Form, FromRequest, Multipart, Query, Request, State,
};
use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use gensokyo_config::Settings;
use gensokyo_media::{HostingKind, ImageKind, OssKind, image_size, sniff_image};
use gensokyo_storage::{IdMapStore, StorageError};
use md5::{Digest, Md5};
use serde_json::json;

use crate::hosting::{config_from_settings, oss_config_from_settings};
use crate::state::AppState;

/// 表单体上限：对齐 Go `MaxMultipartMemory`（32 MiB），大图由 base64 撑大后仍能收下。
const MAX_FORM_BYTES: usize = 32 * 1024 * 1024;
/// 上传频控窗口：Go `RequestInterval = time.Minute`。
const RATE_WINDOW: Duration = Duration::from_secs(60);
/// 本地上传目录：Go `"./channel_temp/"`（相对进程工作目录）。
const CHANNEL_TEMP: &str = "./channel_temp";
/// `getid` 签名时间窗口：Go `±300s`。
const TIMESTAMP_WINDOW: i64 = 300;

/// 三条 idmap / 上传路由。
///
/// `mount_getid=false` 对应 Go `main.go:507`：lotus gRPC 接管 idmap 后不再注册本地
/// `/getid`（同一进程不摆两套 idmap 入口）。是否接管由 [`crate::boot`] 按「服务端是否真的
/// 起来、远程客户端是否真挂上」决定——任一失败降级时仍挂本地路由（Go 那里会
/// `Fatalf`/`panic`），避免远端 lotus 调用整条消失（见 [`router`]）。
pub(crate) fn router(state: AppState, mount_getid: bool) -> Router {
    let routes = Router::new()
        .route("/uploadpic", post(upload_pic))
        .route("/uploadpicv2", post(upload_pic_v2))
        .route("/uploadrecord", post(upload_record));
    let routes = if mount_getid {
        routes.route("/getid", get(get_id))
    } else {
        routes
    };
    routes
        .layer(DefaultBodyLimit::max(MAX_FORM_BYTES))
        .with_state(state)
}

// ---------------------------------------------------------------- /uploadpic

async fn upload_pic(State(state): State<AppState>, request: Request) -> Response {
    // Go 中间件先于 handler，鉴权失败不会消耗频控额度
    if !upload_authorized(&state, &request) {
        return json_error(StatusCode::FORBIDDEN, "鉴权失败");
    }
    let peer = peer_ip(&request);
    if !allow_request(&peer, state.config().snapshot().get_image_limit()) {
        return json_error(StatusCode::TOO_MANY_REQUESTS, "rate limit exceeded");
    }

    let base64_image = match form_field(request, &state, "base64Image").await {
        Ok(value) => value,
        Err(FormFieldError::TooLarge) => return body_too_large(),
    };
    tracing::debug!(chars = base64_image.len(), "收到 base64 图片");

    let (bytes, extension) = match decode_image(&base64_image) {
        Ok(decoded) => decoded,
        Err(DecodeImageError::Base64) => {
            tracing::debug!("base64 解码失败");
            return json_error(StatusCode::BAD_REQUEST, "invalid base64 data");
        }
        Err(DecodeImageError::Format) => {
            return json_error(StatusCode::BAD_REQUEST, "undefined picture format1");
        }
    };

    let file_name = match save_channel_file(&bytes, extension).await {
        Ok(name) => name,
        Err(message) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, message),
    };
    match channel_url(&state, &file_name) {
        Ok(url) => Json(json!({ "url": url })).into_response(),
        Err(message) => json_error(StatusCode::INTERNAL_SERVER_ERROR, message),
    }
}

/// base64 → 图片字节 + 扩展名；只认 Go 注册的 jpeg / png / gif。
fn decode_image(encoded: &str) -> Result<(Vec<u8>, &'static str), DecodeImageError> {
    // Go `base64.StdEncoding.DecodeString` 忽略 \r \n
    let cleaned: String = encoded
        .chars()
        .filter(|ch| *ch != '\r' && *ch != '\n')
        .collect();
    let bytes = BASE64
        .decode(cleaned.as_bytes())
        .map_err(|_| DecodeImageError::Base64)?;
    let extension = sniff_image(&bytes).and_then(go_extension);
    // Go `image.DecodeConfig` 要能读到完整头部；`image_size` 是同一口径的文件头校验
    match (extension, image_size(&bytes)) {
        (Some(extension), Some(_)) => Ok((bytes, extension)),
        _ => Err(DecodeImageError::Format),
    }
}

/// Go `getFileExtensionFromImageFormat` 只映射 jpeg / gif / png（webp / bmp 在 Go 侧属未知格式）。
fn go_extension(kind: ImageKind) -> Option<&'static str> {
    match kind {
        ImageKind::Png => Some("png"),
        ImageKind::Jpeg => Some("jpg"),
        ImageKind::Gif => Some("gif"),
        ImageKind::WebP | ImageKind::Bmp => None,
    }
}

/// 保存到 `./channel_temp/<md5>.<ext>`；同名文件已存在时跳过（Go 行为）。
async fn save_channel_file(bytes: &[u8], extension: &str) -> Result<String, &'static str> {
    let file_name = format!("{}.{extension}", md5_hex(bytes));
    let directory = Path::new(CHANNEL_TEMP);
    tokio::fs::create_dir_all(directory)
        .await
        .map_err(|_| "error creating directory")?;
    let path = directory.join(&file_name);
    if tokio::fs::metadata(&path).await.is_ok() {
        tracing::debug!(file = %file_name, "文件已存在，跳过保存");
    } else {
        tokio::fs::write(&path, bytes)
            .await
            .map_err(|_| "error saving file")?;
    }
    Ok(file_name)
}

/// `/channel_temp/<name>` 的公网地址：`stun` 配置优先，否则 `server_dir:port`。
fn channel_url(state: &AppState, file_name: &str) -> Result<String, &'static str> {
    let settings = state.config().snapshot();
    let server = settings.get_server_dir();
    if server.is_empty() {
        return Err("server address is not configured");
    }
    let frp_port = settings.get_frp_port();
    let port = if frp_port == "0" {
        settings.get_port_value()
    } else {
        frp_port
    };
    let protocol = if port == "443" || settings.get_force_ssl() {
        "https"
    } else {
        "http"
    };
    if let Some(stun) = stun_address(state) {
        return Ok(format!("http://{stun}/channel_temp/{file_name}"));
    }
    Ok(format!(
        "{protocol}://{server}:{port}/channel_temp/{file_name}"
    ))
}

/// `idmap` 的 `stun:addr`（Go `idmap.ReadConfigv2("stun", "addr")`）。
fn stun_address(state: &AppState) -> Option<String> {
    let value = state.idmap()?.read_config("stun", "addr").ok()?;
    (!value.is_empty()).then_some(value)
}

// -------------------------------------------------------------- /uploadpicv2

async fn upload_pic_v2(State(state): State<AppState>, request: Request) -> Response {
    if !upload_authorized(&state, &request) {
        return json_error(StatusCode::FORBIDDEN, "鉴权失败");
    }
    let peer = peer_ip(&request);
    if !allow_request(&peer, state.config().snapshot().get_image_limit()) {
        return json_error(StatusCode::TOO_MANY_REQUESTS, "rate limit exceeded");
    }

    let base64_image = match form_field(request, &state, "base64Image").await {
        Ok(value) => value,
        Err(FormFieldError::TooLarge) => return body_too_large(),
    };
    if base64_image.is_empty() {
        return json_error(
            StatusCode::BAD_REQUEST,
            "either base64Image or url is required",
        );
    }
    let bytes = match decode_base64(&base64_image) {
        Ok(bytes) => bytes,
        Err(err) => {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("base64 解码失败: {err}"),
            );
        }
    };

    let settings = state.config().snapshot();
    let url = match upload_by_oss_type(&state, &settings, settings.get_oss_type(), &bytes).await {
        Ok(url) => url,
        Err(message) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, message),
    };
    let (width, height) = image_size(&bytes).unwrap_or((0, 0));
    Json(json!({ "url": url, "width": width, "height": height })).into_response()
}

/// Go `images.UploadBase64ImageToServer` 的 `oss_type` 分派。
async fn upload_by_oss_type(
    state: &AppState,
    settings: &Settings,
    oss_type: i64,
    bytes: &[u8],
) -> Result<String, String> {
    if oss_type == 0 {
        return local_upload(state, settings, bytes).await;
    }
    if let Some(kind) = OssKind::from_oss_type(oss_type) {
        let config = oss_config_from_settings(settings);
        return state
            .hosting()
            .upload_oss(kind, &config, bytes)
            .await
            .map_err(|err| err.to_string());
    }
    if let Some(kind) = HostingKind::from_oss_type(oss_type) {
        let config = config_from_settings(settings);
        return state
            .hosting()
            .upload(kind, &config, bytes, "image.png")
            .await
            .map_err(|err| err.to_string());
    }
    Err("invalid oss_type".to_owned())
}

/// `oss_type=0`：Go `originalUploadBehavior` 的回环自调用，这里直接保存并拼 URL。
async fn local_upload(
    state: &AppState,
    settings: &Settings,
    bytes: &[u8],
) -> Result<String, String> {
    if settings.get_lotus() && !settings.get_lotus_without_uploadpic() {
        return Err("lotus 模式未实现：无法转发到远端 /uploadpic".to_owned());
    }
    if !is_public_address(settings.get_server_dir()) {
        return Err("local server uses a private address; image upload failed".to_owned());
    }
    let extension = sniff_image(bytes)
        .and_then(go_extension)
        .ok_or_else(|| "undefined picture format1".to_owned())?;
    let file_name = save_channel_file(bytes, extension)
        .await
        .map_err(str::to_owned)?;
    channel_url(state, &file_name).map_err(str::to_owned)
}

/// Go `isPublicAddress`：localhost / `127.` / `192.168.` 视为私有，其余（含域名）视为公网。
fn is_public_address(address: &str) -> bool {
    !(address.contains("localhost")
        || address.starts_with("127.")
        || address.starts_with("192.168."))
}

// ------------------------------------------------------------ /uploadrecord

async fn upload_record(State(state): State<AppState>, request: Request) -> Response {
    // Go 中间件先于 handler，鉴权失败不会消耗频控额度
    if !upload_authorized(&state, &request) {
        return json_error(StatusCode::FORBIDDEN, "鉴权失败");
    }
    // 与 `/uploadpic` 共用同一个进程级计数表（Go 三条路由共用一个 `rateLimiter`）
    let peer = peer_ip(&request);
    if !allow_request(&peer, state.config().snapshot().get_image_limit()) {
        return json_error(StatusCode::TOO_MANY_REQUESTS, "rate limit exceeded");
    }

    let base64_record = match form_field(request, &state, "base64Record").await {
        Ok(value) => value,
        Err(FormFieldError::TooLarge) => return body_too_large(),
    };
    tracing::debug!(chars = base64_record.len(), "收到 base64 语音");

    let bytes = match decode_base64(&base64_record) {
        Ok(bytes) => bytes,
        Err(err) => {
            tracing::debug!(%err, "base64 解码失败");
            return json_error(StatusCode::BAD_REQUEST, "invalid base64 data");
        }
    };

    // Go 不看文件头：空串也能解码出空字节、写成 `<空串 md5>.silk` 并返回 200，这里照抄
    let file_name = match save_channel_file(&bytes, "silk").await {
        Ok(name) => name,
        Err(message) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, message),
    };
    match record_url(&state, &file_name) {
        Ok(url) => Json(json!({ "url": url })).into_response(),
        Err(message) => json_error(StatusCode::INTERNAL_SERVER_ERROR, message),
    }
}

/// `/uploadrecord` 的静态地址：Go `UploadBase64RecordHandler` 只拼 `server_dir:port`。
///
/// 与图片的 [`channel_url`] 不同——Go 的语音 handler 既不看 `frp_port`，也不看 `stun`。
fn record_url(state: &AppState, file_name: &str) -> Result<String, &'static str> {
    let settings = state.config().snapshot();
    let server = settings.get_server_dir();
    if server.is_empty() {
        return Err("server address is not configured");
    }
    let port = settings.get_port_value();
    let protocol = if port == "443" || settings.get_force_ssl() {
        "https"
    } else {
        "http"
    };
    Ok(format!(
        "{protocol}://{server}:{port}/channel_temp/{file_name}"
    ))
}

// ------------------------------------------------------------------- /getid

async fn get_id(State(state): State<AppState>, request: Request) -> Response {
    let peer = peer_ip(&request);
    if let Some(rejection) = authenticate(&state, &request, peer) {
        return rejection;
    }
    let pairs = query_pairs(request.uri());
    let Ok(type_value) = first(&pairs, "type").unwrap_or("").parse::<i64>() else {
        return json_error(StatusCode::BAD_REQUEST, "invalid type");
    };
    let id = first(&pairs, "id").unwrap_or("");
    match type_value {
        1 | 13 => store_identity(&state, id),
        2 => retrieve_by_row(&state, id),
        3 => {
            let subtype = first(&pairs, "subtype").unwrap_or("");
            let value = first(&pairs, "value").unwrap_or("");
            let Some(idmap) = state.idmap() else {
                return idmap_missing();
            };
            match idmap.write_config(id, subtype, value) {
                Ok(()) => Json(json!({ "status": "success" })).into_response(),
                Err(err) => json_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
            }
        }
        4 => {
            let subtype = first(&pairs, "subtype").unwrap_or("");
            let Some(idmap) = state.idmap() else {
                return idmap_missing();
            };
            match idmap.read_config(id, subtype) {
                Ok(value) => Json(json!({ "value": value })).into_response(),
                Err(err) => json_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
            }
        }
        5 => {
            let Some(old_row_value) = parse_param(&pairs, "oldRowValue") else {
                return json_error(StatusCode::BAD_REQUEST, "invalid oldRowValue");
            };
            let Some(new_row_value) = parse_param(&pairs, "newRowValue") else {
                return json_error(StatusCode::BAD_REQUEST, "invalid newRowValue");
            };
            let Some(idmap) = state.idmap() else {
                return idmap_missing();
            };
            // Go `UpdateVirtualValue`：三段反查身份 → 迁移正反表（`new=0` 彻底解绑）。
            match idmap.update_virtual_value(old_row_value, new_row_value) {
                Ok(()) => Json(json!({ "status": "success" })).into_response(),
                Err(err) => {
                    tracing::warn!(
                        %err,
                        old = old_row_value,
                        new = new_row_value,
                        "getid type=5 重绑定失败"
                    );
                    // Go 的文案：旧值查不到是 `不存在:<old>`；目标被占用由
                    // `StorageError::AlreadyExists` 自带 `<new> :已存在`。
                    let message = if err.is_not_found() {
                        format!("不存在:{old_row_value}")
                    } else {
                        err.to_string()
                    };
                    json_error(StatusCode::INTERNAL_SERVER_ERROR, message)
                }
            }
        }
        6 => {
            let Some(virtual_value) = parse_param(&pairs, "virtualValue") else {
                return json_error(StatusCode::BAD_REQUEST, "invalid virtualValue");
            };
            let Some(idmap) = state.idmap() else {
                return idmap_missing();
            };
            match idmap.retrieve_row_by_id(virtual_value) {
                Ok(real) => Json(json!({
                    "virtual": virtual_value.to_string(),
                    "real": real,
                }))
                .into_response(),
                Err(err) => json_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
            }
        }
        7 => {
            if id.is_empty() {
                return json_error(StatusCode::BAD_REQUEST, "invalid id");
            }
            let Some(idmap) = state.idmap() else {
                return idmap_missing();
            };
            match idmap.lookup_virtual_value(id) {
                Ok(virtual_value) => {
                    Json(json!({ "virtual": virtual_value.to_string() })).into_response()
                }
                Err(err) => json_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
            }
        }
        8 => {
            let subid = first(&pairs, "subid").unwrap_or("");
            if subid.is_empty() {
                return json_error(
                    StatusCode::BAD_REQUEST,
                    "subid parameter is required for type 8",
                );
            }
            let Some(idmap) = state.idmap() else {
                return idmap_missing();
            };
            // Go `StoreIDv2Pro`：两次独立的 `StoreIDv2`（v2 无复合键）。
            let row = match idmap.retrieve_virtual_value(id) {
                Ok((_, row)) => row,
                Err(err) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
            };
            let sub_row = match idmap.retrieve_virtual_value(subid) {
                Ok((_, row)) => row,
                Err(err) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
            };
            Json(json!({ "row": row, "subRow": sub_row })).into_response()
        }
        9 => {
            let subid = first(&pairs, "subid").unwrap_or("");
            if subid.is_empty() {
                return json_error(
                    StatusCode::BAD_REQUEST,
                    "subid parameter is required for type 9",
                );
            }
            let Some(idmap) = state.idmap() else {
                return idmap_missing();
            };
            // Go `RetrieveRowByIDv2Pro`：两次独立的 `RetrieveRowByIDv2`。
            let row = match retrieve_row(idmap, id) {
                Ok(row) => row,
                Err(err) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
            };
            let sub_row = match retrieve_row(idmap, subid) {
                Ok(row) => row,
                Err(err) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
            };
            Json(json!({ "id": row, "subid": sub_row })).into_response()
        }
        10 => {
            let subid = first(&pairs, "subid").unwrap_or("");
            if id.is_empty() || subid.is_empty() {
                return json_error(
                    StatusCode::BAD_REQUEST,
                    "id and subid parameters are required for type 10",
                );
            }
            let Some(idmap) = state.idmap() else {
                return idmap_missing();
            };
            // Go `RetrieveVirtualValuev2Pro`：两次 `RetrieveVirtualValuev2`（只查不分配）。
            let first_value = match idmap.lookup_virtual_value(id) {
                Ok(value) => value,
                Err(err) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
            };
            let second_value = match idmap.lookup_virtual_value(subid) {
                Ok(value) => value,
                Err(err) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
            };
            Json(json!({
                "firstValue": first_value.to_string(),
                "secondValue": second_value.to_string(),
            }))
            .into_response()
        }
        11 => {
            let subid = first(&pairs, "subid").unwrap_or("");
            if id.is_empty() || subid.is_empty() {
                return json_error(
                    StatusCode::BAD_REQUEST,
                    "id and subid parameters are required for type 11",
                );
            }
            let Ok(first) = id.parse::<i64>() else {
                return json_error(StatusCode::BAD_REQUEST, "Invalid input values idOrRow");
            };
            let Ok(second) = subid.parse::<i64>() else {
                return json_error(StatusCode::BAD_REQUEST, "Invalid input values subid");
            };
            let Some(idmap) = state.idmap() else {
                return idmap_missing();
            };
            // Go `RetrieveRealValuesv2Pro`：两次 `retrieveIdentity`（vUIN → 身份串）。
            let first_real = match idmap.retrieve_row_by_id(first) {
                Ok(real) => real,
                Err(err) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
            };
            let second_real = match idmap.retrieve_row_by_id(second) {
                Ok(real) => real,
                Err(err) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
            };
            Json(json!({
                "firstRealValue": first_real,
                "secondRealValue": second_real,
            }))
            .into_response()
        }
        12 => {
            let mut values = [0_i64; 4];
            for (slot, param) in values.iter_mut().zip([
                "oldVirtualValue1",
                "newVirtualValue1",
                "oldVirtualValue2",
                "newVirtualValue2",
            ]) {
                let Some(value) = parse_param(&pairs, param) else {
                    return json_error(
                        StatusCode::BAD_REQUEST,
                        format!("Invalid input values {param}"),
                    );
                };
                *slot = value;
            }
            let Some(idmap) = state.idmap() else {
                return idmap_missing();
            };
            // Go `UpdateVirtualValuev2Pro`：两次顺序重绑定，非原子。
            match idmap.update_virtual_value_pro(values[0], values[1], values[2], values[3]) {
                Ok(()) => Json(json!({ "message": "Virtual values updated successfully" }))
                    .into_response(),
                Err(err) => json_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
            }
        }
        14 => {
            let Some(idmap) = state.idmap() else {
                return idmap_missing();
            };
            // Go `FindSubKeysByIdPro`：读退役 bbolt `ids` 桶的 `id:sub` 复合键；redb 按正表
            // 前缀扫，现代 schema 恒空。Go 的 nil 切片序列化成 `null`，这里照抄。
            match idmap.find_sub_keys_by_id(id) {
                Ok(keys) if keys.is_empty() => Json(json!({ "keys": null })).into_response(),
                Ok(keys) => Json(json!({ "keys": keys })).into_response(),
                Err(err) => json_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
            }
        }
        15 => {
            let subtype = first(&pairs, "subtype").unwrap_or("");
            let Some(idmap) = state.idmap() else {
                return idmap_missing();
            };
            match idmap.delete_config(id, subtype) {
                Ok(_) => Json(json!({ "status": "success" })).into_response(),
                Err(err) => json_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
            }
        }
        16 => {
            let Some(idmap) = state.idmap() else {
                return idmap_missing();
            };
            match idmap.store_cache(id) {
                Ok(row) => Json(json!({ "row": row })).into_response(),
                Err(err) => json_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
            }
        }
        17 => {
            let Ok(virtual_id) = id.parse::<i64>() else {
                return json_error(StatusCode::NOT_FOUND, "ID not found");
            };
            let Some(idmap) = state.idmap() else {
                return idmap_missing();
            };
            match idmap.retrieve_row_by_cache(virtual_id) {
                Ok(real) => Json(json!({ "id": real })).into_response(),
                Err(err) if err.is_not_found() => json_error(StatusCode::NOT_FOUND, "ID not found"),
                Err(err) => json_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
            }
        }
        18 => {
            if id.is_empty() {
                return json_error(
                    StatusCode::BAD_REQUEST,
                    "id parameter is required for type 18",
                );
            }
            let Some(idmap) = state.idmap() else {
                return idmap_missing();
            };
            // Go `ForceUnbindID`：删正表条目 + 该身份的全部反对表条目，返回反表计数。
            // Go 的 type 18 分支没有 404，任何错误都回 500。
            match idmap.force_unbind(id) {
                Ok(unbound) => Json(json!({
                    "status": "success",
                    "unbound_count": unbound,
                }))
                .into_response(),
                Err(err) => json_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
            }
        }
        19 => {
            let Some(idmap) = state.idmap() else {
                return idmap_missing();
            };
            // Go `ListAllIdentities`：快照直接作为响应体（不套 data 层）。
            // 差异：Go 的 nil 切片序列化成 `"mappings": null`，空库这里给 `[]`。
            match idmap.list_identities() {
                Ok(mappings) => {
                    let entries: Vec<_> = mappings
                        .iter()
                        .map(|mapping| {
                            json!({
                                "real_id": mapping.real_id,
                                "virtual_id": mapping.virtual_id,
                                "username": mapping.username,
                            })
                        })
                        .collect();
                    let count = entries.len();
                    Json(json!({
                        "mappings": entries,
                        "count": count,
                        "timestamp": unix_now(),
                    }))
                    .into_response()
                }
                Err(err) => json_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
            }
        }
        // Go 的 switch 没有 default：写完 200 头就返回，响应体为空
        _ => StatusCode::OK.into_response(),
    }
}

/// type 1 / 13：Go `StoreID` / `SimplifiedStoreIDv2`（lotus 关闭时同为实现分配）。
fn store_identity(state: &AppState, id: &str) -> Response {
    let Some(idmap) = state.idmap() else {
        return idmap_missing();
    };
    match idmap.retrieve_virtual_value(id) {
        Ok((_, row)) => Json(json!({ "row": row })).into_response(),
        Err(err) => json_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
    }
}

/// type 2：Go `RetrieveRowByID`；解析不出数字按查不到处理（Go 走 `ErrKeyNotFound`）。
fn retrieve_by_row(state: &AppState, id: &str) -> Response {
    let Ok(row) = id.parse::<i64>() else {
        return json_error(StatusCode::NOT_FOUND, "ID not found");
    };
    let Some(idmap) = state.idmap() else {
        return idmap_missing();
    };
    match idmap.retrieve_row_by_id(row) {
        Ok(identity) => Json(json!({ "id": identity })).into_response(),
        Err(err) if err.is_not_found() => json_error(StatusCode::NOT_FOUND, "ID not found"),
        Err(err) => json_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
    }
}

/// `getid` 复合键的 vUIN 反查：非十进制按「查不到」处理（Go `RetrieveRowByIDv2` 的解析错误
/// 在 handler 里同样落到 500）。
fn retrieve_row(idmap: &IdMapStore, raw: &str) -> Result<String, StorageError> {
    match raw.parse::<i64>() {
        Ok(row) => idmap.retrieve_row_by_id(row),
        Err(_) => Err(StorageError::NotFound(format!("vUIN {raw}"))),
    }
}

fn idmap_missing() -> Response {
    json_error(StatusCode::INTERNAL_SERVER_ERROR, "idmap 未装配")
}

// ------------------------------------------------------------------- 鉴权

/// `/getid` 的鉴权；通过返回 `None`。
fn authenticate(state: &AppState, request: &Request, peer: Option<IpAddr>) -> Option<Response> {
    let settings = state.config().snapshot();
    let password = settings.get_lotus_password();
    if password.is_empty() {
        let loopback = peer.is_some_and(|ip| matches!(peer_key(ip).as_str(), "127.0.0.1" | "::1"));
        return (!loopback).then(|| json_error(StatusCode::FORBIDDEN, "仅允许本地访问"));
    }

    let pairs = query_pairs(request.uri());
    let token = first(&pairs, "token")
        .filter(|token| !token.is_empty())
        .or_else(|| {
            request
                .headers()
                .get("X-Token")
                .and_then(|value| value.to_str().ok())
        })
        .unwrap_or("");
    if token.is_empty() {
        return Some(json_error(StatusCode::UNAUTHORIZED, "missing token"));
    }
    let timestamp = first(&pairs, "timestamp").unwrap_or("");
    if timestamp.is_empty() {
        return Some(json_error(StatusCode::UNAUTHORIZED, "missing timestamp"));
    }
    let Ok(sent_at) = timestamp.parse::<i64>() else {
        return Some(json_error(StatusCode::UNAUTHORIZED, "invalid timestamp"));
    };
    let now = unix_now();
    if now.saturating_sub(sent_at) > TIMESTAMP_WINDOW
        || sent_at.saturating_sub(now) > TIMESTAMP_WINDOW
    {
        return Some(json_error(StatusCode::UNAUTHORIZED, "timestamp expired"));
    }

    let payload = sign_payload(request.uri().path(), &pairs, timestamp);
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, password.as_bytes());
    let valid = hex_decode(token)
        .is_some_and(|tag| ring::hmac::verify(&key, payload.as_bytes(), &tag).is_ok());
    if !valid {
        return Some(json_error(StatusCode::UNAUTHORIZED, "unauthorized"));
    }
    None
}

/// 重建待签名串：`path[?k=v&…]&timestamp=<ts>`（Go 在无参数时也会补 `&timestamp=`，无 `?`）。
fn sign_payload(path: &str, pairs: &[(String, String)], timestamp: &str) -> String {
    let mut grouped: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (key, value) in pairs {
        if key == "token" || key == "timestamp" {
            continue;
        }
        grouped
            .entry(key.as_str())
            .or_default()
            .push(value.as_str());
    }
    let mut payload = String::from(path);
    if !grouped.is_empty() {
        let joined: Vec<String> = grouped
            .iter()
            .flat_map(|(key, values)| values.iter().map(move |value| format!("{key}={value}")))
            .collect();
        payload.push('?');
        payload.push_str(&joined.join("&"));
    }
    payload.push_str("&timestamp=");
    payload.push_str(timestamp);
    payload
}

/// 严格小写 hex 解码（Go 的 `token != expected` 区分大小写）。
fn hex_decode(token: &str) -> Option<Vec<u8>> {
    let bytes = token.as_bytes();
    if bytes.len() != 64 {
        return None;
    }
    let mut decoded = Vec::with_capacity(32);
    for [high, low] in bytes.as_chunks::<2>().0 {
        let nibble = |byte: u8| match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            _ => None,
        };
        decoded.push((nibble(*high)? << 4) | nibble(*low)?);
    }
    Some(decoded)
}

// ------------------------------------------------------------------- 工具

/// `UploadAuthMiddleware`（复用 `http_access_token`，失败 403）。
fn upload_authorized(state: &AppState, request: &Request) -> bool {
    let expected = state.config().snapshot().get_access_token().to_owned();
    if expected.is_empty() {
        return true;
    }
    let header_token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(|raw| raw.replacen("Bearer ", "", 1))
        .unwrap_or_default();
    let query_token = first(&query_pairs(request.uri()), "access_token")
        .unwrap_or("")
        .to_owned();
    (header_token == expected) || (query_token == expected)
}

/// 取表单字段值。
///
/// 请求体超限（[`MAX_FORM_BYTES`] 的 `DefaultBodyLimit`）**不能**再降级成空字段：下游会把空
/// base64 写成一个空文件再回 200（静默假成功）。超限显式报 413；其余解析错误仍按 Gin
/// `PostForm` 忽略（降级成空字段，由下游回各自的 400）。
async fn form_field(
    request: Request,
    state: &AppState,
    field: &str,
) -> Result<String, FormFieldError> {
    let is_multipart = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("multipart/form-data"));
    if is_multipart {
        let mut multipart = match Multipart::from_request(request, state).await {
            Ok(multipart) => multipart,
            Err(rejection) => return ignore_unless_too_large(rejection.status()),
        };
        loop {
            match multipart.next_field().await {
                Ok(Some(part)) => {
                    let name = part.name().map(str::to_owned);
                    if name.as_deref() != Some(field) {
                        continue;
                    }
                    return Ok(part.text().await.unwrap_or_default());
                }
                Ok(None) => return Ok(String::new()),
                // 超限是在读流的中途才发现的（分块传输没有 Content-Length），同样不能吞。
                Err(err) => return ignore_unless_too_large(err.status()),
            }
        }
    }
    // Gin `PostForm` 忽略解析错误，这里同样降级成空字段（下游回各自 400）
    match Form::<HashMap<String, String>>::from_request(request, state).await {
        Ok(Form(fields)) => Ok(fields.get(field).cloned().unwrap_or_default()),
        Err(rejection) => ignore_unless_too_large(rejection.status()),
    }
}

/// 取字段时唯一的硬错误：请求体超限（其余提取器错误按 Gin `PostForm` 忽略）。
enum FormFieldError {
    /// `DefaultBodyLimit` 被突破。
    TooLarge,
}

/// 提取器拒绝时的统一口径：413 走显式错误，其余按 Gin 的老行为吞成空字段。
fn ignore_unless_too_large(status: StatusCode) -> Result<String, FormFieldError> {
    if status == StatusCode::PAYLOAD_TOO_LARGE {
        Err(FormFieldError::TooLarge)
    } else {
        Ok(String::new())
    }
}

/// 超限请求体的显式响应（Go 没有对应文案：Gin 靠 `MaxMultipartMemory` 之外的读错误，
/// 原实现则把它吞成了 200 空文件）。
fn body_too_large() -> Response {
    json_error(StatusCode::PAYLOAD_TOO_LARGE, "request body too large")
}

fn decode_base64(encoded: &str) -> Result<Vec<u8>, base64::DecodeError> {
    let cleaned: String = encoded
        .chars()
        .filter(|ch| *ch != '\r' && *ch != '\n')
        .collect();
    BASE64.decode(cleaned.as_bytes())
}

fn query_pairs(uri: &Uri) -> Vec<(String, String)> {
    Query::<Vec<(String, String)>>::try_from_uri(uri)
        .map(|Query(pairs)| pairs)
        .unwrap_or_default()
}

fn first<'a>(pairs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
}

fn parse_param(pairs: &[(String, String)], key: &str) -> Option<i64> {
    first(pairs, key)?.parse::<i64>().ok()
}

fn peer_ip(request: &Request) -> Option<IpAddr> {
    request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(address)| address.ip())
}

/// Go `net.IP.String()` 对 IPv4-mapped IPv6 会打印成 IPv4。
fn peer_key(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => v6
            .to_ipv4_mapped()
            .map_or_else(|| v6.to_string(), |v4| v4.to_string()),
    }
}

/// 每分钟每对端 IP 的请求数上限（Go `RateLimiter.CheckAndUpdateRateLimit`）。
fn allow_request(peer: &Option<IpAddr>, max_requests: i64) -> bool {
    static COUNTS: LazyLock<Mutex<HashMap<String, Vec<Instant>>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));
    let key = peer.map_or_else(|| "unknown".to_owned(), peer_key);
    let now = Instant::now();
    let mut counts = COUNTS.lock().unwrap_or_else(PoisonError::into_inner);
    let stamps = counts.entry(key).or_default();
    stamps.push(now);
    stamps.retain(|stamp| now.duration_since(*stamp) <= RATE_WINDOW);
    stamps.len() as i64 <= max_requests
}

fn md5_hex(bytes: &[u8]) -> String {
    let digest = Md5::digest(bytes);
    let mut hex = String::with_capacity(32);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}

fn json_error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

enum DecodeImageError {
    Base64,
    Format,
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::Body;
    use gensokyo_config::ConfigManager;
    use gensokyo_media::{BoxFuture, HostingTransport, UploadRequest, UploadResponse};
    use gensokyo_storage::IdMapStore;
    use gensokyo_storage::idmap::IdMapOptions;
    use serde_json::Value;
    use tempfile::TempDir;
    use tower::ServiceExt;

    use super::sign_payload;
    use crate::hosting::HostingRuntime;
    use crate::state::AppState;

    /// 路由用例默认覆盖「本地 `/getid` 挂载」这一支（等价 `mount_getid=true`）；
    /// 接管后的隐藏行为在 `getid_route_follows_idmap_grpc_takeover` 里直接走 `super::router`。
    fn router(state: AppState) -> axum::Router {
        super::router(state, true)
    }

    /// 测试侧自己算签名，避免用被测代码当 oracle。
    fn hmac_hex(key: &[u8], data: &[u8]) -> String {
        let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key);
        let tag = ring::hmac::sign(&key, data);
        let mut hex = String::with_capacity(tag.as_ref().len() * 2);
        for byte in tag.as_ref() {
            use std::fmt::Write as _;
            let _ = write!(hex, "{byte:02x}");
        }
        hex
    }

    /// 1x1 PNG（真实文件，含 CRC）——`image_size` 读得到 1×1。
    const PNG_1_1_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==";
    const LOCAL_PEER: &str = "127.0.0.1:5000";

    fn manager(directory: &TempDir, yaml: &str) -> Arc<ConfigManager> {
        let path = directory.path().join("config.yml");
        std::fs::write(&path, format!("version: 1\nsettings:\n{yaml}")).expect("写测试配置失败");
        Arc::new(ConfigManager::new(&path).expect("加载配置失败"))
    }

    fn idmap(directory: &TempDir) -> Arc<IdMapStore> {
        let options = IdMapOptions::new(
            directory.path().join("identity.redb"),
            directory.path().join("messages.redb"),
        )
        .with_app_id("102000001");
        Arc::new(IdMapStore::open(options).expect("打开 idmap"))
    }

    async fn call(app: axum::Router, request: axum::extract::Request) -> (u16, Value) {
        let response = app.oneshot(request).await.expect("调用路由失败");
        let status = response.status().as_u16();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("读取响应体");
        let value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        (status, value)
    }

    fn with_peer(mut request: axum::extract::Request, peer: &str) -> axum::extract::Request {
        let address: std::net::SocketAddr = peer.parse().expect("对端地址");
        request
            .extensions_mut()
            .insert(axum::extract::ConnectInfo(address));
        request
    }

    fn get_request(uri: &str, peer: &str) -> axum::extract::Request {
        let request = axum::extract::Request::builder()
            .method("GET")
            .uri(uri)
            .body(Body::empty())
            .expect("构造请求");
        with_peer(request, peer)
    }

    /// multipart 表单（base64 里的 `+` `/` `=` 不经过 URL 编码）。
    fn multipart_request(
        uri: &str,
        field: &str,
        value: &str,
        peer: &str,
    ) -> axum::extract::Request {
        let boundary = "gensokyo-test-boundary";
        let body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{field}\"\r\n\r\n{value}\r\n--{boundary}--\r\n"
        );
        let request = axum::extract::Request::builder()
            .method("POST")
            .uri(uri)
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(body))
            .expect("构造 multipart 请求");
        with_peer(request, peer)
    }

    fn post_form(uri: &str, body: &str, peer: &str) -> axum::extract::Request {
        let request = axum::extract::Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body.to_owned()))
            .expect("构造请求");
        with_peer(request, peer)
    }

    /// `./channel_temp/` 是相对进程工作目录的，handler 级用例串行切换 CWD。
    static CWD: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[tokio::test]
    #[allow(
        clippy::await_holding_lock,
        reason = "整个用例独占进程工作目录，必须跨 await 持有串行锁"
    )]
    async fn upload_pic_saves_hash_named_file_and_returns_static_url() {
        let _guard = CWD
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let directory = TempDir::new().expect("临时目录");
        let previous = std::env::current_dir().expect("读取工作目录");
        std::env::set_current_dir(directory.path()).expect("切换工作目录");

        let state = AppState::with_builtin_handlers(manager(
            &directory,
            "  server_dir: example.com\n  port: \"8080\"\n  frp_port: \"0\"\n  image_limit: 100\n",
        ));
        let result = call(
            router(state),
            multipart_request("/uploadpic", "base64Image", PNG_1_1_BASE64, LOCAL_PEER),
        )
        .await;

        let files: Vec<String> = std::fs::read_dir(directory.path().join("channel_temp"))
            .expect("channel_temp 应存在")
            .map(|entry| entry.expect("目录项").file_name().into_string().unwrap())
            .collect();
        std::env::set_current_dir(previous).expect("恢复工作目录");

        assert_eq!(result.0, 200, "{:?}", result.1);
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with(".png"), "{files:?}");
        assert_eq!(
            result.1["url"],
            format!("http://example.com:8080/channel_temp/{}", files[0])
        );
    }

    #[tokio::test]
    #[allow(
        clippy::await_holding_lock,
        reason = "整个用例独占进程工作目录，必须跨 await 持有串行锁"
    )]
    async fn upload_pic_reports_go_error_shapes() {
        let _guard = CWD
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let directory = TempDir::new().expect("临时目录");
        let previous = std::env::current_dir().expect("读取工作目录");
        std::env::set_current_dir(directory.path()).expect("切换工作目录");

        let state = AppState::with_builtin_handlers(manager(
            &directory,
            "  server_dir: example.com\n  image_limit: 100\n",
        ));
        let invalid_base64 = call(
            router(state.clone()),
            multipart_request("/uploadpic", "base64Image", "not base64!", LOCAL_PEER),
        )
        .await;
        assert_eq!(invalid_base64.0, 400);
        assert_eq!(invalid_base64.1["error"], "invalid base64 data");

        let not_an_image = call(
            router(state.clone()),
            multipart_request("/uploadpic", "base64Image", "aGVsbG8=", LOCAL_PEER),
        )
        .await;
        assert_eq!(not_an_image.0, 400);
        assert_eq!(not_an_image.1["error"], "undefined picture format1");

        let no_server =
            AppState::with_builtin_handlers(manager(&directory, "  image_limit: 100\n"));
        let missing_server = call(
            router(no_server),
            multipart_request(
                "/uploadpic",
                "base64Image",
                PNG_1_1_BASE64,
                "127.0.0.2:5000",
            ),
        )
        .await;
        assert_eq!(missing_server.0, 500);
        assert_eq!(
            missing_server.1["error"],
            "server address is not configured"
        );
        std::env::set_current_dir(previous).expect("恢复工作目录");
    }

    #[tokio::test]
    async fn upload_auth_rejects_with_403_like_go() {
        let directory = TempDir::new().expect("临时目录");
        let state = AppState::with_builtin_handlers(manager(
            &directory,
            "  server_dir: example.com\n  image_limit: 100\n  http_access_token: s3cret\n",
        ));
        let missing = call(
            router(state.clone()),
            post_form("/uploadpic", "base64Image=", "127.0.0.3:5000"),
        )
        .await;
        assert_eq!(missing.0, 403);
        assert_eq!(missing.1["error"], "鉴权失败");

        let wrong = call(
            router(state.clone()),
            post_form(
                "/uploadpic?access_token=wrong",
                "base64Image=",
                "127.0.0.3:5000",
            ),
        )
        .await;
        assert_eq!(wrong.0, 403);

        // 正确 token 放行，落到业务校验（空 base64 解码成功但读不到图片头 → Go 的 format1）
        let allowed = call(
            router(state),
            post_form(
                "/uploadpic?access_token=s3cret",
                "base64Image=",
                "127.0.0.3:5000",
            ),
        )
        .await;
        assert_eq!(allowed.0, 400);
        assert_eq!(allowed.1["error"], "undefined picture format1");
    }

    #[tokio::test]
    #[allow(
        clippy::await_holding_lock,
        reason = "整个用例独占进程工作目录，必须跨 await 持有串行锁"
    )]
    async fn upload_record_saves_silk_and_returns_static_url() {
        let _guard = CWD
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let directory = TempDir::new().expect("临时目录");
        let previous = std::env::current_dir().expect("读取工作目录");
        std::env::set_current_dir(directory.path()).expect("切换工作目录");

        // frp_port 非 0：Go 的语音 handler 不读它（图片 handler 会读）
        let state = AppState::with_builtin_handlers(manager(
            &directory,
            "  server_dir: example.com\n  port: \"8080\"\n  frp_port: \"9000\"\n  image_limit: 100\n",
        ));
        let result = call(
            router(state.clone()),
            multipart_request("/uploadrecord", "base64Record", "aGVsbG8=", LOCAL_PEER),
        )
        .await;
        let saved = directory
            .path()
            .join("channel_temp/5d41402abc4b2a76b9719d911017c592.silk");
        let contents = std::fs::read(&saved).expect("语音文件应落盘");

        // 同名重复上传不再写盘（Go 的 os.Stat 跳过）
        let repeated = call(
            router(state),
            multipart_request("/uploadrecord", "base64Record", "aGVsbG8=", LOCAL_PEER),
        )
        .await;
        let count = std::fs::read_dir(directory.path().join("channel_temp"))
            .expect("channel_temp 应存在")
            .count();
        std::env::set_current_dir(previous).expect("恢复工作目录");

        assert_eq!(result.0, 200, "{:?}", result.1);
        assert_eq!(contents, b"hello");
        assert_eq!(
            result.1["url"],
            "http://example.com:8080/channel_temp/5d41402abc4b2a76b9719d911017c592.silk"
        );
        assert_eq!(repeated.0, 200, "{:?}", repeated.1);
        assert_eq!(repeated.1["url"], result.1["url"]);
        assert_eq!(count, 1);
    }

    #[tokio::test]
    #[allow(
        clippy::await_holding_lock,
        reason = "整个用例独占进程工作目录，必须跨 await 持有串行锁"
    )]
    async fn upload_record_reports_go_error_shapes_and_ignores_oss_type() {
        let _guard = CWD
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let directory = TempDir::new().expect("临时目录");
        let previous = std::env::current_dir().expect("读取工作目录");
        std::env::set_current_dir(directory.path()).expect("切换工作目录");

        // oss_type=1 且无任何 OSS 凭据：Go 的语音链不碰 oss（死代码），仍走本地
        let state = AppState::with_builtin_handlers(manager(
            &directory,
            "  server_dir: example.com\n  port: \"8080\"\n  image_limit: 100\n  oss_type: 1\n",
        ));
        let invalid = call(
            router(state.clone()),
            multipart_request(
                "/uploadrecord",
                "base64Record",
                "not base64!",
                "127.0.0.2:5000",
            ),
        )
        .await;

        // 空 base64Record：Go 解码成功 → 写一个空文件 → 200
        let empty = call(
            router(state),
            post_form("/uploadrecord", "base64Record=", "127.0.0.2:5000"),
        )
        .await;
        let empty_size = std::fs::metadata(
            directory
                .path()
                .join("channel_temp/d41d8cd98f00b204e9800998ecf8427e.silk"),
        )
        .map(|meta| meta.len())
        .ok();

        // port=443 → https（Go 只看端口与 force_ssl）
        let secure = AppState::with_builtin_handlers(manager(
            &directory,
            "  server_dir: example.com\n  port: \"443\"\n  image_limit: 100\n",
        ));
        let https = call(
            router(secure),
            multipart_request("/uploadrecord", "base64Record", "aGk=", "127.0.0.4:5000"),
        )
        .await;

        // 未配置 server_dir：Go 先落盘再报 500
        let no_server = AppState::with_builtin_handlers(manager(
            &directory,
            "  port: \"8080\"\n  image_limit: 100\n",
        ));
        let missing_server = call(
            router(no_server),
            multipart_request("/uploadrecord", "base64Record", "aGk=", "127.0.0.6:5000"),
        )
        .await;
        std::env::set_current_dir(previous).expect("恢复工作目录");

        assert_eq!(invalid.0, 400);
        assert_eq!(invalid.1["error"], "invalid base64 data");
        assert_eq!(empty.0, 200, "{:?}", empty.1);
        assert_eq!(
            empty.1["url"],
            "http://example.com:8080/channel_temp/d41d8cd98f00b204e9800998ecf8427e.silk"
        );
        assert_eq!(empty_size, Some(0));
        assert!(https.1["url"].as_str().unwrap().starts_with("https://"));
        assert_eq!(missing_server.0, 500);
        assert_eq!(
            missing_server.1["error"],
            "server address is not configured"
        );
    }

    /// 超过 `DefaultBodyLimit`（32 MiB）的请求体必须显式 413：原实现把提取器错误吞成空字段，
    /// 下游据此写出 0 字节 `.silk` 再回 200（P44 评审实测的静默假成功）。
    #[tokio::test]
    #[allow(
        clippy::await_holding_lock,
        reason = "整个用例独占进程工作目录，必须跨 await 持有串行锁"
    )]
    async fn upload_record_rejects_oversized_body_instead_of_faking_success() {
        let _guard = CWD
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let directory = TempDir::new().expect("临时目录");
        let previous = std::env::current_dir().expect("读取工作目录");
        std::env::set_current_dir(directory.path()).expect("切换工作目录");

        let state = AppState::with_builtin_handlers(manager(
            &directory,
            "  server_dir: example.com\n  port: \"8080\"\n  image_limit: 100\n",
        ));
        // 合法 multipart 前缀 + 超限字段体：上限要读到 32 MiB 才会被触发，分块传输（没有
        // Content-Length）走的是同一条路径。
        let boundary = "gensokyo-test-boundary";
        let mut body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"base64Record\"\r\n\r\n"
        )
        .into_bytes();
        body.resize(body.len() + super::MAX_FORM_BYTES + 1, b'a');
        let request = axum::extract::Request::builder()
            .method("POST")
            .uri("/uploadrecord")
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(body))
            .expect("构造超限请求");

        let result = call(router(state), with_peer(request, LOCAL_PEER)).await;
        let written = std::fs::read_dir(directory.path().join("channel_temp"))
            .map_or(0, |entries| entries.count());
        std::env::set_current_dir(previous).expect("恢复工作目录");

        assert_eq!(result.0, 413, "{:?}", result.1);
        assert_eq!(result.1["error"], "request body too large");
        assert_eq!(written, 0, "超限请求不得落任何文件");
    }

    #[tokio::test]
    #[allow(
        clippy::await_holding_lock,
        reason = "整个用例独占进程工作目录，必须跨 await 持有串行锁"
    )]
    async fn upload_record_auth_rejects_with_403_like_go() {
        let _guard = CWD
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let directory = TempDir::new().expect("临时目录");
        let previous = std::env::current_dir().expect("读取工作目录");
        std::env::set_current_dir(directory.path()).expect("切换工作目录");

        let state = AppState::with_builtin_handlers(manager(
            &directory,
            "  server_dir: example.com\n  port: \"8080\"\n  image_limit: 100\n  http_access_token: s3cret\n",
        ));
        let missing = call(
            router(state.clone()),
            post_form("/uploadrecord", "base64Record=", "127.0.0.5:5000"),
        )
        .await;
        let wrong = call(
            router(state.clone()),
            post_form(
                "/uploadrecord?access_token=wrong",
                "base64Record=",
                "127.0.0.5:5000",
            ),
        )
        .await;
        let allowed = call(
            router(state),
            post_form(
                "/uploadrecord?access_token=s3cret",
                "base64Record=aGk=",
                "127.0.0.5:5000",
            ),
        )
        .await;
        std::env::set_current_dir(previous).expect("恢复工作目录");

        assert_eq!(missing.0, 403);
        assert_eq!(missing.1["error"], "鉴权失败");
        assert_eq!(wrong.0, 403);
        assert_eq!(allowed.0, 200, "{:?}", allowed.1);
    }

    #[tokio::test]
    async fn upload_record_rate_limits_like_go() {
        let directory = TempDir::new().expect("临时目录");
        let state = AppState::with_builtin_handlers(manager(
            &directory,
            "  server_dir: example.com\n  port: \"8080\"\n  image_limit: 0\n",
        ));
        let rejected = call(
            router(state),
            post_form("/uploadrecord", "base64Record=", "127.0.0.9:5000"),
        )
        .await;
        assert_eq!(rejected.0, 429);
        assert_eq!(rejected.1["error"], "rate limit exceeded");
    }

    /// 记录请求的假传输：单次上传成功（ChatGLM 的响应形状）。
    #[derive(Debug, Default)]
    struct FakeTransport {
        calls: std::sync::Mutex<Vec<UploadRequest>>,
    }

    impl HostingTransport for FakeTransport {
        fn send(
            &self,
            request: UploadRequest,
        ) -> BoxFuture<'_, gensokyo_media::Result<UploadResponse>> {
            self.calls.lock().expect("锁").push(request);
            Box::pin(async {
                Ok(UploadResponse {
                    status: 200,
                    body: br#"{"code":0,"result":{"file_url":"https://chatglm.cn/files/x.png"}}"#
                        .to_vec(),
                })
            })
        }
    }

    #[tokio::test]
    async fn upload_pic_v2_uploads_through_hosting_and_returns_dimensions() {
        let directory = TempDir::new().expect("临时目录");
        let transport = Arc::new(FakeTransport::default());
        let state = AppState::with_builtin_handlers(manager(
            &directory,
            "  oss_type: 7\n  image_limit: 100\n",
        ))
        .with_hosting(Arc::new(HostingRuntime::with_transport(transport.clone())));
        let (status, body) = call(
            router(state),
            multipart_request(
                "/uploadpicv2",
                "base64Image",
                PNG_1_1_BASE64,
                "127.0.0.4:5000",
            ),
        )
        .await;

        assert_eq!(status, 200, "{body:?}");
        assert_eq!(body["url"], "https://chatglm.cn/files/x.png");
        assert_eq!(body["width"], 1);
        assert_eq!(body["height"], 1);

        let calls = transport.calls.lock().expect("锁");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].url, gensokyo_media::hosting::chatglm::ENDPOINT);
        assert!(
            calls[0]
                .headers
                .iter()
                .any(|(name, value)| name == "X-File-Mime" && value == "image/png")
        );
        let payload = calls[0].body.as_ref().expect("有请求体");
        assert!(payload.content_type.starts_with("multipart/form-data"));
        let png = gensokyo_media::decode_base64(PNG_1_1_BASE64).expect("PNG base64");
        assert!(
            payload.bytes.windows(png.len()).any(|window| window == png),
            "请求体应包含图片字节"
        );
    }

    #[tokio::test]
    async fn upload_pic_v2_degrades_without_credentials_or_valid_oss_type() {
        let directory = TempDir::new().expect("临时目录");
        let missing_credentials = AppState::with_builtin_handlers(manager(
            &directory,
            "  oss_type: 2\n  image_limit: 100\n",
        ));
        let (status, body) = call(
            router(missing_credentials),
            multipart_request(
                "/uploadpicv2",
                "base64Image",
                PNG_1_1_BASE64,
                "127.0.0.5:5000",
            ),
        )
        .await;
        assert_eq!(status, 500);
        assert!(
            body["error"]
                .as_str()
                .is_some_and(|message| !message.is_empty()),
            "{body:?}"
        );

        let invalid = AppState::with_builtin_handlers(manager(
            &directory,
            "  oss_type: 99\n  image_limit: 100\n",
        ));
        let (status, body) = call(
            router(invalid),
            multipart_request(
                "/uploadpicv2",
                "base64Image",
                PNG_1_1_BASE64,
                "127.0.0.6:5000",
            ),
        )
        .await;
        assert_eq!(status, 500);
        assert_eq!(body["error"], "invalid oss_type");

        let empty = AppState::with_builtin_handlers(manager(
            &directory,
            "  oss_type: 7\n  image_limit: 100\n",
        ));
        let (status, body) = call(
            router(empty),
            post_form("/uploadpicv2", "x=1", "127.0.0.7:5000"),
        )
        .await;
        assert_eq!(status, 400);
        assert_eq!(body["error"], "either base64Image or url is required");
    }

    fn getid_state(directory: &TempDir) -> AppState {
        AppState::with_builtin_handlers(manager(
            directory,
            "  server_dir: example.com\n  port: \"8080\"\n  lotus_password: lotus-pass\n",
        ))
        .with_idmap(idmap(directory))
    }

    fn signed_uri(extra: &str, timestamp: i64, password: &str) -> String {
        let uri: axum::http::Uri = format!("/getid?{extra}").parse().expect("query");
        let pairs: Vec<(String, String)> =
            axum::extract::Query::<Vec<(String, String)>>::try_from_uri(&uri)
                .expect("解析 query")
                .0;
        let payload = sign_payload("/getid", &pairs, &timestamp.to_string());
        let token = hmac_hex(password.as_bytes(), payload.as_bytes());
        format!("/getid?{extra}&timestamp={timestamp}&token={token}")
    }

    #[test]
    fn sign_payload_follows_go_sorted_query_shape() {
        let pairs = vec![
            ("type".to_owned(), "7".to_owned()),
            ("id".to_owned(), "1".to_owned()),
            ("token".to_owned(), "x".to_owned()),
            ("timestamp".to_owned(), "9".to_owned()),
        ];
        assert_eq!(
            sign_payload("/getid", &pairs, "1700000000"),
            "/getid?id=1&type=7&timestamp=1700000000"
        );
        // Go 在没有任何业务参数时也补 "&timestamp="（没有 "?"）
        assert_eq!(
            sign_payload("/getid", &[], "1700000000"),
            "/getid&timestamp=1700000000"
        );
    }

    #[tokio::test]
    async fn getid_auth_matches_go_401_and_403_shapes() {
        let directory = TempDir::new().expect("临时目录");
        let state = getid_state(&directory);

        let missing_token = call(
            router(state.clone()),
            get_request("/getid?type=1", LOCAL_PEER),
        )
        .await;
        assert_eq!(missing_token.0, 401);
        assert_eq!(missing_token.1["error"], "missing token");

        let now = super::unix_now();
        let missing_timestamp = call(
            router(state.clone()),
            get_request(
                &format!("/getid?type=1&token={}", "a".repeat(64)),
                LOCAL_PEER,
            ),
        )
        .await;
        assert_eq!(missing_timestamp.0, 401);
        assert_eq!(missing_timestamp.1["error"], "missing timestamp");

        let invalid_timestamp = call(
            router(state.clone()),
            get_request(
                &format!("/getid?type=1&timestamp=abc&token={}", "a".repeat(64)),
                LOCAL_PEER,
            ),
        )
        .await;
        assert_eq!(invalid_timestamp.0, 401);
        assert_eq!(invalid_timestamp.1["error"], "invalid timestamp");

        let expired = call(
            router(state.clone()),
            get_request(
                &signed_uri("type=1&id=openid:a", now - 301, "lotus-pass"),
                LOCAL_PEER,
            ),
        )
        .await;
        assert_eq!(expired.0, 401);
        assert_eq!(expired.1["error"], "timestamp expired");

        let tampered = call(
            router(state.clone()),
            get_request(
                // 用 type=1 的签名配 type=2 的请求
                &signed_uri("type=1&id=openid:a", now, "lotus-pass").replace("type=1", "type=2"),
                LOCAL_PEER,
            ),
        )
        .await;
        assert_eq!(tampered.0, 401);
        assert_eq!(tampered.1["error"], "unauthorized");

        // 无密码时只允许回环
        let remote_directory = TempDir::new().expect("临时目录");
        let no_password = AppState::with_builtin_handlers(manager(
            &remote_directory,
            "  server_dir: example.com\n",
        ))
        .with_idmap(idmap(&remote_directory));
        let remote = call(
            router(no_password.clone()),
            get_request("/getid?type=1", "203.0.113.9:5000"),
        )
        .await;
        assert_eq!(remote.0, 403);
        assert_eq!(remote.1["error"], "仅允许本地访问");

        let local = call(router(no_password), get_request("/getid?type=", LOCAL_PEER)).await;
        assert_eq!(local.0, 400);
        assert_eq!(local.1["error"], "invalid type");
    }

    #[tokio::test]
    async fn getid_dispatches_every_supported_type() {
        let directory = TempDir::new().expect("临时目录");
        let state =
            AppState::with_builtin_handlers(manager(&directory, "  server_dir: example.com\n"))
                .with_idmap(idmap(&directory));

        let (status, body) = call(
            router(state.clone()),
            get_request("/getid?type=1&id=openid:alice", LOCAL_PEER),
        )
        .await;
        assert_eq!(status, 200);
        let row = body["row"].as_i64().expect("row");

        let (status, body) = call(
            router(state.clone()),
            get_request(&format!("/getid?type=2&id={row}"), LOCAL_PEER),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(body["id"], "openid:alice");

        let (status, body) = call(
            router(state.clone()),
            get_request(&format!("/getid?type=2&id={}", row + 9999), LOCAL_PEER),
        )
        .await;
        assert_eq!(status, 404);
        assert_eq!(body["error"], "ID not found");

        let (status, body) = call(
            router(state.clone()),
            get_request("/getid?type=6&virtualValue=0", LOCAL_PEER),
        )
        .await;
        assert_eq!(status, 500, "{body:?}");

        let (status, body) = call(
            router(state.clone()),
            get_request("/getid?type=7&id=openid:alice", LOCAL_PEER),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(body["virtual"], row.to_string());

        let (status, body) = call(
            router(state.clone()),
            get_request("/getid?type=13&id=openid:bob", LOCAL_PEER),
        )
        .await;
        assert_eq!(status, 200);
        assert!(body["row"].as_i64().is_some());

        let (status, body) = call(
            router(state.clone()),
            get_request("/getid?type=3&id=group&subtype=name&value=demo", LOCAL_PEER),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(body["status"], "success");

        let (status, body) = call(
            router(state.clone()),
            get_request("/getid?type=4&id=group&subtype=name", LOCAL_PEER),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(body["value"], "demo");

        let (status, body) = call(
            router(state.clone()),
            get_request("/getid?type=15&id=group&subtype=name", LOCAL_PEER),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(body["status"], "success");

        let (status, body) = call(
            router(state.clone()),
            get_request("/getid?type=16&id=GroupID%20MessageID", LOCAL_PEER),
        )
        .await;
        assert_eq!(status, 200);
        let message_row = body["row"].as_i64().expect("row");

        let (status, body) = call(
            router(state.clone()),
            get_request(&format!("/getid?type=17&id={message_row}"), LOCAL_PEER),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(body["id"], "GroupID MessageID");

        let (status, body) = call(
            router(state),
            get_request(
                &format!("/getid?type=17&id={}", message_row + 9999),
                LOCAL_PEER,
            ),
        )
        .await;
        assert_eq!(status, 404);
        assert_eq!(body["error"], "ID not found");
    }

    #[tokio::test]
    async fn getid_reports_go_error_and_quirk_shapes() {
        let directory = TempDir::new().expect("临时目录");
        let state = AppState::with_builtin_handlers(manager(&directory, "  server_dir: x\n"))
            .with_idmap(idmap(&directory));

        for (uri, expected_status, message) in [
            ("/getid?type=5", 400, "invalid oldRowValue"),
            (
                "/getid?type=8",
                400,
                "subid parameter is required for type 8",
            ),
            (
                "/getid?type=9",
                400,
                "subid parameter is required for type 9",
            ),
            (
                "/getid?type=10&id=1",
                400,
                "id and subid parameters are required for type 10",
            ),
            (
                "/getid?type=11&id=x&subid=1",
                400,
                "Invalid input values idOrRow",
            ),
            (
                "/getid?type=12&oldVirtualValue1=x",
                400,
                "Invalid input values oldVirtualValue1",
            ),
            (
                "/getid?type=18",
                400,
                "id parameter is required for type 18",
            ),
        ] {
            let (status, body) = call(router(state.clone()), get_request(uri, LOCAL_PEER)).await;
            assert_eq!(status, expected_status, "{uri}");
            assert!(
                body["error"]
                    .as_str()
                    .is_some_and(|text| text.contains(message)),
                "{uri} → {body:?}"
            );
        }

        // type 14：Go 的 `ids` 桶在现代 schema 下恒空，nil 切片序列化成 `null`
        let (status, body) = call(
            router(state.clone()),
            get_request("/getid?type=14&id=1", LOCAL_PEER),
        )
        .await;
        assert_eq!(status, 200, "{body:?}");
        assert_eq!(body, serde_json::json!({ "keys": null }));

        // Go switch 无 default：未知 type 是 200 空体
        let (status, body) = call(router(state), get_request("/getid?type=99", LOCAL_PEER)).await;
        assert_eq!(status, 200);
        assert_eq!(body, Value::Null);
    }

    #[tokio::test]
    async fn getid_rebind_types_follow_go() {
        let directory = TempDir::new().expect("临时目录");
        let state =
            AppState::with_builtin_handlers(manager(&directory, "  server_dir: example.com\n"))
                .with_idmap(idmap(&directory));

        let (_, body) = call(
            router(state.clone()),
            get_request("/getid?type=1&id=rebind-a", LOCAL_PEER),
        )
        .await;
        let first = body["row"].as_i64().expect("row");
        let (_, body) = call(
            router(state.clone()),
            get_request("/getid?type=1&id=rebind-b", LOCAL_PEER),
        )
        .await;
        let second = body["row"].as_i64().expect("row");
        let first_new = first + 100;

        // type 5：重绑定成功 → 200 {"status":"success"}
        let (status, body) = call(
            router(state.clone()),
            get_request(
                &format!("/getid?type=5&oldRowValue={first}&newRowValue={first_new}"),
                LOCAL_PEER,
            ),
        )
        .await;
        assert_eq!(status, 200, "{body:?}");
        assert_eq!(body["status"], "success");

        let (status, body) = call(
            router(state.clone()),
            get_request(
                &format!("/getid?type=6&virtualValue={first_new}"),
                LOCAL_PEER,
            ),
        )
        .await;
        assert_eq!(status, 200, "{body:?}");
        assert_eq!(body["real"], "rebind-a");

        // 旧 row 已下线：Go 该分支把存储错误一律映射成 500
        let (status, body) = call(
            router(state.clone()),
            get_request(&format!("/getid?type=6&virtualValue={first}"), LOCAL_PEER),
        )
        .await;
        assert_eq!(status, 500, "{body:?}");
        assert!(
            body["error"]
                .as_str()
                .is_some_and(|text| text.contains("键不存在")),
            "{body:?}"
        );

        // 目标已被占用 → 500（Go 的 `%v :已存在`）
        let (status, body) = call(
            router(state.clone()),
            get_request(
                &format!("/getid?type=5&oldRowValue={second}&newRowValue={first_new}"),
                LOCAL_PEER,
            ),
        )
        .await;
        assert_eq!(status, 500, "{body:?}");
        assert!(
            body["error"]
                .as_str()
                .is_some_and(|text| text.contains("已存在")),
            "{body:?}"
        );

        // 旧值查不到 → 500，正文与 Go 一致是 `不存在:<old>`（内部细节只进日志）
        let (status, body) = call(
            router(state.clone()),
            get_request("/getid?type=5&oldRowValue=999999&newRowValue=1", LOCAL_PEER),
        )
        .await;
        assert_eq!(status, 500, "{body:?}");
        assert_eq!(body["error"], "不存在:999999", "{body:?}");

        // type 12：两次顺序重绑定 → 200 文案
        let (status, body) = call(
            router(state.clone()),
            get_request(
                &format!(
                    "/getid?type=12&oldVirtualValue1={second}&newVirtualValue1={}\
                     &oldVirtualValue2={first_new}&newVirtualValue2={}",
                    second + 100,
                    first_new + 100
                ),
                LOCAL_PEER,
            ),
        )
        .await;
        assert_eq!(status, 200, "{body:?}");
        assert_eq!(body["message"], "Virtual values updated successfully");

        // type 12 存储错误 → 500（第一次就失败）
        let (status, body) = call(
            router(state),
            get_request(
                "/getid?type=12&oldVirtualValue1=1&newVirtualValue1=2\
                 &oldVirtualValue2=3&newVirtualValue2=4",
                LOCAL_PEER,
            ),
        )
        .await;
        assert_eq!(status, 500, "{body:?}");
        assert!(body["error"].as_str().is_some(), "{body:?}");
    }

    #[tokio::test]
    async fn getid_force_unbind_and_list_identities_follow_go() {
        let directory = TempDir::new().expect("临时目录");
        let state =
            AppState::with_builtin_handlers(manager(&directory, "  server_dir: example.com\n"))
                .with_idmap(idmap(&directory));

        // 19：空库 → 空数组（Go 的 nil 切片序列化成 null，这里给 []）
        let (status, body) = call(
            router(state.clone()),
            get_request("/getid?type=19", LOCAL_PEER),
        )
        .await;
        assert_eq!(status, 200, "{body:?}");
        assert_eq!(body["mappings"], Value::Array(Vec::new()));
        assert_eq!(body["count"], 0);
        assert!(
            body["timestamp"].as_i64().is_some_and(|value| value > 0),
            "{body:?}"
        );

        let openid = "E".repeat(32);
        let other = "F".repeat(32);
        let (status, body) = call(
            router(state.clone()),
            get_request(&format!("/getid?type=1&id={openid}"), LOCAL_PEER),
        )
        .await;
        assert_eq!(status, 200, "{body:?}");
        let row = body["row"].as_i64().expect("row");
        let (status, body) = call(
            router(state.clone()),
            get_request(&format!("/getid?type=1&id={other}"), LOCAL_PEER),
        )
        .await;
        assert_eq!(status, 200, "{body:?}");
        state
            .idmap()
            .expect("idmap 已装配")
            .store_user_name(row, "alice");

        // 19：按正表 key 升序（两个 OpenID 前缀相同，比较的是 raw 段）
        let (status, body) = call(
            router(state.clone()),
            get_request("/getid?type=19", LOCAL_PEER),
        )
        .await;
        assert_eq!(status, 200, "{body:?}");
        assert_eq!(body["count"], 2);
        assert_eq!(body["mappings"][0]["real_id"], openid);
        assert_eq!(body["mappings"][0]["virtual_id"], row);
        assert_eq!(body["mappings"][0]["username"], "alice");
        assert_eq!(body["mappings"][1]["real_id"], other);
        assert_eq!(body["mappings"][1]["username"], "");

        // 18：按 OpenID 解绑 → 正反查都查不到
        let (status, body) = call(
            router(state.clone()),
            get_request(&format!("/getid?type=18&id={openid}"), LOCAL_PEER),
        )
        .await;
        assert_eq!(status, 200, "{body:?}");
        assert_eq!(body["status"], "success");
        assert_eq!(body["unbound_count"], 1);
        let (status, body) = call(
            router(state.clone()),
            get_request(&format!("/getid?type=2&id={row}"), LOCAL_PEER),
        )
        .await;
        assert_eq!(status, 404, "{body:?}");
        assert_eq!(body["error"], "ID not found");
        let (status, _) = call(
            router(state.clone()),
            get_request(&format!("/getid?type=7&id={openid}"), LOCAL_PEER),
        )
        .await;
        assert_eq!(status, 500, "正表已删：lookup 失败按 Go 回 500");

        // 18：未知身份串 → Go 也回 200 + 0 条（幂等，该分支没有 404）
        let (status, body) = call(
            router(state.clone()),
            get_request("/getid?type=18&id=nobody", LOCAL_PEER),
        )
        .await;
        assert_eq!(status, 200, "{body:?}");
        assert_eq!(body["unbound_count"], 0);

        // 18：十进制按 vUIN 处理，反查不到 → 500（Go 该分支没有 404）
        let (status, body) = call(
            router(state.clone()),
            get_request("/getid?type=18&id=424242", LOCAL_PEER),
        )
        .await;
        assert_eq!(status, 500, "{body:?}");
        assert!(
            body["error"]
                .as_str()
                .is_some_and(|text| text.contains("424242")),
            "{body:?}"
        );
    }

    #[tokio::test]
    async fn getid_pro_types_pair_two_single_keys() {
        let directory = TempDir::new().expect("临时目录");
        let state =
            AppState::with_builtin_handlers(manager(&directory, "  server_dir: example.com\n"))
                .with_idmap(idmap(&directory));

        // 8：群真实 ID + 用户真实 ID → 两个 vUIN（Go `StoreIDv2Pro`）
        let (status, body) = call(
            router(state.clone()),
            get_request(
                "/getid?type=8&id=openid:group&subid=openid:member",
                LOCAL_PEER,
            ),
        )
        .await;
        assert_eq!(status, 200, "{body:?}");
        let group_row = body["row"].as_i64().expect("row");
        let member_row = body["subRow"].as_i64().expect("subRow");

        // 重复调用命中同一对 vUIN（Go `StoreIDv2` 的查不到才分配）
        let (status, body) = call(
            router(state.clone()),
            get_request(
                "/getid?type=8&id=openid:group&subid=openid:member",
                LOCAL_PEER,
            ),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(body["row"].as_i64(), Some(group_row));
        assert_eq!(body["subRow"].as_i64(), Some(member_row));

        // 9：两个 vUIN → 两个真实 ID（Go `RetrieveRowByIDv2Pro`）
        let (status, body) = call(
            router(state.clone()),
            get_request(
                &format!("/getid?type=9&id={group_row}&subid={member_row}"),
                LOCAL_PEER,
            ),
        )
        .await;
        assert_eq!(status, 200, "{body:?}");
        assert_eq!(body["id"], "openid:group");
        assert_eq!(body["subid"], "openid:member");

        // 10：两个真实 ID → 两个 vUIN 字符串（Go `RetrieveVirtualValuev2Pro`，只查不分配）
        let (status, body) = call(
            router(state.clone()),
            get_request(
                "/getid?type=10&id=openid:group&subid=openid:member",
                LOCAL_PEER,
            ),
        )
        .await;
        assert_eq!(status, 200, "{body:?}");
        assert_eq!(body["firstValue"], group_row.to_string());
        assert_eq!(body["secondValue"], member_row.to_string());

        // 11：两个 vUIN → 两个真实 ID（Go `RetrieveRealValuesv2Pro`）
        let (status, body) = call(
            router(state.clone()),
            get_request(
                &format!("/getid?type=11&id={group_row}&subid={member_row}"),
                LOCAL_PEER,
            ),
        )
        .await;
        assert_eq!(status, 200, "{body:?}");
        assert_eq!(body["firstRealValue"], "openid:group");
        assert_eq!(body["secondRealValue"], "openid:member");

        // 查不到一律 500（Go 把 err 直接映射成 500，不是 404）
        for uri in [
            format!("/getid?type=9&id=999999&subid={member_row}"),
            "/getid?type=10&id=openid:missing&subid=openid:member".to_owned(),
            format!("/getid?type=11&id=999999&subid={member_row}"),
        ] {
            let (status, body) = call(router(state.clone()), get_request(&uri, LOCAL_PEER)).await;
            assert_eq!(status, 500, "{uri} → {body:?}");
            assert!(
                body["error"].as_str().is_some_and(|text| !text.is_empty()),
                "{uri} → {body:?}"
            );
        }
    }

    /// gRPC idmap 服务端接管后本地 `/getid` 不再注册（Go `main.go:507`）；未接管时照常挂载。
    #[tokio::test]
    async fn getid_route_follows_idmap_grpc_takeover() {
        let directory = TempDir::new().expect("临时目录");
        let state = AppState::with_builtin_handlers(manager(
            &directory,
            "  server_dir: x\n  lotus: true\n  lotus_grpc: true\n",
        ));
        let (status, _) = call(
            router(state.clone()),
            get_request("/getid?type=", LOCAL_PEER),
        )
        .await;
        assert_eq!(status, 400, "未接管时应挂载 /getid（400 是缺参数）");

        let (status, body) = call(
            super::router(state, false),
            get_request("/getid?type=", LOCAL_PEER),
        )
        .await;
        assert_eq!(status, 404, "gRPC 接管后 /getid 不应存在: {body:?}");
    }
}

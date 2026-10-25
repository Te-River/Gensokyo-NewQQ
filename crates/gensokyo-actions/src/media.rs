//! 媒体信息 action：`get_image` / `get_record`。
//!
//! # 覆盖范围
//!
//! | action | Go 对应 | 上游端点 | 结果 |
//! |--------|---------|----------|------|
//! | `get_image` | 无（Go 未注册） | 官方无「按 `file` 取图片信息」端点 | 本地回填 `{"file": …}` |
//! | `get_record` | 无（Go 未注册） | 官方无「按 `file` 取语音信息」端点 | 本地回填 `{"file": …}` |
//!
//! # 语义
//!
//! go-cqhttp 的 `get_image` 会解码图片并给出宽高 / 尺寸；官方 v2 没有对应端点，本 crate 也拿不到
//! 媒体落盘句柄（上传在 `gensokyo-media` / runtime 侧），因此这里只做**确定性归一 + 原样回填**：
//!
//! - `file://` → 去掉 scheme 得到本地路径（`file:///C:/a.png` → `C:/a.png`）；
//! - 其它形态（`http(s)://`、绝对 / 相对路径、`base64://`）→ 原样返回。
//!
//! 不校验文件是否存在、不读文件、不解码 base64：伪造尺寸，或把 `base64://` 编造成一个假路径，
//! 都比「把入参原样告诉调用方」更难排查。`get_record` 的 `out_format` 一并忽略——没有可转码的
//! 源文件。
//!
//! # 参数
//!
//! `file` 必填（OneBot v11）；缺失 / 空串 / 非文本 → `retcode = 1400`。

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use gensokyo_core::{Error, Result};
use gensokyo_protocol::action::{ActionRequest, ActionResponse};
use gensokyo_protocol::{Handler, HandlerFuture};

use crate::params;
use crate::util::{bad_request, ok_data, params_object};

/// `file://` scheme（OneBot 客户端给本地文件时最常用的形态）。
const FILE_SCHEME: &str = "file://";

/// `get_image` / `get_record` 的响应数据：归一后的媒体引用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaFileData {
    /// 本地路径或远程 URL（原样回填，不做存在性校验）。
    pub file: String,
}

/// `file` 参数的归一规则：只拆 `file://`，其余形态原样保留。
fn normalize_file(raw: &str) -> String {
    match raw.strip_prefix(FILE_SCHEME) {
        Some(rest) => strip_windows_drive_prefix(rest),
        None => raw.to_owned(),
    }
}

/// `file:///C:/a.png` 去掉 scheme 后是 `/C:/a.png`：Windows 盘符前多出的 `/` 要去掉。
///
/// POSIX 的 `file:///home/a.png` 去前缀后是 `/home/a.png`，本就是合法绝对路径，保持原样。
fn strip_windows_drive_prefix(path: &str) -> String {
    let bytes = path.as_bytes();
    let is_drive_letter =
        bytes.len() >= 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b':';
    if is_drive_letter {
        path[1..].to_owned()
    } else {
        path.to_owned()
    }
}

/// `get_image` / `get_record` 的公共流程：取 `file` → 归一 → 回填。
fn plan_media_file(object: &Map<String, Value>) -> Result<MediaFileData> {
    let raw = params::optional_text(object, "file")?
        .ok_or_else(|| Error::Protocol("缺少 file".to_owned()))?;
    Ok(MediaFileData {
        file: normalize_file(&raw),
    })
}

/// `get_image`。
#[derive(Debug, Clone, Copy, Default)]
pub struct GetImageHandler;

impl GetImageHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        let object = match params_object(request) {
            Ok(object) => object,
            Err(err) => return Ok(bad_request(request, &err)),
        };
        match plan_media_file(object) {
            Ok(data) => ok_data(request, &data),
            Err(err) => Ok(bad_request(request, &err)),
        }
    }
}

impl Handler for GetImageHandler {
    fn name(&self) -> &'static str {
        "get_image"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

/// `get_record`。
#[derive(Debug, Clone, Copy, Default)]
pub struct GetRecordHandler;

impl GetRecordHandler {
    /// 构造（handler 无状态）。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    async fn run(&self, request: &ActionRequest<Value>) -> Result<ActionResponse<Value>> {
        let object = match params_object(request) {
            Ok(object) => object,
            Err(err) => return Ok(bad_request(request, &err)),
        };
        match plan_media_file(object) {
            Ok(data) => ok_data(request, &data),
            Err(err) => Ok(bad_request(request, &err)),
        }
    }
}

impl Handler for GetRecordHandler {
    fn name(&self) -> &'static str {
        "get_record"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(self.run(request))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{GetImageHandler, GetRecordHandler};
    use crate::handler::RETCODE_BAD_REQUEST;
    use crate::testing::{block_on, failed, ok_data, request, request_without_params};
    use gensokyo_protocol::Handler;
    use gensokyo_protocol::Status;

    #[test]
    fn image_strips_file_scheme_from_local_paths() {
        let handler = GetImageHandler::new();
        for (file, expected) in [
            ("file:///C:/tmp/a.png", "C:/tmp/a.png"),
            ("file://C:/tmp/a.png", "C:/tmp/a.png"),
            ("file:///home/a.png", "/home/a.png"),
        ] {
            let response = block_on(handler.handle(&request("get_image", json!({"file": file}))))
                .expect("本地回填不应失败");
            let data = ok_data(response);
            assert_eq!(data["file"], json!(expected), "file = {file}");
        }
        assert_eq!(handler.name(), "get_image");
    }

    #[test]
    fn record_keeps_urls_and_plain_paths_verbatim() {
        let handler = GetRecordHandler::new();
        for file in [
            "https://example.com/a.amr",
            "http://example.com/a.amr?x=1",
            "C:\\tmp\\a.amr",
            "records/a.amr",
            "base64://AAAA",
        ] {
            let response = block_on(handler.handle(&request("get_record", json!({"file": file}))))
                .expect("本地回填不应失败");
            let data = ok_data(response);
            assert_eq!(data["file"], json!(file));
        }
        assert_eq!(handler.name(), "get_record");
    }

    #[test]
    fn missing_file_fails_with_1400() {
        for handler in [
            &GetImageHandler::new() as &dyn Handler,
            &GetRecordHandler::new(),
        ] {
            for req in [
                request(handler.name(), json!({})),
                request(handler.name(), json!({"file": ""})),
                request(handler.name(), json!({"file": {"path": "a.png"}})),
                request_without_params(handler.name()),
            ] {
                let response = block_on(handler.handle(&req)).expect("参数错误也应是 Ok(失败响应)");
                let (status, retcode, data) = failed(response);
                assert_eq!(status, Status::Failed);
                assert_eq!(retcode, RETCODE_BAD_REQUEST);
                assert_eq!(data, None);
            }
        }
    }
}

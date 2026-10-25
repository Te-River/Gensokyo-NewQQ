//! 测试替身：记录上传请求、回放预设响应，并提供 multipart 断言工具。
//!
//! 只在 `cfg(test)` 下编译，供 `hosting` 各子模块的测试共用（避免三份 FakeTransport）。

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::error::{Error, Result};
use crate::fetch::BoxFuture;
use crate::hosting::transport::{HostingTransport, UploadRequest, UploadResponse};

/// 一次预设响应。
#[derive(Debug, Clone)]
pub(crate) enum Reply {
    /// 返回该响应（状态码由后端判定成败）。
    Response(UploadResponse),
    /// 模拟传输层失败。
    Error(String),
}

/// 记录所有请求、按顺序回放响应的传输实现。
#[derive(Debug, Clone, Default)]
pub(crate) struct FakeTransport {
    calls: Arc<Mutex<Vec<UploadRequest>>>,
    replies: Arc<Mutex<VecDeque<Reply>>>,
}

impl FakeTransport {
    /// 没有任何预设响应：每次调用都会得到传输错误。
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// 追加一个响应（可链式调用，按追加顺序消费）。
    pub(crate) fn respond(self, status: u16, body: &str) -> Self {
        self.replies
            .lock()
            .expect("replies 锁未中毒")
            .push_back(Reply::Response(UploadResponse {
                status,
                body: body.as_bytes().to_vec(),
            }));
        self
    }

    /// 追加一次传输失败。
    pub(crate) fn fail(self, message: &str) -> Self {
        self.replies
            .lock()
            .expect("replies 锁未中毒")
            .push_back(Reply::Error(message.to_owned()));
        self
    }

    /// 已记录的请求（按调用顺序）。
    pub(crate) fn calls(&self) -> Vec<UploadRequest> {
        self.calls.lock().expect("calls 锁未中毒").clone()
    }
}

impl HostingTransport for FakeTransport {
    fn send(&self, request: UploadRequest) -> BoxFuture<'_, Result<UploadResponse>> {
        let calls = Arc::clone(&self.calls);
        let replies = Arc::clone(&self.replies);
        Box::pin(async move {
            calls.lock().expect("calls 锁未中毒").push(request);
            match replies.lock().expect("replies 锁未中毒").pop_front() {
                Some(Reply::Response(response)) => Ok(response),
                Some(Reply::Error(message)) => Err(Error::Transport(message)),
                None => Err(Error::Transport(
                    "FakeTransport 没有更多预设响应".to_owned(),
                )),
            }
        })
    }
}

// ---------- multipart 断言工具 ----------

/// 解析后的 multipart 部件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Part {
    /// 字段名（`Content-Disposition` 的 `name`）。
    pub(crate) name: String,
    /// 文件名（`Content-Disposition` 的 `filename`）。
    pub(crate) filename: Option<String>,
    /// 部件的 `Content-Type`。
    pub(crate) content_type: Option<String>,
    /// 部件内容。
    pub(crate) data: Vec<u8>,
}

impl Part {
    /// 内容按 UTF-8 文本读取（图片字节请直接看 [`Part::data`]）。
    pub(crate) fn text(&self) -> String {
        String::from_utf8_lossy(&self.data).into_owned()
    }
}

/// 解析 multipart body，`content_type` 需是带 `boundary=` 的完整值。
pub(crate) fn parse_multipart(content_type: &str, body: &[u8]) -> Vec<Part> {
    let boundary = content_type
        .split("boundary=")
        .nth(1)
        .expect("multipart Content-Type 缺少 boundary")
        .trim()
        .trim_matches('"')
        .to_owned();
    let marker = format!("--{boundary}");

    let mut parts = Vec::new();
    for segment in split_on(body, marker.as_bytes()) {
        let segment = segment.strip_prefix(b"\r\n".as_slice()).unwrap_or(segment);
        if segment.is_empty() || segment.starts_with(b"--") {
            continue;
        }
        let Some(header_end) = find(segment, b"\r\n\r\n") else {
            continue;
        };
        let headers = &segment[..header_end];
        let data = &segment[header_end + 4..];
        let data = data.strip_suffix(b"\r\n".as_slice()).unwrap_or(data);
        parts.push(parse_part(headers, data));
    }
    parts
}

fn parse_part(headers: &[u8], data: &[u8]) -> Part {
    let mut name = String::new();
    let mut filename = None;
    let mut content_type = None;
    for line in headers.split(|byte| *byte == b'\n') {
        let line = String::from_utf8_lossy(line);
        let line = line.trim_end_matches('\r');
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key.trim().to_ascii_lowercase().as_str() {
            "content-disposition" => {
                name = attribute(value, "name").unwrap_or_default();
                filename = attribute(value, "filename");
            }
            "content-type" => content_type = Some(value.to_owned()),
            _ => {}
        }
    }
    Part {
        name,
        filename,
        content_type,
        data: data.to_vec(),
    }
}

/// 读 `Content-Disposition` 里的 `key="value"`，按 multipart 规则反转义。
fn attribute(header: &str, key: &str) -> Option<String> {
    let start = header.find(&format!("{key}=\""))? + key.len() + 2;
    let mut value = String::new();
    let mut chars = header[start..].chars();
    while let Some(character) = chars.next() {
        match character {
            '"' => return Some(value),
            '\\' => value.push(chars.next()?),
            character => value.push(character),
        }
    }
    None
}

fn split_on<'a>(haystack: &'a [u8], needle: &[u8]) -> Vec<&'a [u8]> {
    let mut segments = Vec::new();
    let mut start = 0;
    let mut index = 0;
    while index + needle.len() <= haystack.len() {
        if &haystack[index..index + needle.len()] == needle {
            segments.push(&haystack[start..index]);
            index += needle.len();
            start = index;
        } else {
            index += 1;
        }
    }
    segments.push(&haystack[start..]);
    segments
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    (0..=haystack.len().saturating_sub(needle.len()))
        .find(|&index| &haystack[index..index + needle.len()] == needle)
}

#[cfg(test)]
mod tests {
    use super::parse_multipart;
    use crate::hosting::multipart::MultipartBody;

    #[test]
    fn parses_fields_files_and_escapes() {
        let mut body = MultipartBody::new("b1");
        body.field("module", "ukaka");
        body.file("file", "a\"b.png", b"\x89PNG");
        let (content_type, bytes) = body.finish();

        let parts = parse_multipart(&content_type, &bytes);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].name, "module");
        assert_eq!(parts[0].text(), "ukaka");
        assert_eq!(parts[0].filename, None);
        assert_eq!(parts[1].name, "file");
        assert_eq!(parts[1].filename.as_deref(), Some("a\"b.png"));
        assert_eq!(
            parts[1].content_type.as_deref(),
            Some("application/octet-stream")
        );
        assert_eq!(parts[1].data, b"\x89PNG");
    }
}

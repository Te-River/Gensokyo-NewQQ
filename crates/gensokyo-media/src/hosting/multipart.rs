//! `multipart/form-data` 构造。
//!
//! 不启用 `reqwest` 的 `multipart` feature：那会拉进 `mime_guess`，而 M2b 期间
//! `Cargo.lock` 冻结（`[[package]] gensokyo-media` 的依赖边不允许变动）。Go 侧用的是
//! `mime/multipart.Writer`，线上格式由 RFC 7578 固定，手写一遍即可逐字节对上：
//!
//! ```text
//! --BOUNDARY\r\n
//! Content-Disposition: form-data; name="file"; filename="a.png"\r\n
//! Content-Type: application/octet-stream\r\n
//! \r\n
//! <bytes>\r\n
//! --BOUNDARY--\r\n
//! ```

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::sniff::{self, ImageKind};

/// 文件部件的 `Content-Type`：与 Go `multipart.Writer.CreateFormFile` 一致，恒为八位流；
/// 真实图片类型由 ChatGLM 的 `X-File-Mime` 头声明。
const FILE_PART_CONTENT_TYPE: &str = "application/octet-stream";
/// 清洗后文件名为空时的兜底。
const DEFAULT_FILENAME: &str = "image";
/// 嗅探失败时的扩展名（对齐 Go `detectExt` 的兜底分支）。
const DEFAULT_EXTENSION: &str = "jpg";

static BOUNDARY_SEQ: AtomicU64 = AtomicU64::new(0);

/// 生成一个 boundary：`gensokyo-<纳秒>-<序号>`，只含 RFC 2046 允许的字符。
///
/// 不含随机源：本 crate 没有 `rand` 依赖（Cargo.lock 冻结），纳秒 + 进程内序号在同一
/// 进程的连续请求之间已经足够唯一。
#[must_use]
pub fn generate_boundary() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let seq = BOUNDARY_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("gensokyo-{nanos:x}-{seq:x}")
}

/// 一个 multipart 请求体；部件按加入顺序写出。
#[derive(Debug)]
pub struct MultipartBody {
    boundary: String,
    buffer: Vec<u8>,
}

impl MultipartBody {
    /// 用给定 boundary 开始构造（测试用固定 boundary，生产用 [`generate_boundary`]）。
    #[must_use]
    pub fn new(boundary: impl Into<String>) -> Self {
        Self {
            boundary: boundary.into(),
            buffer: Vec::new(),
        }
    }

    /// 追加一个普通表单字段（值按 UTF-8 原样写出，不做百分号编码）。
    pub fn field(&mut self, name: &str, value: &str) -> &mut Self {
        self.start_part();
        self.buffer
            .extend_from_slice(b"Content-Disposition: form-data; name=\"");
        push_escaped(&mut self.buffer, name);
        self.buffer.extend_from_slice(b"\"\r\n\r\n");
        self.buffer.extend_from_slice(value.as_bytes());
        self.buffer.extend_from_slice(b"\r\n");
        self
    }

    /// 追加文件部件。
    pub fn file(&mut self, name: &str, filename: &str, data: &[u8]) -> &mut Self {
        self.start_part();
        self.buffer
            .extend_from_slice(b"Content-Disposition: form-data; name=\"");
        push_escaped(&mut self.buffer, name);
        self.buffer.extend_from_slice(b"\"; filename=\"");
        push_escaped(&mut self.buffer, filename);
        self.buffer.extend_from_slice(b"\"\r\nContent-Type: ");
        self.buffer
            .extend_from_slice(FILE_PART_CONTENT_TYPE.as_bytes());
        self.buffer.extend_from_slice(b"\r\n\r\n");
        self.buffer.extend_from_slice(data);
        self.buffer.extend_from_slice(b"\r\n");
        self
    }

    /// 当前 boundary（不含前导 `--`）。
    #[must_use]
    pub fn boundary(&self) -> &str {
        &self.boundary
    }

    /// 收尾并返回 `(Content-Type, body)`。
    #[must_use]
    pub fn finish(mut self) -> (String, Vec<u8>) {
        let content_type = format!("multipart/form-data; boundary={}", self.boundary);
        self.buffer.extend_from_slice(b"--");
        self.buffer.extend_from_slice(self.boundary.as_bytes());
        self.buffer.extend_from_slice(b"--\r\n");
        (content_type, self.buffer)
    }

    fn start_part(&mut self) {
        self.buffer.extend_from_slice(b"--");
        self.buffer.extend_from_slice(self.boundary.as_bytes());
        self.buffer.extend_from_slice(b"\r\n");
    }
}

/// 规整上传文件名：去掉目录部分与控制字符，并按嗅探到的真实格式修正扩展名。
///
/// 对齐 Go `ensureExt`：`photo.png` 配 JPEG 字节 → `photo.jpg`；`a.tar.gz` → `a.tar.png`
/// （Go 用 `strings.LastIndex(".")` 整段替换）。差异：Go 不动目录部分，本 crate 用
/// 文件名部分（避免把客户端的本地路径带进 `Content-Disposition`）。
#[must_use]
pub fn ensured_filename(filename: &str, data: &[u8]) -> String {
    let base = filename.rsplit(['/', '\\']).next().unwrap_or_default();
    let mut name: String = base.chars().filter(|ch| !ch.is_control()).collect();
    name = name.trim().to_owned();
    if name.is_empty() || name == "." || name == ".." {
        name = DEFAULT_FILENAME.to_owned();
    }
    let extension = sniff::sniff_image(data).map_or(DEFAULT_EXTENSION, ImageKind::extension);
    if name.to_lowercase().ends_with(&format!(".{extension}")) {
        return name;
    }
    if let Some(index) = name.rfind('.') {
        name.truncate(index);
    }
    name.push('.');
    name.push_str(extension);
    name
}

/// RFC 3986 查询参数值编码：保留 unreserved 字符，其余按 UTF-8 字节百分号转义。
///
/// 与 Go `url.Values.Encode()` 的差异：空格编码为 `%20` 而不是 `+`（签名服务按 RFC 3986
/// 解码，`+` 会被当字面量）。
#[must_use]
pub fn encode_query_value(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(char::from(byte));
            }
            _ => {
                out.push('%');
                out.push(char::from(HEX[usize::from(byte >> 4)]));
                out.push(char::from(HEX[usize::from(byte & 0x0F)]));
            }
        }
    }
    out
}

/// 转义 `Content-Disposition` 里的 name / filename。
///
/// Go 的 `escapeQuotes` 只处理 `\` 与 `"`；本 crate 额外丢弃控制字符（含 CR/LF），
/// 不给 header 注入留口子。
fn push_escaped(out: &mut Vec<u8>, value: &str) {
    for byte in value.bytes() {
        match byte {
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'"' => out.extend_from_slice(b"\\\""),
            0x00..=0x1F | 0x7F => {}
            byte => out.push(byte),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MultipartBody, encode_query_value, ensured_filename, generate_boundary};

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0DIHDR";

    #[test]
    fn writes_go_compatible_multipart_bytes() {
        let mut body = MultipartBody::new("fixed");
        body.field("nickname", "阿求");
        body.file("file", "a.png", b"PNG");
        let (content_type, bytes) = body.finish();

        assert_eq!(content_type, "multipart/form-data; boundary=fixed");
        let expected = concat!(
            "--fixed\r\n",
            "Content-Disposition: form-data; name=\"nickname\"\r\n\r\n",
            "阿求\r\n",
            "--fixed\r\n",
            "Content-Disposition: form-data; name=\"file\"; filename=\"a.png\"\r\n",
            "Content-Type: application/octet-stream\r\n\r\n",
            "PNG\r\n",
            "--fixed--\r\n",
        );
        assert_eq!(
            String::from_utf8(bytes).expect("body 是 UTF-8"),
            expected,
            "与 Go mime/multipart 逐字节一致"
        );
    }

    #[test]
    fn escapes_quotes_backslashes_and_strips_control_characters() {
        let mut body = MultipartBody::new("b");
        body.file("file", "a\"b\\c\r\nd.png", b"x");
        let (_, bytes) = body.finish();
        let text = String::from_utf8(bytes).expect("header 部分是 UTF-8");
        assert!(text.contains(r#"filename="a\"b\\cd.png""#), "{text}");
        assert!(!text.contains("\r\nd.png"), "CR/LF 不能进 header: {text}");
    }

    #[test]
    fn fixes_extension_from_sniffed_format() {
        assert_eq!(ensured_filename("a.png", PNG), "a.png");
        assert_eq!(
            ensured_filename("a.PNG", PNG),
            "a.PNG",
            "大小写不影响已正确的后缀"
        );
        assert_eq!(ensured_filename("a.jpg", PNG), "a.png");
        assert_eq!(ensured_filename("a.tar.gz", PNG), "a.tar.png");
        assert_eq!(ensured_filename("noext", PNG), "noext.png");
        assert_eq!(ensured_filename("noext", b"not an image"), "noext.jpg");
        assert_eq!(ensured_filename("C:\\tmp\\a.webp", PNG), "a.png");
        assert_eq!(ensured_filename("/var/tmp/photo", PNG), "photo.png");
        // 结尾是分隔符 → 没有文件名部分，走兜底
        assert_eq!(ensured_filename("/var/tmp/", PNG), "image.png");
        assert_eq!(ensured_filename("", PNG), "image.png");
        assert_eq!(ensured_filename("..", PNG), "image.png");
        assert_eq!(ensured_filename("图中.png", PNG), "图中.png");
    }

    #[test]
    fn percent_encodes_query_values() {
        assert_eq!(encode_query_value("chatglm"), "chatglm");
        assert_eq!(encode_query_value("a.b_c-d~e"), "a.b_c-d~e");
        assert_eq!(encode_query_value("image/png"), "image%2Fpng");
        assert_eq!(encode_query_value("图 1.png"), "%E5%9B%BE%201.png");
        assert_eq!(encode_query_value("a&b=c"), "a%26b%3Dc");
        assert_eq!(encode_query_value(""), "");
    }

    #[test]
    fn boundaries_are_multipart_safe_and_unique() {
        let first = generate_boundary();
        let second = generate_boundary();
        assert_ne!(first, second);
        for boundary in [&first, &second] {
            assert!(
                boundary
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'),
                "{boundary}"
            );
            assert!(boundary.starts_with("gensokyo-"));
        }
    }
}

//! 出站 URL 转换管线（Go `handlers/message_parser.go:1749-1793` 的 `transformMessageTextUrl`）。
//!
//! - [`transform`]：门控仅 `transfer_url` 一项（lead 裁决 2026-11-13：Go handler 不消费
//!   `auto_escape`，Rust 不设该门控），逐 URL 优先级对齐 Go：IP 隐藏 → QR 收集 → 短链替换；
//!   原位替换、不 trim 周围空白。
//! - [`UrlMint`]：短链铸造两分派——本地 [`LocalMint`]（Go `GenerateShortURL` 的
//!   `lotus=false` 分支 + `GetBaseURL` 拼接）与远程 [`LotusMint`]（`lotus=true` 分支，
//!   token 口径同 Go `url/shorturl.go:151-157` 的 `hex(HMAC-SHA256(lotus_password, 长链))`，
//!   与服务端 `crate::http_api` 的 `ring::hmac` 实现同 key 同消息同编码）。
//! - 失败口径逐字对齐 Go：lotus 分支 mint 失败替换为空串（五个失败分支逐分支 `warn`，
//!   Go `shorturl.go` 每分支同样有日志）；本地分支 mint 失败仍拼
//!   `https://{server_dir}/url/` 悬空链接（Go 怪癖照抄，见 [`local_link`]，修复另立任务）。
//!   `mint` 传 `None`（未装配）时**保留原文不改写**，整次 [`transform`] 调用最多一条
//!   `warn`——这是 Rust-only 装配防御路径（Go 的 idmap/db 恒存在，无对应态），不是 Go
//!   行为：删空等于静默丢用户内容，比 Go 的悬空链接更糟。
//! - mint 的拼接参数（`server_dir` / `port` / `force_ssl` / `lotus_password`）逐调用现读
//!   传入的 `&Settings`，对齐 Go `GenerateShortURL` 每次 `config.GetXxx()`；两个 mint
//!   结构体本身不固化这些值。QR 图片生成与侧发送不在本模块（P56-T4 的 deferred hook
//!   消费 [`UrlTransform::qr_urls`]）。

use std::collections::HashMap;
use std::net::IpAddr;

use gensokyo_config::Settings;
use gensokyo_storage::{ShortUrlStore, StorageError, hex_lower, short_url_link};

/// Go 客户端 `http.Client{Timeout: 30 * time.Second}`（`url/shorturl.go:170`）。
const MINT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// 一次文本变换的结果（设计 §3.3）。
#[derive(Debug)]
pub struct UrlTransform {
    /// 替换后的文本。
    pub text: String,
    /// `url_to_qrimage` 开启时被删掉的 URL（按扫描序；PNG 生成与发送由 T4 hook 做）。
    pub qr_urls: Vec<String>,
}

/// 短链铸造分派。变体由装配侧按 `Settings::get_lotus()` 选定（T4 的 boot 装配），
/// [`transform`] 不感知 lotus 开关、只调 [`UrlMint::mint`]——Go 两个分支最终都落到
/// `GenerateShortURL` 一个入口，差别只在函数内部走哪条分支。
#[derive(Debug)]
pub enum UrlMint {
    /// 本地主节点：短码落 [`ShortUrlStore`]，替换为完整短链。
    Local(LocalMint),
    /// lotus 从节点：POST 另一台 Gensokyo 的 `/url`，替换为远端返回的完整 shortURL。
    Lotus(LotusMint),
}

impl UrlMint {
    /// 铸造一条短链。失败口径分分支：lotus 失败返回空串（Go 直接 `return shortURL`）；
    /// 本地失败返回悬空 base（Go 调用方无条件拼 base，见 [`local_link`]）。
    ///
    /// 拼接用的 `server_dir` / `port` / `force_ssl` / `lotus_password` 一律现读 `cfg`
    /// （Go `GenerateShortURL` 每次 `config.GetXxx()` 同口径）：`port` 与 `force_ssl`
    /// 不在重启表（`gensokyo-config/src/diff.rs`）内，热改后必须立刻生效，固化进 mint
    /// 会让每次请求都打旧端口/旧协议而必然失败。
    pub async fn mint(&self, long_url: &str, cfg: &Settings) -> String {
        match self {
            Self::Local(mint) => mint.mint(long_url, cfg),
            Self::Lotus(mint) => mint.mint(long_url, cfg).await,
        }
    }
}

/// 本地短链 mint（Go `GenerateShortURL` 的 `lotus=false` 分支 + `GetBaseURL()+"/url/"`，
/// `message_parser.go:1784-1789`）。只持短链库句柄，地址逐调用现读 `cfg`。
#[derive(Debug)]
pub struct LocalMint {
    /// 短链库（与 HTTP 服务端路由 `crate::http_api` 共用同一实例）。
    pub store: std::sync::Arc<ShortUrlStore>,
}

impl LocalMint {
    /// `create` 成功 → `https://{server_dir}/url/{code}`；Err → `https://{server_dir}/url/`
    /// （Go 缺陷照抄，见 [`local_link`]）。`server_dir` 取 `cfg` 当前快照——Go `GetBaseURL`
    /// 只看它，不看端口/SSL。
    pub fn mint(&self, long_url: &str, cfg: &Settings) -> String {
        local_link(self.store.create(long_url), cfg.get_server_dir())
    }
}

/// `create` 结果 → 替换文本的纯映射（Err 口径单独可测）。
///
/// Err 分支**逐字照抄 Go 缺陷**（`message_parser.go:1784-1789`）：`GenerateShortURL` 失败
/// 返回 `""` 后，调用方仍无条件拼 `GetBaseURL()+"/url/"+code`，替换文本是悬空链接
/// `https://{server_dir}/url/`。与 HTTP 无 token 缺陷同口径——**修复另立任务**，这里
/// 不得「顺手改好」成空串（parity 基准是 Go 的实际行为，不是它的应然行为）。
fn local_link(result: Result<String, StorageError>, server_dir: &str) -> String {
    let code = result.unwrap_or_default();
    short_url_link(server_dir, &code)
}

/// lotus 远程 mint（Go `GenerateShortURL` 的 `lotus=true` 分支，`url/shorturl.go:142-198`）。
///
/// 只持共享 HTTP 客户端：目标地址（`server_dir` / `port` / `force_ssl`）与 `lotus_password`
/// 逐调用现读 `cfg`。`port` / `force_ssl` 不在重启表（`gensokyo-config/src/diff.rs`）内，
/// boot 期固化的话热改后每次 mint 都打旧端口/旧协议，必然每次失败。
#[derive(Debug)]
pub struct LotusMint {
    /// 共享 HTTP 客户端（[`LotusMint::try_new`] 构造，带 [`MINT_TIMEOUT`] 超时）。
    client: reqwest::Client,
}

impl LotusMint {
    /// 按 Go 口径构造客户端（含 30s 超时）。reqwest 只用无 feature 门槛的核心 API：
    /// body 手写 urlencoded（[`form_encode`]）、响应 [`reqwest::Response::text`] +
    /// serde_json，不开 `form`/`json` feature（workspace 声明 default-features=false）。
    ///
    /// 构造前先预检进程级 `CryptoProvider`（同 `gensokyo-storage` 的
    /// `idmap/http_remote.rs` spawn 预检口径）：reqwest 0.13.5 无 provider 时
    /// `Client::builder().build()` 直接 **panic**（`async_impl/client.rs:2501`）而非
    /// 返回 `Err`，workspace 又是 `panic = "abort"`（根 `Cargo.toml`）——库嵌入方
    /// （`main.rs` 文档路径）配 `lotus=true` 会在 boot 期终止进程。provider 未安装时
    /// 返回 `Err`；builder 失败同样返回 `Err`，交调用方（[`crate::boot`]）记日志并
    /// **不注入 mint**：短链分支保留 URL 原文（M-C 语义），不崩进程。这里
    /// 不得回退 `reqwest::Client::new()`——它同样会 panic。本仓约定 boot 不
    /// panic：`media.rs` 不在 boot 建 client、`startup.rs` 的 shared_transport 只降级。
    /// 生产二进制由 gensokyo-bin startup 进程级安装 ring，此预检在正常路径不触发。
    pub fn try_new() -> Result<Self, String> {
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            return Err(
                "未安装默认 crypto provider，lotus HTTP 客户端不构建：需宿主先安装进程级 rustls CryptoProvider（本仓二进制由 gensokyo-bin startup 安装）"
                    .to_string(),
            );
        }
        reqwest::Client::builder()
            .timeout(MINT_TIMEOUT)
            .build()
            .map(|client| Self { client })
            .map_err(|err| err.to_string())
    }

    /// `POST {proto}://{server_dir}:{port}/url`，form `token` + `url`（密码非空时）。
    /// 网络错误 / 非 200 / 响应体读取失败 / JSON 解码失败 / 缺 `shortURL` → 空串，且
    /// 逐分支 `warn`（Go `shorturl.go:162/173/180-181/188/194` 每个失败分支都有日志；
    /// 只带状态码与请求 URL，**绝不**含 token / 密码）。成功返回远端给的**完整
    /// shortURL**（Go `:192-198` 直接 return，不拼本地前缀）。
    pub async fn mint(&self, long_url: &str, cfg: &Settings) -> String {
        let request_url = lotus_request_url(
            cfg.get_port_value(),
            cfg.get_force_ssl(),
            cfg.get_server_dir(),
        );
        let password = cfg.get_lotus_password();
        // Go 用 `url.Values.Encode()`：键按字母序输出，密码非空时是 `token=…&url=…`。
        let form = if password.is_empty() {
            format!("url={}", form_encode(long_url))
        } else {
            format!(
                "token={}&url={}",
                lotus_token(password, long_url),
                form_encode(long_url)
            )
        };
        let response = match self
            .client
            .post(request_url.as_str())
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(form)
            .send()
            .await
        {
            Ok(response) => response,
            Err(err) => {
                // Go 在建请求（`:160-164`）与发请求（`:171-175`）两处分别记日志；reqwest
                // 到 send 才校验 URL，两个失败点在此合并为一个分支。
                tracing::warn!(
                    error = %err,
                    request_url = %request_url,
                    "lotus 短链请求发送失败"
                );
                return String::new();
            }
        };
        let status = response.status();
        if status != reqwest::StatusCode::OK {
            tracing::warn!(
                status = %status,
                request_url = %request_url,
                "lotus 短链返回非 200（400 请检查 lotus_password）"
            );
            return String::new();
        }
        // Go 直接从 body 流解码（`:186`），读取与解码两类失败在 Rust 分两步、各有日志。
        let body = match response.text().await {
            Ok(body) => body,
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    request_url = %request_url,
                    "lotus 短链响应体读取失败"
                );
                return String::new();
            }
        };
        // Go 解进 map[string]string：任何字段值不是字符串都算解码失败。
        let parsed = match serde_json::from_str::<HashMap<String, String>>(&body) {
            Ok(parsed) => parsed,
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    request_url = %request_url,
                    "lotus 短链响应 JSON 解码失败"
                );
                return String::new();
            }
        };
        match parsed.get("shortURL") {
            Some(short_url) => short_url.clone(),
            None => {
                tracing::warn!(
                    request_url = %request_url,
                    "lotus 短链响应缺少 shortURL 字段"
                );
                String::new()
            }
        }
    }
}

/// [`LotusMint::mint`] 请求 URL 的纯函数形态（协议判定单独可测，不必真发 TLS），
/// 参数由 mint 逐调用从 `cfg` 取。
fn lotus_request_url(port_value: &str, force_ssl: bool, server_dir: &str) -> String {
    let proto = if port_value == "443" || force_ssl {
        "https"
    } else {
        "http"
    };
    format!("{proto}://{server_dir}:{port_value}/url")
}

/// token 计算（Go `shorturl.go:152-154`）：`hex(HMAC-SHA256(key=password, msg=长链))`，
/// 与服务端 `crate::http_api::is_valid_token` 同用 `ring::hmac::HMAC_SHA256` +
/// [`gensokyo_storage::hex_lower`]（服务端的自指比较口径是 Go 缺陷的忠实移植，与本客户端
/// 无关——Go 两端从来就不同串，密码非空时服务端恒 400、mint 恒空串，本包按 Go 行为对齐）。
fn lotus_token(password: &str, long_url: &str) -> String {
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, password.as_bytes());
    hex_lower(ring::hmac::sign(&key, long_url.as_bytes()).as_ref())
}

/// `application/x-www-form-urlencoded` 值编码（Go `url.QueryEscape` 口径：字母数字与
/// `-_.~` 原样、空格 `+`、其余 `%XX` 大写十六进制；服务端按标准 urlencoded 解码）。
fn form_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(*byte));
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// 出站 URL 变换（Go `transformMessageTextUrl`，`message_parser.go:1749-1793`）。
///
/// 逐 span 按扫描序原位替换，每个 URL 的优先级互斥短路，与 Go 一致：
/// 1. `server_dir` 是 IP 且 `visible_ip=false` → 替换空串（优先于 QR）；
/// 2. `url_to_qrimage=true` → URL 收进 [`UrlTransform::qr_urls`]、文本替换空串
///    （Go 的 QR 生成失败也删 URL，生成与侧发送在 T4）；
/// 3. 否则 → [`UrlMint::mint`] 的返回值原位替换（lotus 失败空串、本地失败悬空 base，
///    见 [`local_link`]）；`mint` 为 `None`（未装配，Rust-only 防御路径，非 Go 行为）
///    → **保留原文不改写**，整次调用结束后最多一条 `warn`（无 URL 命中该分支时不打）。
///
/// 门控只有 `transfer_url` 一项（lead 裁决：Go handler 不消费 auto_escape）。span 是
/// 字节偏移、不重叠，按游标重建原文片段——替换文本不参与扫描，与 Go
/// `ReplaceAllStringFunc` 逐匹配替换等价；周围空白逐字节保留、不 trim。
pub async fn transform(text: &str, cfg: &Settings, mint: Option<&UrlMint>) -> UrlTransform {
    if !cfg.get_transfer_url() {
        return UrlTransform {
            text: text.to_owned(),
            qr_urls: Vec::new(),
        };
    }
    // Go `net.ParseIP` 口径：带端口的字符串不是 IP（Rust `IpAddr` 解析同口径）。
    let is_ip = cfg.get_server_dir().parse::<IpAddr>().is_ok();
    let hide_ip = is_ip && !cfg.get_visible_ip();
    let to_qr = cfg.get_url_to_qrimage();

    let mut out = String::with_capacity(text.len());
    let mut qr_urls = Vec::new();
    let mut cursor = 0usize;
    let mut mint_missing = 0usize;
    for span in gensokyo_protocol::xurls::find_urls(text) {
        out.push_str(&text[cursor..span.start]);
        let url = &text[span.start..span.end];
        let replacement = if hide_ip {
            String::new()
        } else if to_qr {
            qr_urls.push(url.to_owned());
            String::new()
        } else {
            match mint {
                Some(mint) => mint.mint(url, cfg).await,
                // Rust-only 防御路径：Go 的 idmap/db 恒存在，没有「未装配」态。未装配时
                // 原文照抄保留（删空等于静默丢用户内容），计数、循环外统一告警一次，
                // 不影响 QR / IP 分支。
                None => {
                    mint_missing += 1;
                    url.to_owned()
                }
            }
        };
        out.push_str(&replacement);
        cursor = span.end;
    }
    out.push_str(&text[cursor..]);
    if mint_missing > 0 {
        tracing::warn!(count = mint_missing, "短链 mint 未装配，URL 原文保留不改写");
    }
    UrlTransform { text: out, qr_urls }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
    use std::time::Duration;

    use gensokyo_storage::short_url::hashed_code;

    /// 生产入口用的 URL（xurls T1 金样已验证能被 `find_urls` 扫到）。
    const URL1: &str = "http://a.com";
    const URL2: &str = "https://b.org/p";

    fn base_settings() -> Settings {
        // 测试构造沿用 webhook.rs 的 pub 字段直改口径（结构体更新语法过 clippy）。
        Settings {
            transfer_url: true,
            server_dir: "example.com".to_owned(),
            ..Default::default()
        }
    }

    fn temp_store(dir: &tempfile::TempDir) -> Arc<ShortUrlStore> {
        Arc::new(ShortUrlStore::open(dir.path().join("url-pipeline.redb")).expect("打开短链库"))
    }

    /// lotus 分支用例配置：mint 逐调用现读 `cfg`（对齐 Go `GenerateShortURL` 每次
    /// `config.GetXxx()`），所以 stub 的地址、端口与密码都写在配置里而不是 mint 上。
    /// `server_dir` 取 127.0.0.1，配 `visible_ip: true` 免触发 IP 隐藏门控。
    fn lotus_settings(port: &str, password: &str) -> Settings {
        Settings {
            transfer_url: true,
            server_dir: "127.0.0.1".to_owned(),
            visible_ip: true,
            port: port.to_owned(),
            lotus_password: password.to_owned(),
            ..Default::default()
        }
    }

    // ---------- transform 门控与优先级 ----------

    #[tokio::test]
    async fn gate_off_returns_input_unchanged() {
        // 最凶配置（IP + QR 开）也必须整体不生效。
        let mut settings = base_settings();
        settings.transfer_url = false;
        settings.server_dir = "1.2.3.4".to_owned();
        settings.url_to_qrimage = true;
        let dir = tempfile::tempdir().expect("临时目录");
        let store = temp_store(&dir);
        let mint = UrlMint::Local(LocalMint { store });
        let text = "看 http://a.com 好";
        let result = transform(text, &settings, Some(&mint)).await;
        assert_eq!(result.text, text);
        assert!(result.qr_urls.is_empty());
    }

    #[tokio::test]
    async fn ip_hidden_wins_over_qr() {
        let mut settings = base_settings();
        settings.server_dir = "1.2.3.4".to_owned();
        settings.visible_ip = false;
        settings.url_to_qrimage = true;
        let dir = tempfile::tempdir().expect("临时目录");
        let store = temp_store(&dir);
        let mint = UrlMint::Local(LocalMint {
            store: Arc::clone(&store),
        });
        let result = transform("见 http://a.com 吧", &settings, Some(&mint)).await;
        assert_eq!(result.text, "见  吧", "原位删除、双空格保留");
        assert!(result.qr_urls.is_empty(), "IP 隐藏优先于 QR，不收集");
        assert_eq!(
            store.resolve(&hashed_code(URL1)).expect("查询"),
            None,
            "IP 隐藏路径不得调用 mint"
        );
    }

    #[tokio::test]
    async fn ip_visible_falls_through_to_qr() {
        let mut settings = base_settings();
        settings.server_dir = "1.2.3.4".to_owned();
        settings.visible_ip = true;
        settings.url_to_qrimage = true;
        let dir = tempfile::tempdir().expect("临时目录");
        let store = temp_store(&dir);
        let mint = UrlMint::Local(LocalMint {
            store: Arc::clone(&store),
        });
        let result = transform("看 http://a.com 好", &settings, Some(&mint)).await;
        assert_eq!(result.text, "看  好");
        assert_eq!(result.qr_urls, [URL1.to_owned()]);
        assert_eq!(
            store.resolve(&hashed_code(URL1)).expect("查询"),
            None,
            "QR 路径不调用 mint"
        );
    }

    #[tokio::test]
    async fn qr_collects_multiple_urls_in_scan_order() {
        let mut settings = base_settings();
        settings.url_to_qrimage = true;
        let dir = tempfile::tempdir().expect("临时目录");
        let mint = UrlMint::Local(LocalMint {
            store: temp_store(&dir),
        });
        let result = transform(
            "先 http://a.com 中 https://b.org/p 后",
            &settings,
            Some(&mint),
        )
        .await;
        assert_eq!(result.text, "先  中  后");
        assert_eq!(result.qr_urls, [URL1.to_owned(), URL2.to_owned()]);
    }

    #[tokio::test]
    async fn inplace_replacement_keeps_surrounding_text() {
        // Go 的 ReplaceAllStringFunc 语义：URL 删空，周围一个字符都不多动。
        // 含全角括号紧邻（xurls 金样口径：（https://a.com/x）只取括号内的 URL）与多字节
        // 中文混排，逐 span 替换后偏移不错位。
        let mut settings = base_settings();
        settings.url_to_qrimage = true;
        let dir = tempfile::tempdir().expect("临时目录");
        let mint = UrlMint::Local(LocalMint {
            store: temp_store(&dir),
        });
        let text = "（https://a.com/x）中 http://a.com 尾 https://b.org/p";
        let result = transform(text, &settings, Some(&mint)).await;
        assert_eq!(result.text, "（）中  尾 ");
        assert_eq!(
            result.qr_urls,
            [
                "https://a.com/x".to_owned(),
                URL1.to_owned(),
                URL2.to_owned()
            ]
        );
    }

    #[tokio::test]
    async fn local_mint_replaces_with_full_link() {
        let settings = base_settings();
        let dir = tempfile::tempdir().expect("临时目录");
        let store = temp_store(&dir);
        let mint = UrlMint::Local(LocalMint {
            store: Arc::clone(&store),
        });
        let result = transform(
            "看 http://a.com 好 https://b.org/p 呢",
            &settings,
            Some(&mint),
        )
        .await;
        assert_eq!(
            result.text,
            format!(
                "看 https://example.com/url/{} 好 https://example.com/url/{} 呢",
                hashed_code(URL1),
                hashed_code(URL2)
            )
        );
        assert!(result.qr_urls.is_empty());
        assert_eq!(
            store.resolve(&hashed_code(URL1)).expect("查询").as_deref(),
            Some(URL1),
            "短码必须真实落库（Go storeURL 同义）"
        );
    }

    #[tokio::test]
    async fn lotus_variant_replaces_via_transform() {
        ensure_crypto_provider();
        // 端到端验证 UrlMint 枚举分派（LotusMint 细节见下方 mint 用例）。
        let stub = Stub::spawn(|| (200, r#"{"shortURL":"https://s.cn/x9"}"#.to_owned()));
        let settings = lotus_settings(&stub.port(), "");
        let mint = UrlMint::Lotus(LotusMint::try_new().expect("构造 lotus client"));
        let result = transform("看 http://a.com 好", &settings, Some(&mint)).await;
        assert_eq!(result.text, "看 https://s.cn/x9 好");
    }

    // ---------- LocalMint 映射 ----------

    #[test]
    fn local_link_error_keeps_go_dangling_base() {
        // 真库（tempfile + redb）的 create 没有对外可触发的错误路径，错误口径在纯映射
        // 函数上断言。**Go 缺陷照抄（同 HTTP 无 token 口径，修复另立任务）**：
        // GenerateShortURL 失败返回 "" 后调用方仍无条件拼 base+"/url/"+code →
        // 替换文本是悬空链接而不是空串（message_parser.go:1784-1789）。
        let err = StorageError::RetriesExhausted("测试注入".to_owned());
        assert_eq!(
            local_link(Err(err), "example.com"),
            "https://example.com/url/"
        );
        assert_eq!(
            local_link(Ok("abc123".to_owned()), "example.com"),
            short_url_link("example.com", "abc123"),
            "成功走 https 前缀 + /url/ + 短码"
        );
    }

    #[tokio::test]
    async fn mint_absent_keeps_original_text() {
        // Rust-only 装配防御（Go 无此态）：mint 为 None 时短链分支**保留原文不改写**
        // （删空等于静默丢用户内容），整次调用最多一条 warn；QR / IP 分支不受影响
        // （下方与 qr 收集用例已覆盖 None 之外的分支）。
        let settings = base_settings();
        let result = transform("看 http://a.com 好", &settings, None).await;
        assert_eq!(result.text, "看 http://a.com 好");
        assert!(result.qr_urls.is_empty());
    }

    #[tokio::test]
    async fn mint_absent_keeps_ip_hide_and_qr_collect() {
        // None 只影响短链分支：QR 收集路径照常删空并进 qr_urls。
        let mut settings = base_settings();
        settings.url_to_qrimage = true;
        let result = transform("看 http://a.com 好", &settings, None).await;
        assert_eq!(result.text, "看  好");
        assert_eq!(result.qr_urls, [URL1.to_owned()]);
    }

    // ---------- LotusMint ----------

    /// `rustls-no-provider` 口径（media/runtime dev-dep 注释同款）：构建 reqwest Client
    /// 纯 HTTP 也需要进程级 provider，测试里自行安装（生产由二进制 startup 装）。
    fn ensure_crypto_provider() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }

    #[test]
    fn request_url_protocol_matches_go() {
        assert_eq!(
            lotus_request_url("8080", false, "s.example"),
            "http://s.example:8080/url"
        );
        assert_eq!(
            lotus_request_url("443", false, "s.example"),
            "https://s.example:443/url",
            "端口 443 即 https（Go shorturl.go:138）"
        );
        assert_eq!(
            lotus_request_url("80", true, "s.example"),
            "https://s.example:80/url",
            "force_ssl 即 https，优先级与 Go 的 || 一致"
        );
    }

    #[test]
    fn form_encode_matches_go_query_escape() {
        // 期望值按 Go url.QueryEscape 手抄，不经本函数生成。
        assert_eq!(
            form_encode("https://example.com/a?b=c&d=e#f 你"),
            "https%3A%2F%2Fexample.com%2Fa%3Fb%3Dc%26d%3De%23f+%E4%BD%A0"
        );
        // Go 的 QueryEscape 只保留字母数字与 `-_.~`，`*` 也要转义。
        assert_eq!(form_encode("a-_.~*+/%"), "a-_.~%2A%2B%2F%25");
    }

    #[test]
    fn lotus_token_is_hmac_sha256_of_long_url() {
        // 独立于实现断言：ring 签名 + 手写 hex，期望值固定 key=密码、msg=长链
        // （Go shorturl.go:152-154 与服务端 http_api.rs:159-160 同算法同编码）。
        let password = "p@ss";
        let long_url = "https://example.com/a";
        let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, password.as_bytes());
        let expected: String = ring::hmac::sign(&key, long_url.as_bytes())
            .as_ref()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(lotus_token(password, long_url), expected);
        assert_eq!(expected.len(), 64);
    }

    #[tokio::test]
    async fn lotus_mint_posts_form_and_returns_remote_link() {
        ensure_crypto_provider();
        let stub = Stub::spawn(|| (200, r#"{"shortURL":"https://s.cn/Ab12Cd"}"#.to_owned()));
        let settings = lotus_settings(&stub.port(), "p@ss");
        let mint = LotusMint::try_new().expect("构造 lotus client");
        assert_eq!(
            mint.mint(URL1, &settings).await,
            "https://s.cn/Ab12Cd",
            "远端完整 shortURL 原样返回"
        );

        let requests = stub.requests();
        assert_eq!(requests.len(), 1);
        let raw = &requests[0];
        let first_line = raw.lines().next().expect("请求行");
        assert_eq!(
            first_line, "POST /url HTTP/1.1",
            "method/path 对齐 Go 客户端"
        );
        assert!(
            raw.to_ascii_lowercase()
                .contains("content-type: application/x-www-form-urlencoded"),
            "表单 content-type：{raw}"
        );
        let body = raw.split_once("\r\n\r\n").expect("头尾分隔").1;
        let expected_token: String = {
            let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, b"p@ss");
            ring::hmac::sign(&key, URL1.as_bytes())
                .as_ref()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect()
        };
        // Go `url.Values.Encode()` 按 key 字母序输出：`token` 在 `url` 前。
        assert_eq!(
            body,
            format!("token={expected_token}&url=http%3A%2F%2Fa.com")
        );
        assert_eq!(expected_token.len(), 64, "token 为 64 位小写 hex");
    }

    #[tokio::test]
    async fn lotus_mint_omits_token_when_password_empty() {
        ensure_crypto_provider();
        let stub = Stub::spawn(|| (200, r#"{"shortURL":"https://s.cn/1"}"#.to_owned()));
        let settings = lotus_settings(&stub.port(), "");
        let mint = LotusMint::try_new().expect("构造 lotus client");
        assert_eq!(mint.mint(URL1, &settings).await, "https://s.cn/1");
        let body = stub
            .requests()
            .remove(0)
            .split_once("\r\n\r\n")
            .expect("头尾分隔")
            .1
            .to_owned();
        assert_eq!(
            body, "url=http%3A%2F%2Fa.com",
            "空密码不发 token（Go :151 条件）"
        );
    }

    #[tokio::test]
    async fn lotus_mint_failure_paths_return_empty() {
        ensure_crypto_provider();
        // 非 200 / 缺 shortURL / 坏 JSON / 值不是字符串（Go 解 map[string]string 会失败）。
        let cases: [(u16, &str); 4] = [
            (500, r#"{"shortURL":"https://s.cn/x"}"#),
            (200, r#"{"other":"1"}"#),
            (200, "not json"),
            (200, r#"{"shortURL":5}"#),
        ];
        for (status, body) in cases {
            let stub = Stub::spawn(move || (status, body.to_owned()));
            let settings = lotus_settings(&stub.port(), "");
            let mint = LotusMint::try_new().expect("构造 lotus client");
            assert_eq!(
                mint.mint(URL1, &settings).await,
                "",
                "失败口径：状态 {status} 响应 {body}"
            );
        }
    }

    #[tokio::test]
    async fn lotus_mint_connection_error_returns_empty() {
        ensure_crypto_provider();
        // 拿端口后立刻释放监听——连接必然被拒（Go 的 client.Do 错误分支 → ""）。
        let port = bind_test_listener()
            .local_addr()
            .expect("取地址")
            .port()
            .to_string();
        let settings = lotus_settings(&port, "");
        let mint = LotusMint::try_new().expect("构造 lotus client");
        assert_eq!(mint.mint(URL1, &settings).await, "");
    }

    // ---------- 手写 HTTP stub（沿用 idmap/http_remote.rs 测试的口径） ----------

    /// 测试专用端口分配器：http_remote 同款的专用段（20000–32767），从 22000 起避开
    /// idmap stub 的占位段；bind 失败递增重试。
    fn bind_test_listener() -> TcpListener {
        static NEXT: AtomicU16 = AtomicU16::new(22000);
        loop {
            let port = NEXT.fetch_add(1, Ordering::Relaxed);
            assert!(port < 32768, "测试端口段耗尽");
            if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) {
                return listener;
            }
        }
    }

    struct Stub {
        addr: SocketAddr,
        requests: Arc<std::sync::Mutex<Vec<String>>>,
        stop: Arc<AtomicBool>,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl Stub {
        /// responder 固定应答 (status, body)；记录每个请求的完整原文（头+体）。
        fn spawn(responder: impl Fn() -> (u16, String) + Send + 'static) -> Self {
            let listener = bind_test_listener();
            let addr = listener.local_addr().unwrap();
            listener.set_nonblocking(true).unwrap();
            let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
            let stop = Arc::new(AtomicBool::new(false));
            let shared = Arc::clone(&requests);
            let stopped = Arc::clone(&stop);
            let handle = std::thread::spawn(move || {
                while !stopped.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => handle_conn(stream, &shared, &responder),
                        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            });
            Self {
                addr,
                requests,
                stop,
                handle: Some(handle),
            }
        }

        fn port(&self) -> String {
            self.addr.port().to_string()
        }

        fn requests(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }
    }

    impl Drop for Stub {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }

    fn handle_conn(
        mut stream: TcpStream,
        requests: &std::sync::Mutex<Vec<String>>,
        responder: &impl Fn() -> (u16, String),
    ) {
        stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
        // 一应答一连接（应答带 Connection: close 后断连；比 http_remote 的 keep-alive
        // 循环简单，且不会让 accept 线程挂在下一请求的 read 上）。
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        let head_end = loop {
            match stream.read(&mut chunk) {
                Ok(0) => return,
                Ok(read) => {
                    buf.extend_from_slice(&chunk[..read]);
                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        break pos;
                    }
                }
                Err(_) => return,
            }
        };
        // 按 Content-Length 读满请求体。
        let head = String::from_utf8_lossy(&buf[..head_end]).to_ascii_lowercase();
        let need = head_end
            + 4
            + head
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|value| value.trim().parse::<usize>().ok())
                .unwrap_or(0);
        while buf.len() < need {
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => buf.extend_from_slice(&chunk[..read]),
                Err(_) => return,
            }
        }
        requests
            .lock()
            .unwrap()
            .push(String::from_utf8_lossy(&buf).into_owned());
        let (status, body) = responder();
        let response = format!(
            "HTTP/1.1 {status} Status\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        if stream.write_all(response.as_bytes()).is_err() {
            return;
        }
        let _ = stream.flush();
    }
}

//! 短链库与纯函数（Go `url/shorturl.go`）。
//!
//! Go 把 `短码 → 长链` 存进 bbolt `gensokyo.db` 的 bucket `shortURLs`（`shorturl.go:24-76`），
//! 本 crate 用 redb 文件 `short-urls.redb` 的 `short_urls` 表：key/value 都是 UTF-8 原样，
//! 短码格式不变。
//!
//! # 生成规则（与 Go 一致）
//!
//! 1. 首选 `sha256(长链)` 前 3 字节的小写十六进制 = 6 字符（`generateHashedString`）；
//! 2. 该短码已被占用（同一个长链第二次生成就会命中）时改为 6 位随机码，直到不冲突
//!    （`generateRandomString`，字符集 62 个字母数字）；
//! 3. 写入 `短码 → 长链` 并返回短码。
//!
//! 注意 Go 不做"同一长链复用同一短码"：第二次生成得到的是随机码，两条记录并存。
//! 这里照抄（迁移期同 URL 会产生多条映射，与 Go 库内容一致）。
//!
//! # 只实现"库 + 纯函数"，HTTP 入口留给 runtime
//!
//! Go 的两个 gin handler 挂在主服务器（`main.go:566-567`：`POST /url`、
//! `GET /url/:shortURL`）。Rust 的等价入口是 runtime 的 `http_api` / 主 listener——那是
//! "已定稿"模块，本波次不动。接线时的契约：
//!
//! - 创建：`is_malicious` / `validate_url` 不过 → 400；否则 [`ShortUrlStore::create`]，
//!   响应体 `{"shortURL": short_url_link(server_dir, code)}`；
//! - 跳转：[`ShortUrlStore::resolve`] 查不到 → 404；查到 → `301` + [`redirect_target`]。
//!
//! # 与 Go 的差异（明确列出）
//!
//! - **文件**：`gensokyo.db`（bbolt）→ `short-urls.redb`；表名 / 键值编码保持一致。
//! - **Lotus 远程模式**：`lotus: true` 时 Go 把创建/查询转发给另一台 gensokyo（HTTP POST +
//!   表单 + HMAC token）。本层不做网络调用，本地库路径始终生效；远程模式需要在 HTTP 层另接。
//! - **token 校验**：Go `isValidToken`（`shorturl.go:350-364`）对 **token 本身**做
//!   HMAC-SHA256 再与 token 比较（自指），密码非空时永远失败——属 Go 侧缺陷。接线时按注释
//!   意图（对长链做 HMAC）实现，并在该处注明与 Go 的行为差异。
//! - **Base64 入参归一**：Go `decodeBase64IfNeeded` 在 HTTP handler 里做（`shorturl.go:331`），
//!   属入口层职责，本层不收；接线时补在 handler。
//! - **随机源**：Go 用 `crypto/rand`，失败回退时间戳；本实现用 SHA-256 扩散
//!   （`RandomState`（OS 种子）+ 纳秒时间 + 进程号 + 递增计数），冲突由重试兜底。
//!   短码不是安全凭证，不为它新引 `rand` / `getrandom`。
//! - **URL 校验**：比 Go 严格两处——host 为空（`http://`）拒绝；`localhost` 大小写不敏感。
//!   其余照抄 Go：只拦 `127.0.0.1` / `::1` / `localhost` 与 `10/8`、`172.16/12`、
//!   `192.168/16` 三段私有 IPv4，**不解析域名**（DNS rebinding 是 Go 既有缺口，
//!   这里不改语义，登记在案）；`127.0.0.2` 等其它回环地址同样放行（Go 原行为）。
//!   私网判定复用 [`gensokyo_core::net::is_rfc1918`]（同一条规则只有一处实现）；它有意
//!   不含回环 / 链路本地——更严的 SSRF 口径只在 `gensokyo-media::redirect` 使用。

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::net::IpAddr;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use gensokyo_core::net;
use redb::{Database, ReadableDatabase, TableDefinition};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::StorageError;
use crate::hex_lower;
use crate::idmap::rb;

/// 默认库文件名（Go 的 `gensokyo.db` 同时存 idmap 与短链，Rust 拆成独立文件）。
pub const DEFAULT_SHORT_URL_PATH: &str = "short-urls.redb";

/// 唯一的表，对应 Go bbolt bucket `shortURLs`。
const SHORT_URLS: TableDefinition<'static, &'static str, &'static str> =
    TableDefinition::new("short_urls");

/// Go `charset`：62 个字母数字。
const CHARSET: &[u8; 62] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

/// Go `length`：短码长度。
const CODE_LEN: usize = 6;

/// 随机码连续冲突的尝试上限。Go 是 `for {}` 死循环；62^6 ≈ 5.7e10 的空间下
/// 连续 64 次冲突只可能是随机源退化，返回错误比挂住进程好。
const MAX_RANDOM_ATTEMPTS: usize = 64;

/// 跳转状态码（Go `RedirectFromShortURLHandler` 用 `StatusMovedPermanently`）。
pub const REDIRECT_STATUS: u16 = 301;

/// 短链库。
#[derive(Debug)]
pub struct ShortUrlStore {
    db: Database,
}

impl ShortUrlStore {
    /// 打开（必要时创建）短链库并建表。
    ///
    /// # Errors
    ///
    /// 路径不可写、或 redb 打开 / 建表失败时返回 [`StorageError::Redb`]。
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let db = Database::create(path).map_err(rb)?;
        let store = Self { db };
        let txn = store.db.begin_write().map_err(rb)?;
        {
            txn.open_table(SHORT_URLS).map_err(rb)?;
        }
        txn.commit().map_err(rb)?;
        Ok(store)
    }

    /// 短码是否已被占用（Go `existsInDB`）。
    ///
    /// # Errors
    ///
    /// 读事务失败时返回 [`StorageError::Redb`]。
    pub fn exists(&self, code: &str) -> Result<bool, StorageError> {
        let txn = self.db.begin_read().map_err(rb)?;
        let table = txn.open_table(SHORT_URLS).map_err(rb)?;
        Ok(table.get(code).map_err(rb)?.is_some())
    }

    /// 写入一条映射（Go `storeURL`）。同码覆盖，与 bbolt `Put` 一致。
    ///
    /// # Errors
    ///
    /// 写事务失败时返回 [`StorageError::Redb`]。
    pub fn store(&self, code: &str, long_url: &str) -> Result<(), StorageError> {
        let txn = self.db.begin_write().map_err(rb)?;
        {
            let mut table = txn.open_table(SHORT_URLS).map_err(rb)?;
            table.insert(code, long_url).map_err(rb)?;
        }
        txn.commit().map_err(rb)?;
        Ok(())
    }

    /// 按短码取长链（Go `getLongURLFromDB` 的本地分支）。查不到返回 `None`。
    ///
    /// # Errors
    ///
    /// 读事务失败时返回 [`StorageError::Redb`]。
    pub fn resolve(&self, code: &str) -> Result<Option<String>, StorageError> {
        let txn = self.db.begin_read().map_err(rb)?;
        let table = txn.open_table(SHORT_URLS).map_err(rb)?;
        Ok(table
            .get(code)
            .map_err(rb)?
            .map(|value| value.value().to_owned()))
    }

    /// 生成并落库一条短链（Go `GenerateShortURL` 的本地分支），返回短码。
    ///
    /// # Errors
    ///
    /// 库事务失败，或随机码连续 [`MAX_RANDOM_ATTEMPTS`] 次冲突时返回 [`StorageError`]。
    pub fn create(&self, long_url: &str) -> Result<String, StorageError> {
        let mut code = hashed_code(long_url);
        if self.exists(&code)? {
            code = self.random_unused_code()?;
        }
        self.store(&code, long_url)?;
        Ok(code)
    }

    fn random_unused_code(&self) -> Result<String, StorageError> {
        for _ in 0..MAX_RANDOM_ATTEMPTS {
            let candidate = random_code();
            if !self.exists(&candidate)? {
                return Ok(candidate);
            }
        }
        Err(StorageError::RetriesExhausted(format!(
            "短码连续 {MAX_RANDOM_ATTEMPTS} 次冲突（62^{CODE_LEN} 空间），随机源疑似退化"
        )))
    }
}

/// `sha256(长链)` 前 3 字节的小写十六进制（Go `generateHashedString`）。
#[must_use]
pub fn hashed_code(long_url: &str) -> String {
    let digest = Sha256::digest(long_url.as_bytes());
    hex_lower(&digest[..3])
}

/// 6 位随机短码（Go `generateRandomString`）。
///
/// 拒绝采样消除取模偏置：只接受 `< 248`（62×4）的字节。
#[must_use]
pub fn random_code() -> String {
    let mut code = String::with_capacity(CODE_LEN);
    let mut seed = entropy_seed();
    while code.len() < CODE_LEN {
        let digest = Sha256::digest(seed.to_le_bytes());
        seed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        for &byte in digest.as_slice() {
            if byte < 248 {
                code.push(CHARSET[usize::from(byte % 62)] as char);
                if code.len() == CODE_LEN {
                    break;
                }
            }
        }
    }
    code
}

/// 进程内递增计数：保证同一纳秒内的两次调用也得到不同种子。
static RANDOM_COUNTER: AtomicU64 = AtomicU64::new(0);

fn entropy_seed() -> u64 {
    let mut hasher = RandomState::new().build_hasher();
    // `RandomState` 每个实例的键来自 OS 熵；再混入时间/进程号/计数器，避免重放。
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    hasher.write_u128(nanos);
    hasher.write_u32(std::process::id());
    hasher.write_u64(RANDOM_COUNTER.fetch_add(1, Ordering::Relaxed));
    hasher.finish()
}

/// 短链入参被拒绝的原因（Go `isValidURL` 的日志分支）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum UrlRejection {
    /// 空串（Go handler 里 `longURL == ""` 单独判）。
    #[error("链接为空")]
    Empty,
    /// 不是带协议头的绝对地址（Go `url.ParseRequestURI` 失败）。
    #[error("链接不是带协议头的绝对地址")]
    NotAbsolute,
    /// 协议不是 http/https。
    #[error("只支持 http/https 协议")]
    UnsupportedScheme,
    /// 指向本机/回环地址。
    #[error("拒绝指向本机/回环地址的链接")]
    LoopbackHost,
    /// 指向私有 IPv4 段。
    #[error("拒绝指向内网地址的链接")]
    PrivateAddress,
}

/// 校验要缩短的长链（Go `isValidURL`，`shorturl.go:79-120`）。
///
/// # Errors
///
/// 见 [`UrlRejection`]；Go 侧对应日志 + 400。
pub fn validate_url(input: &str) -> Result<(), UrlRejection> {
    if input.is_empty() {
        return Err(UrlRejection::Empty);
    }
    // Go 的 `url.ParseRequestURI` 会拒绝空白与控制字符（`net/url` 的解析前置检查）。
    if input
        .chars()
        .any(|ch| ch.is_whitespace() || ch.is_control())
    {
        return Err(UrlRejection::NotAbsolute);
    }
    let Some((scheme, rest)) = input.split_once("://") else {
        return Err(UrlRejection::NotAbsolute);
    };
    // Go `getScheme` 会把协议名小写化。
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err(UrlRejection::UnsupportedScheme);
    }

    let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    if authority.is_empty() {
        // Go 的 `url.Parse("http://")` 不报错、Host 为空串，isValidURL 会放行；
        // 这里收紧：空 host 没有可跳转目标。
        return Err(UrlRejection::NotAbsolute);
    }
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);

    let host = if let Some(without_bracket) = authority.strip_prefix('[') {
        // IPv6 字面量：`[::1]:443`，方括号内是 host，冒号是六位组分隔符而非端口。
        let Some((inside, _)) = without_bracket.split_once(']') else {
            return Err(UrlRejection::NotAbsolute);
        };
        inside
    } else {
        authority.split(':').next().unwrap_or_default()
    };
    if host.is_empty() {
        return Err(UrlRejection::NotAbsolute);
    }

    // Go 的 localHostnames 是精确比较；这里对 localhost 用大小写不敏感（收紧）。
    if host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "::1" {
        return Err(UrlRejection::LoopbackHost);
    }
    if let Ok(ip) = host.parse::<IpAddr>()
        && net::is_rfc1918(ip)
    {
        return Err(UrlRejection::PrivateAddress);
    }
    Ok(())
}

/// Go `isMalicious`：`javascript:` / `data:` 前缀与常见 HTML 标签（XSS 兜底）。
#[must_use]
pub fn is_malicious(input: &str) -> bool {
    const PREFIXES: [&str; 2] = ["javascript:", "data:"];
    const TAGS: [&str; 5] = ["<script", "<img", "<iframe", "<link", "<style"];

    let lower = input.to_ascii_lowercase();
    PREFIXES.iter().any(|prefix| lower.starts_with(prefix))
        || TAGS.iter().any(|tag| lower.contains(tag))
}

/// 跳转目标（Go `RedirectFromShortURLHandler` 的方案补全）：缺协议头时补 `https://`。
#[must_use]
pub fn redirect_target(long_url: &str) -> String {
    if long_url.starts_with("http://") || long_url.starts_with("https://") {
        long_url.to_owned()
    } else {
        format!("https://{long_url}")
    }
}

/// 短链对外 base（Go `GetBaseURL`：固定 `https://<server_dir>`，不看端口/SSL 配置）。
#[must_use]
pub fn base_url(server_dir: &str) -> String {
    format!("https://{server_dir}")
}

/// 完整的短链地址（Go handler 的 `baseUrl + "/url/" + shortURL`，
/// 消息链 `message_parser.go:1788` 同样拼法）。
#[must_use]
pub fn short_url_link(server_dir: &str, code: &str) -> String {
    format!("{}/url/{code}", base_url(server_dir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn store() -> (ShortUrlStore, TempDir) {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join(DEFAULT_SHORT_URL_PATH);
        let store = ShortUrlStore::open(&path).expect("打开短链库");
        (store, dir)
    }

    #[test]
    fn hashed_code_matches_go_vector() {
        // sha256("https://example.com") 前 3 字节 = 10 06 80
        assert_eq!(hashed_code("https://example.com"), "100680");
        assert_eq!(hashed_code("https://example.com").len(), CODE_LEN);
    }

    #[test]
    fn create_then_resolve_and_survive_reopen() {
        let (store, dir) = store();
        let long = "https://example.com/a?b=c";
        let code = store.create(long).expect("生成短链");
        assert_eq!(code, hashed_code(long), "首选哈希码");
        assert_eq!(store.resolve(&code).expect("查询").as_deref(), Some(long));
        assert!(store.exists(&code).expect("存在性"));
        assert_eq!(store.resolve("missing").expect("查询"), None);
        drop(store);

        let reopened =
            ShortUrlStore::open(dir.path().join(DEFAULT_SHORT_URL_PATH)).expect("重开短链库");
        assert_eq!(
            reopened.resolve(&code).expect("查询").as_deref(),
            Some(long),
            "短链必须持久化"
        );
    }

    #[test]
    fn second_create_of_same_url_gets_a_new_random_code() {
        let (store, _dir) = store();
        let long = "https://example.com/dup";
        let first = store.create(long).expect("第一次");
        let second = store.create(long).expect("第二次");

        assert_eq!(first, hashed_code(long));
        assert_ne!(second, first, "哈希码已被占用，Go 会另取随机码");
        assert_eq!(second.len(), CODE_LEN);
        assert!(
            second.bytes().all(|byte| CHARSET.contains(&byte)),
            "随机码只含 62 个字母数字: {second}"
        );
        // 两条映射并存（Go 同样不复用）
        assert_eq!(store.resolve(&first).expect("查询"), Some(long.to_owned()));
        assert_eq!(store.resolve(&second).expect("查询"), Some(long.to_owned()));
    }

    #[test]
    fn hash_collision_with_another_url_falls_back_to_random() {
        let (store, _dir) = store();
        let long = "https://example.com/collide";
        store
            .store(&hashed_code(long), "https://other.example/")
            .expect("预占哈希码");

        let code = store.create(long).expect("生成");
        assert_ne!(code, hashed_code(long));
        assert_eq!(store.resolve(&code).expect("查询"), Some(long.to_owned()));
        assert_eq!(
            store.resolve(&hashed_code(long)).expect("查询").as_deref(),
            Some("https://other.example/"),
            "预占记录不被覆盖"
        );
    }

    #[test]
    fn random_codes_stay_in_charset_and_vary() {
        let samples: Vec<String> = (0..200).map(|_| random_code()).collect();
        for code in &samples {
            assert_eq!(code.len(), CODE_LEN);
            assert!(code.bytes().all(|byte| CHARSET.contains(&byte)));
        }
        assert!(
            samples
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                > 1,
            "随机码不应退化成常量"
        );
    }

    #[test]
    fn validate_accepts_public_http_urls() {
        for url in [
            "https://example.com",
            "https://example.com/a/b?c=d#e",
            "http://1.2.3.4:8080/path",
            "HTTP://EXAMPLE.COM/Path",
            "https://user:pass@example.com/",
            "http://172.32.0.1/",
            // Go 只精确拦 127.0.0.1，127.0.0.2 会放行——保留该行为
            "http://127.0.0.2/",
        ] {
            assert_eq!(validate_url(url), Ok(()), "应接受 {url}");
        }
    }

    #[test]
    fn validate_rejects_unusable_or_internal_targets() {
        assert_eq!(validate_url(""), Err(UrlRejection::Empty));
        assert_eq!(validate_url("example.com"), Err(UrlRejection::NotAbsolute));
        assert_eq!(validate_url("http://"), Err(UrlRejection::NotAbsolute));
        assert_eq!(
            validate_url("https://exa mple.com"),
            Err(UrlRejection::NotAbsolute)
        );
        assert_eq!(
            validate_url("ftp://example.com"),
            Err(UrlRejection::UnsupportedScheme)
        );
        for url in [
            "http://localhost:8080/",
            "http://LocalHost/",
            "http://127.0.0.1/",
            "http://[::1]:8080/",
        ] {
            assert_eq!(validate_url(url), Err(UrlRejection::LoopbackHost), "{url}");
        }
        for url in [
            "http://10.1.2.3/",
            "http://172.16.0.1/",
            "http://172.31.255.255/",
            "http://192.168.1.1/",
            "http://[::ffff:192.168.1.1]/",
        ] {
            assert_eq!(
                validate_url(url),
                Err(UrlRejection::PrivateAddress),
                "{url}"
            );
        }
    }

    #[test]
    fn xss_payloads_are_malicious() {
        for payload in [
            "javascript:alert(1)",
            "JavaScript:alert(1)",
            "data:text/html;base64,PHNjcmlwdD4=",
            "https://example.com/<script>alert(1)</script>",
            "https://example.com/<iframe src=x>",
        ] {
            assert!(is_malicious(payload), "应判为恶意: {payload}");
        }
        assert!(!is_malicious("https://example.com/?a=1&b=2"));
    }

    #[test]
    fn redirect_adds_https_only_when_scheme_missing() {
        assert_eq!(redirect_target("example.com/x"), "https://example.com/x");
        assert_eq!(redirect_target("http://example.com"), "http://example.com");
        assert_eq!(
            redirect_target("https://example.com"),
            "https://example.com"
        );
        assert_eq!(REDIRECT_STATUS, 301);
    }

    #[test]
    fn links_are_composed_like_go() {
        assert_eq!(base_url("1.2.3.4:5700"), "https://1.2.3.4:5700");
        assert_eq!(
            short_url_link("1.2.3.4:5700", "abc123"),
            "https://1.2.3.4:5700/url/abc123"
        );
    }
}

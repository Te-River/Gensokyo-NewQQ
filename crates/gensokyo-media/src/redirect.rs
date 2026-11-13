//! 短签名图床外链 → 直链解析（对齐 Go `imagehosting/selffetch.go` 的 `ResolveRedirectURL`）。
//!
//! ChatGLM（`oss_type = 7`）返回的 `result.file_url` 是带短签名的外链（约 30 分钟有效），
//! 直接写进出站字段时 QQ 转存拉取失败、用户查看时签名已过期。Go 的做法是先按域名判定
//! （`IsShortLivedImageURL`，清单只有 `chatglm.cn` 及其子域），命中则把短链的 302 解析成
//! OSS 预签名直链（约 15 分钟有效）——发送时刻拉取得到就足够。
//!
//! # 规则（逐条对齐 Go）
//! - **只读响应头**：请求带 `Range: bytes=0-0`，拿到响应立即丢弃正文；出站只需要 URL。
//! - **逐跳手动跟随**：必须先关掉 `reqwest` 的自动重定向，才能对每一跳目标做 SSRF 判定。
//! - **每跳校验**：只允许 http(s)；host 解析不出、或解析到私有 / 回环 / 链路本地地址即拒绝
//!   （域名先解析、逐个 IP 判定，解析失败也拒绝）。口径同 Go `isPrivateOrLoopback`，
//!   另加 `0.0.0.0` / `::`（Go 未列，属于更严格的补充）。IP 判定复用
//!   `gensokyo_core::net`（RFC1918 + 可组合的回环 / 链路本地 / 未指定 / ULA）；
//!   短链校验只用 RFC1918，是刻意差异（它要逐字节对齐 Go 的放行行为），见该模块文档。
//! - **上限**：最多跟随 [`MAX_REDIRECTS`] 跳，整链总超时 [`RESOLVE_TIMEOUT`]。
//! - **必须真的发生重定向**：末跳 2xx 且地址与入参不同才算成功（Go 同口径；把原短链交回去
//!   没有意义，交给调用方走降级语义）。
//!
//! # 残余风险
//! 判定用的是解析结果，真正建连时 `reqwest` 会再解析一次 DNS，理论上存在 rebinding 窗口；
//! Go 的实现同样如此，这是与 Go 一致的已知边界。
//!
//! # 共用
//! 逐跳判定（`target_allowed`）与跳数上限（`MAX_REDIRECTS`）同时被通用下载路径
//! （`fetch::HttpTransport`，P56）复用：守卫只有这一处实现。下载路径与 Go 的差异
//! （5 跳 vs 10 跳、reply_helpers 缺口收紧等）登记在 crate 级文档的「P56」节。

use std::net::{IpAddr, ToSocketAddrs};
use std::sync::OnceLock;
use std::time::Duration;

use gensokyo_core::net;
use reqwest::Url;

use crate::error::{Error, Result};
use crate::fetch::{BoxFuture, HttpTransport};
use crate::hosting::transport::USER_AGENT;

/// 需要解析直链的短签名图床域名（对齐 Go `shortLivedImageHostSuffixes`，含子域）。
const SHORT_LIVED_HOST_SUFFIXES: &[&str] = &["chatglm.cn"];

/// 重定向跳数上限（不含首跳，对齐 Go `resolveMaxHops`）。
/// 通用下载路径（`fetch::HttpTransport`）共用同一上限。
pub(crate) const MAX_REDIRECTS: usize = 5;

/// 整条解析链的总超时（Go `resolveTimeout` 是 10s，这里按 P25 口径收紧到 5s）。
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(5);

/// `Range` 头：图床至多回 1 字节，正文一个字节都不读（Go `ResolveRedirectURL` 同口径）。
const RANGE_HEADER_VALUE: &str = "bytes=0-0";

/// 一次不跟随重定向的响应，只需要状态码与 `Location`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeResponse {
    /// HTTP 状态码。
    pub status: u16,
    /// `Location` 头（原样，可能是相对地址）。
    pub location: Option<String>,
}

/// 直链解析需要的两个网络动作：发一次不跟随重定向的 GET、解析主机名。
///
/// 放在同一个 trait 里由调用方注入：测试可以完全离线地构造 302 链与域名解析结果，
/// 生产实现 [`LazyHttpProbe`] 复用内置根证书与浏览器 UA。
pub trait RedirectProbe: Send + Sync + std::fmt::Debug {
    /// GET `url`，返回状态码与 `Location`，不读响应体。
    fn probe(&self, url: String, timeout: Duration) -> BoxFuture<'_, Result<ProbeResponse>>;
    /// 解析主机名（IP 字面量由调用方直接判定，不会走到这里）。
    fn resolve(&self, host: String, port: u16) -> BoxFuture<'_, Result<Vec<IpAddr>>>;
}

/// 生产探测器：不跟随重定向的 `reqwest` 客户端 + 系统 DNS。
///
/// 客户端在进程内首次解析时构建并复用；二进制需先安装 rustls `CryptoProvider`
/// （与 [`HttpTransport::new`] 同一前提），未安装时解析直接失败、调用方保留原短链。
#[derive(Debug, Default, Clone, Copy)]
pub struct LazyHttpProbe;

impl RedirectProbe for LazyHttpProbe {
    fn probe(&self, url: String, timeout: Duration) -> BoxFuture<'_, Result<ProbeResponse>> {
        let client = probe_client().cloned();
        Box::pin(async move {
            let client = client?;
            let response = client
                .get(&url)
                .header(reqwest::header::USER_AGENT, USER_AGENT)
                .header(reqwest::header::RANGE, RANGE_HEADER_VALUE)
                .timeout(timeout)
                .send()
                .await
                .map_err(|err| Error::Transport(err.to_string()))?;
            let status = response.status().as_u16();
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            Ok(ProbeResponse { status, location })
        })
    }

    fn resolve(&self, host: String, port: u16) -> BoxFuture<'_, Result<Vec<IpAddr>>> {
        Box::pin(async move {
            // `ToSocketAddrs` 是阻塞调用，放进 blocking 池，避免卡住 runtime worker
            let addresses = tokio::task::spawn_blocking(move || {
                (host.as_str(), port)
                    .to_socket_addrs()
                    .map(|addresses| addresses.map(|address| address.ip()).collect::<Vec<_>>())
            })
            .await
            .map_err(|err| Error::Transport(format!("DNS 解析任务失败: {err}")))?;
            addresses.map_err(|err| Error::Transport(format!("DNS 解析失败: {err}")))
        })
    }
}

/// 进程级共享的「不跟随重定向」客户端：首次使用时构建，之后复用连接池。
fn probe_client() -> Result<&'static reqwest::Client> {
    static CLIENT: OnceLock<std::result::Result<reqwest::Client, String>> = OnceLock::new();
    match CLIENT.get_or_init(|| HttpTransport::no_redirect_client().map_err(|err| err.to_string()))
    {
        Ok(client) => Ok(client),
        Err(message) => Err(Error::Transport(message.clone())),
    }
}

/// 判断 URL 是否为需要解析直链的短签名外链（Go `IsShortLivedImageURL`）。
///
/// 按 host 边界判定：host 等于清单域名、或以 `.域名` 结尾（覆盖 `t1.` 等子域）；
/// 兼容 legacy foundItems 剥离 scheme 的裸域形态（先原样解析，取不到 host 再补成绝对 URL 重解）。
#[must_use]
pub fn is_short_lived_image_url(raw: &str) -> bool {
    let Some(host) = url_host(raw) else {
        return false;
    };
    SHORT_LIVED_HOST_SUFFIXES.iter().any(|suffix| {
        host == *suffix
            || host
                .strip_suffix(suffix)
                .is_some_and(|head| head.ends_with('.'))
    })
}

/// 解析 URL 的 host（小写、去端口）；解析不出 host 时返回 `None`。
fn url_host(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    if let Ok(url) = Url::parse(raw)
        && let Some(host) = url.host_str()
    {
        return Some(host.to_ascii_lowercase());
    }
    // 裸域 / "域:端口/路径"：Go 的 `imageURLHost` 同样补 "//" 再解一次（这里补成绝对 URL）
    let absolute = if let Some(rest) = raw.strip_prefix("//") {
        format!("https:{rest}")
    } else {
        format!("https://{raw}")
    };
    Url::parse(&absolute)
        .ok()?
        .host_str()
        .map(str::to_ascii_lowercase)
}

/// 把短签名外链解析为末跳直链；任何失败（SSRF 拒绝 / 超时 / 跳数超限 / 末跳非 2xx /
/// 未发生重定向）一律返回 `None`，由调用方保留原 URL。
///
/// `probe` 是网络出口的接缝；生产用 [`LazyHttpProbe`]。
pub async fn resolve_redirect(probe: &dyn RedirectProbe, raw: &str) -> Option<String> {
    tokio::time::timeout(RESOLVE_TIMEOUT, follow(probe, raw))
        .await
        .ok()
        .flatten()
}

/// 逐跳跟随重定向（总超时由 [`resolve_redirect`] 负责）。
async fn follow(probe: &dyn RedirectProbe, raw: &str) -> Option<String> {
    let original = parse_target(raw)?;
    if !target_allowed(probe, &original).await {
        return None;
    }
    let mut current = original.clone();
    // 最多发出 `MAX_REDIRECTS + 1` 次请求：每次要么拿到 2xx 收尾，要么消耗一跳
    for _ in 0..=MAX_REDIRECTS {
        let response = probe
            .probe(current.as_str().to_owned(), RESOLVE_TIMEOUT)
            .await
            .ok()?;
        match response.status {
            200..=299 => return (current != original).then(|| current.to_string()),
            301 | 302 | 303 | 307 | 308 => {
                let next = current.join(response.location.as_deref()?).ok()?;
                if !target_allowed(probe, &next).await {
                    return None;
                }
                current = next;
            }
            _ => return None,
        }
    }
    None
}

/// 还原完整的绝对 URL：已带 scheme 的原样用；协议相对（`//host/path`）与 legacy 裸域形态
/// （`host/path`）统一按 https 补全（短签名外链都是 https）。
fn parse_target(raw: &str) -> Option<Url> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let absolute = if let Some(rest) = raw.strip_prefix("//") {
        format!("https:{rest}")
    } else if Url::parse(raw).is_ok_and(|url| url.host_str().is_some()) {
        raw.to_owned()
    } else {
        format!("https://{raw}")
    };
    let url = Url::parse(&absolute).ok()?;
    url.host_str()?;
    Some(url)
}

/// 单跳目标是否允许访问：协议、host 与 SSRF 判定（域名先解析再逐个 IP 判）。
/// 通用下载路径（`fetch::HttpTransport`）逐跳复用本判定。
pub(crate) async fn target_allowed(probe: &dyn RedirectProbe, url: &Url) -> bool {
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    // `host_str` 对 IPv6 保留方括号（`[::1]`），去掉后再按字面量判定
    let literal = host
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(host);
    if let Ok(ip) = literal.parse::<IpAddr>() {
        return !is_blocked_ip(ip);
    }
    let port = url.port_or_known_default().unwrap_or(443);
    match probe.resolve(host.to_owned(), port).await {
        // Go：任一解析结果为私有 / 回环 / 链路本地即拒绝
        Ok(addresses) => !addresses.is_empty() && addresses.iter().all(|ip| !is_blocked_ip(*ip)),
        Err(_) => false,
    }
}

/// 私有 / 回环 / 链路本地地址判定（Go `ip.IsPrivate() || ip.IsLoopback() || ip.IsLinkLocalUnicast()`，
/// 另把 `0.0.0.0` / `::` 与 IPv4-mapped IPv6 一并归入）。各条规则在 `gensokyo_core::net`，
/// 这里只做组合；行为与拆分前逐条一致。
fn is_blocked_ip(ip: IpAddr) -> bool {
    net::is_rfc1918(ip)
        || net::is_loopback(ip)
        || net::is_link_local(ip)
        || net::is_unspecified(ip)
        || net::is_unique_local(ip)
}

#[cfg(test)]
pub(crate) mod fixtures {
    use std::collections::HashMap;
    use std::net::{IpAddr, Ipv4Addr};
    use std::str::FromStr;
    use std::sync::Mutex;
    use std::time::Duration;

    use super::{ProbeResponse, RedirectProbe};
    use crate::error::{Error, Result};
    use crate::fetch::BoxFuture;

    /// 默认解析结果：TEST-NET-3（公网），只关心跳转链的用例不必重复预置。
    const PUBLIC_IP: Ipv4Addr = Ipv4Addr::new(203, 0, 113, 7);

    /// 离线探测器：按 URL 预置响应、按主机名预置解析结果，并记录每次请求。
    #[derive(Debug, Default)]
    pub(crate) struct FakeProbe {
        calls: Mutex<Vec<String>>,
        responses: Mutex<HashMap<String, ProbeResponse>>,
        hosts: Mutex<HashMap<String, Vec<IpAddr>>>,
        failures: Mutex<Vec<String>>,
    }

    impl FakeProbe {
        pub(crate) fn new() -> Self {
            Self::default()
        }

        /// 预置 `url` 的响应；未预置的 URL 视为探测失败。
        pub(crate) fn respond(self, url: &str, status: u16, location: Option<&str>) -> Self {
            self.responses.lock().unwrap().insert(
                url.to_owned(),
                ProbeResponse {
                    status,
                    location: location.map(str::to_owned),
                },
            );
            self
        }

        /// 预置 `host` 的解析结果；未预置的域名解析到 [`PUBLIC_IP`]。
        pub(crate) fn resolve_to(self, host: &str, ips: &[&str]) -> Self {
            self.hosts.lock().unwrap().insert(
                host.to_owned(),
                ips.iter()
                    .map(|ip| IpAddr::from_str(ip).expect("测试用 IP 非法"))
                    .collect(),
            );
            self
        }

        /// 预置 `host` 解析失败（SSRF 口径：解析失败即拒绝）。
        pub(crate) fn fail_resolve(self, host: &str) -> Self {
            self.failures.lock().unwrap().push(host.to_owned());
            self
        }

        pub(crate) fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl RedirectProbe for FakeProbe {
        fn probe(&self, url: String, _timeout: Duration) -> BoxFuture<'_, Result<ProbeResponse>> {
            Box::pin(async move {
                self.calls.lock().unwrap().push(url.clone());
                let reply = self.responses.lock().unwrap().get(&url).cloned();
                reply.ok_or_else(|| Error::Transport(format!("未预置响应: {url}")))
            })
        }

        fn resolve(&self, host: String, _port: u16) -> BoxFuture<'_, Result<Vec<IpAddr>>> {
            Box::pin(async move {
                if self
                    .failures
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|failed| failed == &host)
                {
                    return Err(Error::Transport(format!("DNS 解析失败（预置）: {host}")));
                }
                Ok(self
                    .hosts
                    .lock()
                    .unwrap()
                    .get(&host)
                    .cloned()
                    .unwrap_or_else(|| vec![IpAddr::V4(PUBLIC_IP)]))
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_REDIRECTS, is_short_lived_image_url, resolve_redirect};
    use crate::redirect::fixtures::FakeProbe;

    const SHORT: &str = "https://t1.chatglm.cn/files/a.png?sig=short";

    #[test]
    fn matches_only_short_lived_hosts() {
        for hit in [
            "https://chatglm.cn/files/a.png",
            "https://t1.chatglm.cn/a.png?sig=1",
            "chatglm.cn/oss/a.png?sign=abc//def",
            "t1.chatglm.cn:443/a.png?sig=a//b",
        ] {
            assert!(is_short_lived_image_url(hit), "{hit}");
        }
        for miss in [
            "https://evil-chatglm.cn.example.com/a.png",
            "https://chatglm.cn.evil.com/a.png",
            "https://example.com/a.png",
            "",
        ] {
            assert!(!is_short_lived_image_url(miss), "{miss}");
        }
    }

    #[tokio::test]
    async fn follows_two_hop_chain_to_final_url() {
        let probe = FakeProbe::new()
            .respond(SHORT, 302, Some("https://cdn.example.net/step2.png?sig=b"))
            .respond(
                "https://cdn.example.net/step2.png?sig=b",
                302,
                Some("https://oss.example.com/final.png?sig=c"),
            )
            .respond("https://oss.example.com/final.png?sig=c", 200, None);

        assert_eq!(
            resolve_redirect(&probe, SHORT).await.as_deref(),
            Some("https://oss.example.com/final.png?sig=c")
        );
        assert_eq!(
            probe.calls(),
            vec![
                SHORT.to_owned(),
                "https://cdn.example.net/step2.png?sig=b".to_owned(),
                "https://oss.example.com/final.png?sig=c".to_owned(),
            ]
        );
    }

    #[tokio::test]
    async fn rejects_private_loopback_and_link_local_targets_per_hop() {
        for target in [
            "http://127.0.0.1/a.png",
            "http://10.1.2.3/a.png",
            "http://172.16.0.1/a.png",
            "http://192.168.1.1/a.png",
            "http://169.254.169.254/latest/meta-data",
            "http://[::1]/a.png",
            "http://0.0.0.0/a.png",
        ] {
            let probe = FakeProbe::new().respond(SHORT, 302, Some(target));
            assert_eq!(resolve_redirect(&probe, SHORT).await, None, "{target}");
            // 只探测了首跳：私有目标那一跳在校验阶段就被拒，未发出请求
            assert_eq!(probe.calls(), vec![SHORT.to_owned()], "{target}");
        }
    }

    #[tokio::test]
    async fn rejects_hostnames_resolving_to_private_addresses() {
        let probe = FakeProbe::new()
            .respond(SHORT, 302, Some("https://intranet.example.com/a.png"))
            .resolve_to("intranet.example.com", &["10.0.0.5"]);

        assert_eq!(resolve_redirect(&probe, SHORT).await, None);
        assert_eq!(probe.calls(), vec![SHORT.to_owned()]);
    }

    #[tokio::test]
    async fn gives_up_after_hop_limit() {
        let mut probe = FakeProbe::new();
        for hop in 0..=MAX_REDIRECTS {
            probe = probe.respond(
                &format!("https://hop{hop}.example.com/a.png"),
                302,
                Some(&format!("https://hop{}.example.com/a.png", hop + 1)),
            );
        }

        assert_eq!(
            resolve_redirect(&probe, "https://hop0.example.com/a.png").await,
            None
        );
        assert_eq!(probe.calls().len(), MAX_REDIRECTS + 1);
    }

    #[tokio::test]
    async fn rejects_non_http_targets() {
        for target in [
            "file:///etc/passwd",
            "ftp://example.com/a.png",
            "data:image/png;base64,AAAA",
        ] {
            let probe = FakeProbe::new().respond(SHORT, 302, Some(target));
            assert_eq!(resolve_redirect(&probe, SHORT).await, None, "{target}");
            assert_eq!(probe.calls(), vec![SHORT.to_owned()], "{target}");
        }
        // 首跳本身不是 http(s)：连请求都不发
        let probe = FakeProbe::new();
        assert_eq!(
            resolve_redirect(&probe, "ftp://chatglm.cn/a.png").await,
            None
        );
        assert!(probe.calls().is_empty());
    }

    #[tokio::test]
    async fn returns_none_without_redirect() {
        let probe = FakeProbe::new().respond(SHORT, 200, None);
        assert_eq!(resolve_redirect(&probe, SHORT).await, None);
        assert_eq!(probe.calls(), vec![SHORT.to_owned()]);
    }
}

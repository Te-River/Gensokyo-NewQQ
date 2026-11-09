//! IP 地址判定的单一事实源。
//!
//! 两个调用方的口径**刻意不同**，但共享同一套基础判定：
//!
//! - `gensokyo-storage::short_url` 对齐 Go `isPrivateIP`，只认 RFC1918 三段
//!   （[`is_rfc1918`]）；
//! - `gensokyo-media::redirect` 对齐 Go `isPrivateOrLoopback` 的 SSRF 守卫，在 RFC1918
//!   之外还拦回环 / 链路本地 / 未指定 / IPv6 唯一本地（[`is_loopback`] 等可组合判定）。
//!
//! 差异是刻意保留的：短链校验必须逐字节对齐 Go 的放行行为（`127.0.0.2` 照样放行），
//! 不能顺手套用更严的 SSRF 口径。所以这里只提供可组合的判定，不提供"包打天下"的单一函数；
//! 两个模块各自组合，但每条规则只有这一处实现。所有 IPv4-mapped IPv6 先归一到 IPv4。

use std::net::IpAddr;

/// IPv4-mapped IPv6（`::ffff:a.b.c.d`）归一到 IPv4；其它地址原样返回。
fn normalized(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V4(_) => ip,
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
    }
}

/// RFC1918 三段私网：`10/8`、`172.16/12`、`192.168/16`。
///
/// 对应 Go `net.IP.IsPrivate` 的 IPv4 部分；IPv4-mapped IPv6 先归一
/// （`::ffff:192.168.1.1` 同样命中）。短链校验用这个口径。
#[must_use]
pub fn is_rfc1918(ip: IpAddr) -> bool {
    match normalized(ip) {
        IpAddr::V4(v4) => v4.is_private(),
        IpAddr::V6(_) => false,
    }
}

/// 回环地址：IPv4 `127/8`、IPv6 `::1`；IPv4-mapped 先归一。
#[must_use]
pub fn is_loopback(ip: IpAddr) -> bool {
    match normalized(ip) {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => v6.is_loopback(),
    }
}

/// 链路本地地址：IPv4 `169.254/16`、IPv6 `fe80::/10`；IPv4-mapped 先归一。
#[must_use]
pub fn is_link_local(ip: IpAddr) -> bool {
    match normalized(ip) {
        IpAddr::V4(v4) => v4.is_link_local(),
        IpAddr::V6(v6) => v6.is_unicast_link_local(),
    }
}

/// 未指定地址：`0.0.0.0` / `::`；IPv4-mapped 先归一。
#[must_use]
pub fn is_unspecified(ip: IpAddr) -> bool {
    match normalized(ip) {
        IpAddr::V4(v4) => v4.is_unspecified(),
        IpAddr::V6(v6) => v6.is_unspecified(),
    }
}

/// IPv6 唯一本地地址（`fc00::/7`）；IPv4 恒为 `false`。
#[must_use]
pub fn is_unique_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(_) => false,
        IpAddr::V6(v6) => v6.is_unique_local(),
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, Ipv6Addr};

    use super::*;

    #[test]
    fn rfc1918_matches_the_three_go_ranges() {
        for ip in [
            Ipv4Addr::new(10, 0, 0, 1),
            Ipv4Addr::new(172, 16, 0, 1),
            Ipv4Addr::new(172, 31, 255, 255),
            Ipv4Addr::new(192, 168, 1, 1),
        ] {
            assert!(is_rfc1918(IpAddr::V4(ip)), "应判为 RFC1918: {ip}");
        }
        for ip in [
            Ipv4Addr::new(8, 8, 8, 8),
            Ipv4Addr::new(11, 0, 0, 1),
            Ipv4Addr::new(172, 15, 255, 255),
            Ipv4Addr::new(172, 32, 0, 1),
            Ipv4Addr::new(192, 169, 0, 1),
        ] {
            assert!(!is_rfc1918(IpAddr::V4(ip)), "不应判为 RFC1918: {ip}");
        }
        // IPv4-mapped IPv6 归一到 IPv4 再判
        assert!(is_rfc1918("::ffff:192.168.1.1".parse().unwrap()));
        assert!(!is_rfc1918("fd00::1".parse().unwrap()));
    }

    #[test]
    fn extra_predicates_cover_redirect_rules() {
        assert!(is_loopback(Ipv4Addr::LOCALHOST.into()));
        assert!(is_loopback(Ipv6Addr::LOCALHOST.into()));
        assert!(is_loopback("::ffff:127.0.0.1".parse().unwrap()));
        assert!(!is_loopback(Ipv4Addr::new(128, 0, 0, 1).into()));

        assert!(is_link_local(Ipv4Addr::new(169, 254, 1, 1).into()));
        assert!(is_link_local("fe80::1".parse().unwrap()));
        assert!(is_link_local("::ffff:169.254.1.1".parse().unwrap()));
        assert!(!is_link_local(Ipv4Addr::new(8, 8, 8, 8).into()));

        assert!(is_unspecified(Ipv4Addr::UNSPECIFIED.into()));
        assert!(is_unspecified(Ipv6Addr::UNSPECIFIED.into()));
        assert!(is_unspecified("::ffff:0.0.0.0".parse().unwrap()));
        assert!(!is_unspecified(Ipv4Addr::LOCALHOST.into()));

        assert!(is_unique_local("fc00::1".parse().unwrap()));
        assert!(is_unique_local("fd12::1".parse().unwrap()));
        assert!(!is_unique_local(Ipv4Addr::new(10, 0, 0, 1).into()));
    }
}

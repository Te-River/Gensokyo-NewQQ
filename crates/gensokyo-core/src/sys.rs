//! 本机网络信息：Go `sys/re.go` 的 `GetLocalIP` 等价实现。
//!
//! 首启生成 `config.yml` 时用它替换 `<YOUR_SERVER_DIR>` 占位符。

use std::net::{IpAddr, Ipv4Addr};

/// 返回本机内网 IPv4 地址。
///
/// 选择顺序与 Go `sys.GetLocalIP` 一致，只有三档：`192.*` → `10.*` → `172.16.0.0/12`。
/// 三档都取不到时返回 `None`（调用方保留模板占位符）。Go 没有"首个非回环 IPv4"
/// 的兜底档，这里也不加：那会把公网/链路本地地址误当内网 IP 填进模板。
#[must_use]
pub fn local_ip() -> Option<IpAddr> {
    let mut candidates = Vec::new();
    for interface in if_addrs::get_if_addrs().ok()? {
        if let if_addrs::IfAddr::V4(v4) = interface.addr {
            candidates.push(IpAddr::V4(v4.ip));
        }
    }
    pick_local_ip(&candidates)
}

/// 从候选地址中按 Go 的优先级挑一个：首个 `192.*` > 首个 `10.*` > 首个 `172.16~31.*`。
///
/// 前缀判断沿用 Go 的字符串切片语义（`ip[:3] == "192"` / `ip[:2] == "10"`），
/// 因此 `100.*`（CGNAT）同样命中 `10` 档——这是 Go 原实现的既有行为，为对齐而保留。
/// 回环地址跳过；三个档位都不命中时返回 `None`，绝不回退到"第一个非回环地址"。
fn pick_local_ip(candidates: &[IpAddr]) -> Option<IpAddr> {
    let mut preferred: Option<Ipv4Addr> = None;
    let mut ten: Option<Ipv4Addr> = None;
    let mut private_172: Option<Ipv4Addr> = None;

    for &candidate in candidates {
        let IpAddr::V4(ip) = candidate else {
            continue;
        };
        if ip.is_loopback() {
            continue;
        }
        // 档位互斥，等价于 Go 的 if/else if 链：先命中的 192 不会被后续地址覆盖
        if ip.to_string().starts_with("192") {
            preferred.get_or_insert(ip);
        } else if ip.to_string().starts_with("10") {
            ten.get_or_insert(ip);
        } else if ip.octets()[0] == 172 && (16..=31).contains(&ip.octets()[1]) {
            private_172.get_or_insert(ip);
        }
    }

    preferred.or(ten).or(private_172).map(IpAddr::V4)
}

#[cfg(test)]
mod tests {
    use std::net::Ipv6Addr;

    use super::*;

    fn v4(octets: [u8; 4]) -> IpAddr {
        IpAddr::V4(Ipv4Addr::from(octets))
    }

    #[test]
    fn local_ip_is_ipv4_or_none() {
        // 机器可能没有任何候选地址（容器/离线 CI），两种结果都合法
        match local_ip() {
            Some(IpAddr::V4(ip)) => {
                assert!(!ip.is_loopback());
                assert!(!ip.is_unspecified());
            }
            Some(IpAddr::V6(_)) => panic!("只应返回 IPv4"),
            None => {}
        }
    }

    #[test]
    fn pick_prefers_192_then_10_then_172() {
        let candidates = [v4([172, 16, 0, 1]), v4([10, 1, 2, 3]), v4([192, 168, 0, 5])];
        assert_eq!(pick_local_ip(&candidates), Some(v4([192, 168, 0, 5])));
        assert_eq!(pick_local_ip(&candidates[..2]), Some(v4([10, 1, 2, 3])));
        assert_eq!(pick_local_ip(&candidates[..1]), Some(v4([172, 16, 0, 1])));
    }

    #[test]
    fn pick_returns_none_without_a_private_candidate() {
        // 没有第 4 档兜底：只剩公网/链路本地地址时返回 None（模板保留占位符）
        assert_eq!(
            pick_local_ip(&[v4([8, 8, 8, 8]), v4([169, 254, 1, 1])]),
            None
        );
        // 172.16~172.31 之外的 172.* 不算内网段
        assert_eq!(pick_local_ip(&[v4([172, 15, 0, 1])]), None);
        assert_eq!(pick_local_ip(&[v4([172, 32, 0, 1])]), None);
        // 回环地址跳过，IPv6 候选不参与
        assert_eq!(
            pick_local_ip(&[
                IpAddr::V4(Ipv4Addr::LOCALHOST),
                IpAddr::V6(Ipv6Addr::LOCALHOST),
            ]),
            None
        );
        // 空候选列表
        assert_eq!(pick_local_ip(&[]), None);
    }

    #[test]
    fn pick_matches_go_string_prefix_for_ten() {
        // Go 用 `ip[:2] == "10"` 判断，100.64.0.0/10（CGNAT）同样命中 10 档；为对齐保留
        assert_eq!(
            pick_local_ip(&[v4([100, 64, 0, 1])]),
            Some(v4([100, 64, 0, 1]))
        );
    }
}

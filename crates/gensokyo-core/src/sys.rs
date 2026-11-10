//! 本机网络与进程信息：Go `sys/` 的只读部分（`GetLocalIP`、`GetExecutableName`）。
//!
//! 首启生成 `config.yml` 时用 [`local_ip`] 替换 `<YOUR_SERVER_DIR>` 占位符。
//!
//! # 未实现（Go 有、Rust 无等价入口，登记在案）
//!
//! - **重启**：`RestartApplication`（`sys/re.go:172-193` + `restart_windows.go` /
//!   `restart_unix.go`）写 `restart.bat` / `restart.sh` 拉起新进程后 `os.Exit(0)`，并留下
//!   `restart.flag`。Rust 的进程生命周期由 runtime/boot 管理，重启涉及停机与端口交接，
//!   本 crate 不做；WebUI 的"重启应用"入口在 runtime 侧。
//! - **双击检测 / 安全启动脚本**：`RunningByDoubleClick` / `NoMoreDoubleClick`
//!   （`sys/safelunch*.go`）依赖 Win32 `MessageBoxW` + `GetConsoleProcessList` 并释出
//!   `gensokyo.bat`，只在交互式 Windows 双击场景生效；Rust 无同类入口。
//! - **控制台标题**：`SetTitle`（`sys/re.go:164-169`，Windows `SetConsoleTitleW` /
//!   Linux ANSI 转义），无调用方。
//! - **公网 IP**：`GetPublicIP`（`sys/re.go:62-97`）请求固定第三方地址再正则抓 IP，只有
//!   `config.ComposeWebUIURLv2`（`config.go:1248`）使用；core 不引 HTTP 客户端，缺口在册。
//!   用它拼展示地址还依赖该第三方站点存活，重写时宜换成本地探测。
//! - **`SystemInfo` / `executable_name` 已接线**（P36）：WebUI `/api/status` 的扩展字段
//!   （`os` / `arch` / `executable` / `local_ip`）由 [`SystemInfo::collect`] 投影；
//!   `GetPublicIP` 不做（见上一条）。

use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;

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

/// 当前可执行文件去掉扩展名的路径（Go `sys.GetExecutableName`，`sys/re.go:116-123`）。
///
/// Go 用 `strings.TrimSuffix(executable, filepath.Ext(executable))`，对
/// `C:\x\gensokyo.exe` 得到 `C:\x\gensokyo`；[`PathBuf::with_extension`] 传空串是同一语义
/// （只去掉文件名最后一段扩展名，目录里的点不受影响）。
///
/// 取不到时返回 `None`（Go 返回 error）；调用方自行决定是否致命。
#[must_use]
pub fn executable_name() -> Option<PathBuf> {
    Some(std::env::current_exe().ok()?.with_extension(""))
}

/// 本机 / 当前进程的只读信息快照。
///
/// Go 没有单个结构体，这些字段散在 `sys.GetLocalIP`（`sys/re.go:25`）、`os.Executable`、
/// `os.Getpid` 与 `runtime.GOOS/GOARCH`。Rust 侧给成一个可打印/可注入的形状，供日志与
/// WebUI 状态接口使用（WebUI `/api/status` 的字段投影在 `gensokyo-webui::status`，
/// 比本结构体多 CPU/内存/磁盘，那些需要 gopsutil 等价物，不在 core 范围）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemInfo {
    /// 操作系统（`std::env::consts::OS`，对应 Go `runtime.GOOS`）。
    pub os: &'static str,
    /// CPU 架构（`std::env::consts::ARCH`，对应 Go `runtime.GOARCH`）。
    pub arch: &'static str,
    /// 当前进程号（Go `os.Getpid()`）。
    pub pid: u32,
    /// 可执行文件（去扩展名，见 [`executable_name`]）。
    pub executable: Option<PathBuf>,
    /// 本机内网 IPv4（见 [`local_ip`]）。
    pub local_ip: Option<IpAddr>,
}

impl SystemInfo {
    /// 采集当前进程与本机信息；任何一项取不到就留 `None`，不报错。
    #[must_use]
    pub fn collect() -> Self {
        Self {
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            pid: std::process::id(),
            executable: executable_name(),
            local_ip: local_ip(),
        }
    }
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

    #[test]
    fn executable_name_has_no_extension() {
        // 测试进程一定有可执行文件路径；各平台扩展名（`.exe`）都应被去掉
        let name = executable_name().expect("当前可执行文件路径");
        assert!(name.is_absolute(), "应保留绝对路径: {name:?}");
        assert!(
            name.extension().is_none(),
            "Go GetExecutableName 去掉扩展名: {name:?}"
        );
    }

    #[test]
    fn system_info_shape_is_consistent() {
        let info = SystemInfo::collect();
        assert!(
            matches!(info.os, "windows" | "linux" | "macos"),
            "os 应为 std 常量: {}",
            info.os
        );
        assert!(!info.arch.is_empty());
        assert!(info.pid > 0);
        assert_eq!(info.local_ip, local_ip(), "local_ip 与独立函数同源");
        if let Some(executable) = &info.executable {
            assert!(executable.is_absolute());
            assert!(executable.extension().is_none());
        }
    }
}

//! `GET /api/status`：机器与当前进程状态（Go `webui/monitor_real.go:handleSysInfo` 的字段投影）。
//!
//! 字段与 Go 响应逐字段同名同型（前端 `SystemStatus` 直接消费，字段缺失会读到 `undefined`，
//! 因此形状必须完整）。**取值来源**：Go 用 gopsutil 采 CPU / 内存 / 磁盘 / 系统启动时间，
//! 本 crate 不引系统信息依赖，P5 reporter 的采集源也尚未对外提供，所以除
//! `process.pid`（`std::process::id()`）与 `process.status`（恒为 `running`）外一律返回 `0`。
//! `0` 在这里表示"未采集"，不是真实读数；接入采集源时只改 [`SystemStatus::current`]，
//! 路由与响应形状不变。
//!
//! # Rust 扩展字段（Go 没有）
//!
//! `os` / `arch` / `executable` / `local_ip` 来自 [`gensokyo_core::sys::SystemInfo`]（P36 接线），
//! Go 的 `handleSysInfo` 没有这几项——它们是 [`gensokyo_core::sys`] 的消费点，补成额外字段
//! 而不是改动既有 5 个字段：前端的 `SystemStatus` 只读自己认识的键，多出来的键无影响。
//! `executable` 对应 Go `sys.GetExecutableName`（`sys/re.go:116-123`）。

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use gensokyo_core::sys::SystemInfo;

use crate::auth::{is_authenticated, unauthorized};
use crate::state::WebUiState;

/// 内存使用情况（Go `mem.VirtualMemory()` 的投影）。
#[derive(Debug, Serialize)]
pub struct MemoryDetail {
    /// 总内存（字节）。
    pub total: u64,
    /// 可用内存（字节）。
    pub available: u64,
    /// 已用内存（字节）。
    pub used: u64,
    /// 已用百分比（0~100）。
    pub percent: f64,
}

/// 磁盘使用情况（Go `disk.Usage("/")` 的投影）。
#[derive(Debug, Serialize)]
pub struct DiskDetail {
    /// 总容量（字节）。
    pub total: u64,
    /// 剩余空间（字节）。
    pub free: u64,
    /// 已用百分比（0~100）。
    pub percent: f64,
}

/// 当前进程状态（Go `process.Process` 的投影）。
#[derive(Debug, Serialize)]
pub struct RunningProcessDetail {
    /// 进程号。
    pub pid: u32,
    /// 进程状态，Go 固定填 `running`。
    pub status: String,
    /// 进程常驻内存（字节）。
    pub memory_used: u64,
    /// 进程 CPU 使用率（0~100）。
    pub cpu_percent: f64,
    /// 进程启动时刻（Unix 秒）。
    pub start_time: u64,
}

/// `GET /api/status` 的响应体。
#[derive(Debug, Serialize)]
pub struct SystemStatus {
    /// 整机 CPU 使用率（0~100）。
    pub cpu_percent: f64,
    /// 内存使用情况。
    pub memory: MemoryDetail,
    /// 磁盘使用情况。
    pub disk: DiskDetail,
    /// 系统启动时刻（Unix 秒）。
    pub boot_time: u64,
    /// 当前进程。
    pub process: RunningProcessDetail,
    /// 操作系统（`std::env::consts::OS`）。Rust 扩展字段，Go 无。
    pub os: &'static str,
    /// CPU 架构（`std::env::consts::ARCH`）。Rust 扩展字段，Go 无。
    pub arch: &'static str,
    /// 可执行文件（去扩展名，Go `sys.GetExecutableName`）。Rust 扩展字段，Go 无。
    pub executable: Option<String>,
    /// 本机内网 IPv4（Go `sys.GetLocalIP`）。Rust 扩展字段，Go 无。
    pub local_ip: Option<String>,
}

impl SystemStatus {
    /// 采集一次状态；除 pid / status 与 [`SystemInfo`] 的字段外均为未采集占位值（见模块文档）。
    #[must_use]
    pub fn current() -> Self {
        let info = SystemInfo::collect();
        Self {
            cpu_percent: 0.0,
            memory: MemoryDetail {
                total: 0,
                available: 0,
                used: 0,
                percent: 0.0,
            },
            disk: DiskDetail {
                total: 0,
                free: 0,
                percent: 0.0,
            },
            boot_time: 0,
            process: RunningProcessDetail {
                pid: info.pid,
                status: "running".to_owned(),
                memory_used: 0,
                cpu_percent: 0.0,
                start_time: 0,
            },
            os: info.os,
            arch: info.arch,
            executable: info.executable.map(|path| path.display().to_string()),
            local_ip: info.local_ip.map(|ip| ip.to_string()),
        }
    }
}

/// `GET /api/status`：未登录 401，登录后回 [`SystemStatus`]。
pub(crate) async fn system_status(
    State(state): State<Arc<WebUiState>>,
    headers: HeaderMap,
) -> Response {
    if !is_authenticated(&state, &headers) {
        return unauthorized();
    }
    Json(SystemStatus::current()).into_response()
}

#[cfg(test)]
mod tests {
    use super::SystemStatus;

    #[test]
    fn status_projects_system_info_into_rust_extension_fields() {
        let status = SystemStatus::current();
        assert_eq!(status.os, std::env::consts::OS);
        assert_eq!(status.arch, std::env::consts::ARCH);
        assert_eq!(status.process.pid, std::process::id());
        assert_eq!(status.process.status, "running");
        // Go 的那 5 个字段仍然齐全（形状不变，只是多了扩展键）。
        let json = serde_json::to_value(&status).expect("序列化 status");
        for key in ["cpu_percent", "memory", "disk", "boot_time", "process"] {
            assert!(json.get(key).is_some(), "缺字段 {key}");
        }
        for key in ["os", "arch", "executable", "local_ip"] {
            assert!(json.get(key).is_some(), "缺扩展字段 {key}");
        }
    }
}

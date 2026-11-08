//! `GET /api/status`：机器与当前进程状态（Go `webui/monitor_real.go:handleSysInfo` 的字段投影）。
//!
//! 字段与 Go 响应逐字段同名同型（前端 `SystemStatus` 直接消费，字段缺失会读到 `undefined`，
//! 因此形状必须完整）。**取值来源**：Go 用 gopsutil 采 CPU / 内存 / 磁盘 / 系统启动时间，
//! 本 crate 不引系统信息依赖，P5 reporter 的采集源也尚未对外提供，所以除
//! `process.pid`（`std::process::id()`）与 `process.status`（恒为 `running`）外一律返回 `0`。
//! `0` 在这里表示"未采集"，不是真实读数；接入采集源时只改 [`SystemStatus::current`]，
//! 路由与响应形状不变。

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

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
}

impl SystemStatus {
    /// 采集一次状态；除 pid 与 status 外均为未采集占位值（见模块文档）。
    #[must_use]
    pub fn current() -> Self {
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
                pid: std::process::id(),
                status: "running".to_owned(),
                memory_used: 0,
                cpu_percent: 0.0,
                start_time: 0,
            },
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

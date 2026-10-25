//! 集成测试共用的小工具：临时配置、`AppState`、裸 HTTP 请求。
//!
//! 放在 `tests/support/mod.rs`（子目录模块），不会各自变成独立的测试 target。

#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::Arc;

use gensokyo_config::ConfigManager;
use gensokyo_protocol::Registry;
use gensokyo_runtime::AppState;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// 写一份临时 `config.yml` 并加载；返回 (配置管理器, 临时目录)。
///
/// 临时目录必须由调用方持有——drop 之后配置文件消失，热重载会失败。
pub fn config(settings: &str) -> (Arc<ConfigManager>, TempDir) {
    let dir = tempfile::tempdir().expect("临时目录");
    let path = dir.path().join("config.yml");
    std::fs::write(&path, format!("version: 1\nsettings:\n{settings}")).expect("写测试配置失败");
    let manager = Arc::new(ConfigManager::new(&path).expect("加载配置失败"));
    (manager, dir)
}

/// 构造只注册内置 action 的 `AppState`。
pub fn state(settings: &str) -> (AppState, TempDir) {
    let (manager, dir) = config(settings);
    let mut registry = Registry::new();
    gensokyo_runtime::builtin::register_all(&mut registry, Arc::clone(&manager));
    (AppState::new(manager, Arc::new(registry)), dir)
}

/// 裸 HTTP `POST`（不走 reqwest：测试不需要额外依赖）。
pub async fn post_json(addr: SocketAddr, path: &str, body: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(addr).await.expect("连接测试服务失败");
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .await
        .expect("写请求失败");
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.expect("读响应失败");
    let text = String::from_utf8_lossy(&raw).into_owned();
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or_default();
    (status, text)
}

/// 取响应 body（`\r\n\r\n` 之后的部分）。
pub fn response_body(response: &str) -> &str {
    response.split_once("\r\n\r\n").map_or("", |(_, body)| body)
}

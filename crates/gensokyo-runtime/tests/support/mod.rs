//! 集成测试共用的小工具：临时配置、`AppState`、裸 HTTP 请求。
//!
//! 放在 `tests/support/mod.rs`（子目录模块），不会各自变成独立的测试 target。

#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use gensokyo_config::ConfigManager;
use gensokyo_core::Ob11Id;
use gensokyo_protocol::action::{ActionRequest, ActionResponse};
use gensokyo_protocol::event::{MessageEvent, MessageType};
use gensokyo_protocol::{Event, Handler, HandlerFuture, MessageContent};
use gensokyo_runtime::AppState;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// 写一份临时 `config.yml` 并加载；返回 (配置管理器, 临时目录)。
///
/// 坑（#55 移交钉桩）：`settings` 串必须**两格缩进**（`"  app_id: 12345\n"`）——本函数拼的是 `version: 1\nsettings:\n{settings}`，不缩进的顶层键落在 `settings:` 之外，加载后得到空 settings 而不报错；历史用例 `state("app_id: 12345\n")` 即此形态（依赖项一律走默认值），改成缩进会改变加载结果、属行为变更，故保留原样。
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
    (AppState::with_builtin_handlers(manager), dir)
}

/// 在 `dir` 里开一份临时 idmap 与 botstats 并注入状态。
///
/// `Runtime::start*` 的 `attach_idmap` / `attach_botstats` 在状态没有对应句柄时按默认路径
/// （工作目录）建库；集成测试必须先注入，否则 `cargo test` 会在 crate 目录留下
/// `openid-map.redb` / `msgid-map.redb` / `bot-stats.redb`。
pub fn with_temp_stores(state: AppState, dir: &TempDir) -> AppState {
    let idmap = gensokyo_storage::IdMapStore::open(
        gensokyo_storage::IdMapOptions::new(
            dir.path().join("openid-map.redb"),
            dir.path().join("msgid-map.redb"),
        )
        .with_app_id("102000001"),
    )
    .expect("打开临时 idmap 失败");
    let botstats = gensokyo_storage::BotStatsStore::open(
        dir.path().join(gensokyo_storage::DEFAULT_BOTSTATS_PATH),
    )
    .expect("打开临时 botstats 失败");
    state
        .with_idmap(Arc::new(idmap))
        .with_botstats(Arc::new(botstats))
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

/// 裸 HTTP `GET`；返回 (状态码, 完整响应文本)。
pub async fn get(addr: SocketAddr, path: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(addr).await.expect("连接测试服务失败");
    let request = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
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

/// 一条可被 hub 广播的群消息事件（正向 / 反向 WS 用例共用）。
pub fn hub_event() -> Event {
    Event::Message(MessageEvent {
        time: 1_700_000_000,
        self_id: Ob11Id::num(12345),
        message_type: MessageType::Group,
        sub_type: Some("normal".to_owned()),
        message_id: Ob11Id::str("M1"),
        user_id: Ob11Id::str("U1"),
        group_id: Some(Ob11Id::str("G1")),
        message: MessageContent::Text("hub".to_owned()),
        raw_message: "hub".to_owned(),
        font: Some(0),
        sender: None,
        anonymous: None,
        message_seq: None,
        to_me: Some(true),
        extra: Default::default(),
    })
}

/// 慢 action（300ms）：验证分发在独立任务里跑，事件推送不等它结束。
#[derive(Debug)]
pub struct Slow;

impl Handler for Slow {
    fn name(&self) -> &'static str {
        "slow"
    }

    fn handle<'a>(&'a self, request: &'a ActionRequest<Value>) -> HandlerFuture<'a> {
        Box::pin(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            Ok(ActionResponse::ok(json!({ "slept": true })).with_echo(request.echo.clone()))
        })
    }
}

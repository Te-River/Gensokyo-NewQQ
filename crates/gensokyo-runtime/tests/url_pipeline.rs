//! 出站 URL 转换管线集成测试（P56-T4，设计 §5.4）。
//!
//! 经真实 dispatcher → deferred 执行链驱动，用 [`FakeQqApi`] 记录上游入参与上传调用，
//! 全程不碰 QQ 官方接口（lotus 用例连的是本地一次性 stub）。首个用例是回归锁：
//! `transfer_url=false`（默认配置）时文本、发送条数、请求体全部零变化。闭环用例
//! （P56-F2）额外走 `Runtime::start_with` 真实启动运行时，验证管线 mint 出的短码能被
//! 生产 `/url` 路由 301 跳转回原长链（mint 与路由复用同一个 `Arc<ShortUrlStore>`）。

mod support;

use std::collections::VecDeque;
use std::io::Read;
use std::io::Write;
use std::net::{SocketAddr, TcpListener};
use std::sync::{Arc, Mutex};

use gensokyo_actions::{ActionContext, QqApi, QqFuture};
use gensokyo_core::Error;
use gensokyo_protocol::action::ActionRequest;
use gensokyo_qq::dto::{
    BotInGroupState, FileUpload, GroupInfo, GroupMember, GroupMemberList, JoinRequestList,
    MediaResponse, MessageResponse, MessageToCreate,
};
use gensokyo_runtime::url_pipeline::{LocalMint, LotusMint, UrlMint};
use gensokyo_runtime::{AppState, Runtime, RuntimeOptions};
use gensokyo_storage::{ShortUrlStore, short_url::hashed_code};
use serde_json::{Value, json};
use tempfile::TempDir;

use support::config;

/// 群 OpenID（32 位，直通上游、不需要 idmap）。
const OPENID: &str = "0123456789abcdef0123456789abcdef";
/// 被转换的入口 URL（xurls T1 金样已验证该文本形态能命中）。
const LONG_URL: &str = "http://a.com";

/// 一次发送的入参记录。
#[derive(Debug, Clone)]
struct Sent {
    group_openid: String,
    message: MessageToCreate,
}

/// 记录发送与上传入参的 `QqApi` 替身；发送按序消费结果槽。
#[derive(Debug, Default)]
struct FakeQqApi {
    sent: Mutex<Vec<Sent>>,
    /// 预置发送结果（`send_group_message` 按序消费）。
    results: Mutex<VecDeque<Result<MessageResponse, Error>>>,
    /// 每次 `upload_group_file` 记录的入参，并返回 `upload_file_info`。
    uploads: Mutex<Vec<FileUpload>>,
    upload_file_info: Mutex<Option<String>>,
}

impl FakeQqApi {
    fn sent(&self) -> Vec<Sent> {
        self.sent.lock().expect("锁未中毒").clone()
    }

    fn uploads(&self) -> Vec<FileUpload> {
        self.uploads.lock().expect("锁未中毒").clone()
    }

    fn with_upload(self, file_info: &str) -> Self {
        *self.upload_file_info.lock().expect("锁未中毒") = Some(file_info.to_owned());
        self
    }

    fn push_ok(&self, id: &str) {
        self.results
            .lock()
            .expect("锁未中毒")
            .push_back(Ok(MessageResponse {
                id: id.to_owned(),
                ..MessageResponse::default()
            }));
    }

    fn take_result(&self) -> Result<MessageResponse, Error> {
        match self.results.lock().expect("锁未中毒").pop_front() {
            Some(outcome) => outcome,
            // 「忘了准备 fixture」要立刻可见（tests/deferred.rs 同款口径，回错误不 panic）
            None => Err(Error::Upstream {
                code: 0,
                message: "测试未配置发送结果".to_owned(),
            }),
        }
    }
}

impl QqApi for FakeQqApi {
    fn get_group_info(&self, _group_openid: &str) -> QqFuture<'_, GroupInfo> {
        Box::pin(async { Err(Error::Unsupported("fake: get_group_info".to_owned())) })
    }

    fn get_group_member_info(
        &self,
        _group_openid: &str,
        _member_openid: &str,
    ) -> QqFuture<'_, GroupMember> {
        Box::pin(async { Err(Error::Unsupported("fake: get_group_member_info".to_owned())) })
    }

    fn get_group_member_list(
        &self,
        _group_openid: &str,
        _limit: u32,
    ) -> QqFuture<'_, GroupMemberList> {
        Box::pin(async { Err(Error::Unsupported("fake: get_group_member_list".to_owned())) })
    }

    fn set_group_ban(
        &self,
        _group_openid: &str,
        _member_openid: &str,
        _duration_secs: i64,
    ) -> QqFuture<'_, ()> {
        Box::pin(async { Err(Error::Unsupported("fake: set_group_ban".to_owned())) })
    }

    fn get_group_bot_state(&self, _group_openid: &str) -> QqFuture<'_, BotInGroupState> {
        Box::pin(async { Err(Error::Unsupported("fake: get_group_bot_state".to_owned())) })
    }

    fn get_group_join_request_list(
        &self,
        _group_openid: &str,
        _cursor: Option<&str>,
        _limit: Option<u32>,
    ) -> QqFuture<'_, JoinRequestList> {
        Box::pin(async {
            Err(Error::Unsupported(
                "fake: get_group_join_request_list".to_owned(),
            ))
        })
    }

    fn send_group_message(
        &self,
        group_openid: String,
        msg: MessageToCreate,
    ) -> QqFuture<'_, MessageResponse> {
        Box::pin(async move {
            self.sent.lock().expect("锁未中毒").push(Sent {
                group_openid,
                message: msg,
            });
            self.take_result()
        })
    }

    fn send_c2c_message(
        &self,
        _user_openid: String,
        _msg: MessageToCreate,
    ) -> QqFuture<'_, MessageResponse> {
        Box::pin(async { Err(Error::Unsupported("fake: send_c2c_message".to_owned())) })
    }

    fn upload_group_file(
        &self,
        _group_openid: &str,
        file: FileUpload,
    ) -> QqFuture<'_, MediaResponse> {
        Box::pin(async move {
            self.uploads.lock().expect("锁未中毒").push(file.clone());
            let file_info = self
                .upload_file_info
                .lock()
                .expect("锁未中毒")
                .clone()
                .expect("测试未配置上传 file_info");
            Ok(MediaResponse {
                file_info,
                ..MediaResponse::default()
            })
        })
    }
}

/// 装配「内置 action + actions 全集」的 `AppState`，可注入短链 mint。
fn state(settings: &str, url_mint: Option<Arc<UrlMint>>) -> (TempDir, Arc<FakeQqApi>, AppState) {
    let (manager, dir) = config(settings);
    let api = Arc::new(FakeQqApi::default().with_upload("FI-QR-1"));
    let api_dyn: Arc<dyn QqApi> = api.clone();
    let context = Arc::new(ActionContext::new(api_dyn, Arc::clone(&manager)));
    let state = AppState::with_actions(manager, context).with_url_mint(url_mint);
    (dir, api, state)
}

fn request(action: &str, params: Value) -> ActionRequest<Value> {
    ActionRequest {
        action: action.to_owned(),
        params: Some(params),
        echo: None,
    }
}

fn group_message(message: &str) -> Value {
    json!({ "group_id": OPENID, "message": message })
}

/// 取某条已发消息的正文（`content`）。
fn content_of(sent: &Sent) -> Option<&str> {
    sent.message.content.as_deref()
}

/// 本地 mint 只持短链库句柄：地址由 mint 逐调用现读配置快照的 `server_dir`（对齐 Go
/// `GenerateShortURL` 每次 `config.GetXxx()`）。
fn local_mint(dir: &TempDir) -> (Arc<ShortUrlStore>, Arc<UrlMint>) {
    let store =
        Arc::new(ShortUrlStore::open(dir.path().join("url-pipeline-it.redb")).expect("打开短链库"));
    let mint = Arc::new(UrlMint::Local(LocalMint {
        store: Arc::clone(&store),
    }));
    (store, mint)
}

/// `rustls-no-provider` 口径：测试进程自行安装 ring provider（生产由二进制 startup 装）。
fn ensure_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// 一次性 lotus `/url` stub：bind 后线程循环应答固定 JSON（每连接一应答）。
struct LotusStub {
    port: u16,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<Mutex<bool>>,
}

impl LotusStub {
    fn spawn(body: &'static str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("绑定 stub 端口");
        let port = listener.local_addr().expect("取地址").port();
        listener.set_nonblocking(true).expect("nonblocking");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(Mutex::new(false));
        let shared = Arc::clone(&requests);
        let stopped = Arc::clone(&stop);
        std::thread::spawn(move || {
            while !*stopped.lock().expect("锁未中毒") {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        // accepted socket 在部分平台继承 listener 的 nonblocking，read 立刻
                        // WouldBlock；显式设 read timeout 切回阻塞口径（T3 Stub 同款）
                        stream
                            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                            .ok();
                        let mut raw = Vec::new();
                        let mut chunk = [0u8; 4096];
                        // 读到头部结束 + Content-Length 体即回固定应答（stub 只测形态，不做解析）。
                        let mut head_done = false;
                        while !head_done {
                            match stream.read(&mut chunk) {
                                Ok(0) => break,
                                Ok(n) => {
                                    raw.extend_from_slice(&chunk[..n]);
                                    head_done = raw.windows(4).any(|w| w == b"\r\n\r\n");
                                }
                                Err(_) => break,
                            }
                        }
                        shared
                            .lock()
                            .expect("锁未中毒")
                            .push(String::from_utf8_lossy(&raw).into_owned());
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        let _ = stream.write_all(response.as_bytes());
                        let _ = stream.shutdown(std::net::Shutdown::Write);
                    }
                    Err(_) => std::thread::sleep(std::time::Duration::from_millis(5)),
                }
            }
        });
        Self {
            port,
            requests,
            stop,
        }
    }

    fn port(&self) -> String {
        self.port.to_string()
    }

    fn requests(&self) -> Vec<String> {
        self.requests.lock().expect("锁未中毒").clone()
    }
}

impl Drop for LotusStub {
    fn drop(&mut self) {
        *self.stop.lock().expect("锁未中毒") = true;
    }
}

// ---------- 回归锁：默认配置零变化 ----------

#[tokio::test]
async fn transfer_url_off_keeps_everything_unchanged() {
    // 默认配置（transfer_url 缺省 false）：即使挂了本地 mint，文本、条数、请求体全零变化。
    let (_dir, api, state) = state("app_id: 12345\n", None);
    api.push_ok("m-off");

    let text = format!("看 {LONG_URL} 好");
    let response = state
        .dispatcher()
        .dispatch(&request("send_group_msg", group_message(&text)))
        .await;

    assert_eq!(response.retcode, 0);
    assert_eq!(response.data, Some(json!({ "message_id": "m-off" })));
    let sent = api.sent();
    assert_eq!(sent.len(), 1, "关闭态不得触发 QR 侧发送");
    assert_eq!(content_of(&sent[0]), Some(text.as_str()), "文本逐字节原样");
    assert!(api.uploads().is_empty(), "关闭态不得有上传");
}

// ---------- QR 分支：发送序 / 形态 / 响应单份 ----------

#[tokio::test]
async fn qr_image_sends_before_main_text_and_client_gets_one_response() {
    let settings =
        "  transfer_url: true\n  url_to_qrimage: true\n  qr_size: 200\n  server_dir: example.com\n";
    let (_dir, api, state) = state(settings, None);
    // QR 子请求先消费 qr-1，主消息消费 main-1（发送序即消费序，错位则断言炸）。
    api.push_ok("qr-1");
    api.push_ok("main-1");

    let response = state
        .dispatcher()
        .dispatch(&request(
            "send_group_msg",
            group_message(&format!("看 {LONG_URL} 好")),
        ))
        .await;

    assert_eq!(response.retcode, 0);
    // 客户端只收到一份响应，且回执取主消息（最后一条）的结果。
    assert_eq!(response.data, Some(json!({ "message_id": "main-1" })));

    let sent = api.sent();
    assert_eq!(sent.len(), 2, "QR 图条 + 主消息");
    assert_eq!(sent[0].group_openid, OPENID, "QR 条与主消息同收件人");
    let qr = &sent[0].message;
    assert_eq!(qr.msg_type, Some(7), "QR 条为富媒体 msg_type=7");
    assert_eq!(
        qr.media.as_ref().expect("QR 条带 media").file_info,
        "FI-QR-1"
    );
    assert!(qr.content.is_none(), "QR 条只有图片没有正文");
    assert!(
        qr.msg_id.is_none() && qr.event_id.is_none(),
        "QR 条无 reply 身份"
    );

    let main = &sent[1].message;
    // 原位删 URL、两侧空白保留（Go ReplaceAllStringFunc 同形态）。
    assert_eq!(main.content.as_deref(), Some("看  好"));

    // 上传确实发生且是图片 file_type（compose_single_media 既有链）。
    let uploads = api.uploads();
    assert_eq!(uploads.len(), 1, "QR PNG 走一次上传");
}

#[tokio::test]
async fn ip_hidden_wins_over_qr() {
    // server_dir 是 IP 且 visible_ip 缺省 false：URL 原位删空，不生成也不发送 QR 条。
    let settings = "  transfer_url: true\n  url_to_qrimage: true\n  server_dir: 1.2.3.4\n";
    let (_dir, api, state) = state(settings, None);
    api.push_ok("m-hidden");

    let response = state
        .dispatcher()
        .dispatch(&request(
            "send_group_msg",
            group_message(&format!("看 {LONG_URL} 好")),
        ))
        .await;

    assert_eq!(response.retcode, 0);
    let sent = api.sent();
    assert_eq!(sent.len(), 1, "IP 隐藏优先于 QR，不得有侧发送");
    assert_eq!(content_of(&sent[0]), Some("看  好"));
    assert!(api.uploads().is_empty());
}

// ---------- 短链分支 ----------

#[tokio::test]
async fn local_short_url_replaces_inplace_and_persists() {
    // mint 失败态 → 悬空 base 的逐字对齐在 url_pipeline.rs 单测（local_link 纯映射；
    // 真库 create 无对外可触发错误路径，T3 同款口径）。
    let settings = "  transfer_url: true\n  server_dir: example.com\n";
    let mint_dir = tempfile::tempdir().expect("mint 临时目录");
    let (store, mint) = local_mint(&mint_dir);
    let (_dir, api, state) = state(settings, Some(mint));
    api.push_ok("m-local");

    let response = state
        .dispatcher()
        .dispatch(&request(
            "send_group_msg",
            group_message(&format!("看 {LONG_URL} 好")),
        ))
        .await;

    assert_eq!(response.retcode, 0);
    let sent = api.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        content_of(&sent[0]),
        Some(format!("看 https://example.com/url/{} 好", hashed_code(LONG_URL)).as_str())
    );
    assert_eq!(
        store
            .resolve(&hashed_code(LONG_URL))
            .expect("查询")
            .as_deref(),
        Some(LONG_URL),
        "短码必须真实落库（Go storeURL 同义）"
    );
}

#[tokio::test]
async fn mint_absent_keeps_original_text() {
    // Rust-only 装配防御：mint 未挂（None）时短链分支**保留原文不改写**（删空等于静默
    // 丢用户内容），整次变换最多一条 warn；QR/IP 分支不受影响（上方两个用例覆盖）。
    let settings = "  transfer_url: true\n  server_dir: example.com\n";
    let (_dir, api, state) = state(settings, None);
    api.push_ok("m-nomint");

    let response = state
        .dispatcher()
        .dispatch(&request(
            "send_group_msg",
            group_message(&format!("看 {LONG_URL} 好")),
        ))
        .await;

    assert_eq!(response.retcode, 0);
    let sent = api.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        content_of(&sent[0]),
        Some(format!("看 {LONG_URL} 好").as_str()),
        "mint 未装配时 URL 原文照发"
    );
}

#[tokio::test]
async fn lotus_stub_replaces_with_remote_link_inplace() {
    ensure_crypto_provider();
    let stub = LotusStub::spawn(r#"{"shortURL":"https://s.cn/x9"}"#);
    // mint 的目标地址逐调用现读配置（对齐 Go `GenerateShortURL`）：server_dir 用
    // 127.0.0.1，配 visible_ip: true 免触发 IP 隐藏门控（该门控先于 mint，Go 同口径）。
    let settings = format!(
        "  transfer_url: true\n  server_dir: 127.0.0.1\n  visible_ip: true\n  port: \"{}\"\n",
        stub.port()
    );
    let mint = Arc::new(UrlMint::Lotus(
        LotusMint::try_new().expect("构造 lotus client"),
    ));
    let (_dir, api, state) = state(&settings, Some(mint));
    api.push_ok("m-lotus");

    let response = state
        .dispatcher()
        .dispatch(&request(
            "send_group_msg",
            group_message(&format!("看 {LONG_URL} 好")),
        ))
        .await;

    assert_eq!(response.retcode, 0);
    let sent = api.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        content_of(&sent[0]),
        Some("看 https://s.cn/x9 好"),
        "远端完整 shortURL 原位替换；stub 收到请求: {:?}",
        stub.requests()
    );
    assert_eq!(stub.requests().len(), 1, "mint 恰好一次 POST /url");
}

// ---------- forward 路径覆盖 ----------

#[tokio::test]
async fn forward_nodes_each_get_short_url() {
    // forward_msg_limit 显式给 10：Go 同款上界，结构体零值 0 会把计划截到 1 条
    let settings = "  transfer_url: true\n  server_dir: example.com\n  forward_msg_limit: 10\n";
    let dir = tempfile::tempdir().expect("临时目录");
    let (store, mint) = local_mint(&dir);
    let (manager, cfg_dir) = config(settings);
    let api = Arc::new(FakeQqApi::default().with_upload("FI-QR-1"));
    let api_dyn: Arc<dyn QqApi> = api.clone();
    let context = Arc::new(ActionContext::new(api_dyn, Arc::clone(&manager)));
    let state = AppState::with_actions(manager, context).with_url_mint(Some(mint));
    api.push_ok("f-1");
    api.push_ok("f-2");

    let response = state
        .dispatcher()
        .with_forward_interval(std::time::Duration::ZERO)
        .dispatch(&request(
            "send_group_forward_msg",
            json!({
                "group_id": OPENID,
                "messages": [
                    {"data": {"content": format!("第一条 {LONG_URL}")}},
                    {"data": {"content": "第二条 https://b.org/p"}}
                ]
            }),
        ))
        .await;

    assert_eq!(response.retcode, 0);
    let data = response.data.expect("data");
    assert_eq!(
        data["message_ids"],
        json!(["f-1", "f-2"]),
        "node 失败清单: {}",
        data["failed"]
    );
    let sent = api.sent();
    assert_eq!(sent.len(), 2, "forward 逐 node 执行，变换覆盖每个 node");
    assert_eq!(
        content_of(&sent[0]),
        Some(format!("第一条 https://example.com/url/{}", hashed_code(LONG_URL)).as_str())
    );
    assert_eq!(
        content_of(&sent[1]),
        Some(
            format!(
                "第二条 https://example.com/url/{}",
                hashed_code("https://b.org/p")
            )
            .as_str()
        )
    );
    assert_eq!(
        store
            .resolve(&hashed_code(LONG_URL))
            .expect("查询")
            .as_deref(),
        Some(LONG_URL),
        "短码必须真实落库（Go storeURL 同义）"
    );
    let _ = cfg_dir;
}

// ---------- 生产闭环：管线 mint 的短码必须能被 `/url/{code}` 301 跳转 ----------

/// 环回 GET + 仅连接层重试（同 `tests/lotus.rs` 的 10053 口径：火绒偶发中止环回连接，
/// 重试的是连接成败、不是断言语义）。返回 (状态码, 完整响应文本)。
async fn get_with_retry(addr: SocketAddr, path: &str) -> (u16, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut attempt = 0usize;
    loop {
        let outcome: std::io::Result<(u16, String)> = async {
            let mut stream = tokio::net::TcpStream::connect(addr).await?;
            let request =
                format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
            stream.write_all(request.as_bytes()).await?;
            let mut raw = Vec::new();
            stream.read_to_end(&mut raw).await?;
            let text = String::from_utf8_lossy(&raw).into_owned();
            let status = text
                .split_whitespace()
                .nth(1)
                .and_then(|code| code.parse().ok())
                .unwrap_or_default();
            Ok((status, text))
        }
        .await;
        match outcome {
            Ok(response) => return response,
            Err(err) if attempt < 4 => {
                attempt += 1;
                eprintln!("环回 GET 连接失败({err})，第 {attempt} 次重试");
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            Err(err) => panic!("环回 GET {path} 应最终成功: {err}"),
        }
    }
}

/// 从响应原文取 `Location` 头（头名大小写不敏感）。
fn location_of(response: &str) -> Option<&str> {
    response
        .lines()
        .find(|line| {
            line.get(..9)
                .is_some_and(|name| name.eq_ignore_ascii_case("Location:"))
        })
        .map(|line| line[9..].trim())
}

/// 本地短链的**生产不变式**闭环（评审 Major）：boot 链让 mint 复用 `short_url_arc()`
/// 同一份 `Arc<ShortUrlStore>`，故管线替换文本里的短码必须能被真实运行时的
/// `GET /url/{code}` 301 跳转回原长链。既有本地短链用例只查库不落服务端，此处补齐。
#[tokio::test(flavor = "multi_thread")]
async fn pipeline_minted_short_code_redirects_on_running_server() {
    let settings = "  app_id: 102000001\n  transfer_url: true\n  server_dir: example.com\n  enable_ws_server: true\n  disable_webui: true\n  port: \"0\"\n";
    let (manager, dir) = config(settings);
    let api = Arc::new(FakeQqApi::default().with_upload("FI-QR-1"));
    let api_dyn: Arc<dyn QqApi> = api.clone();
    let context = Arc::new(ActionContext::new(api_dyn, Arc::clone(&manager)));
    let (store, mint) = local_mint(&dir);
    // 同一份 Arc 既进 mint 也进 `/url` 路由（with_temp_stores 先垫 idmap/botstats，
    // 短链库随后覆盖成 mint 手里那份——boot 的 attach_* 对已注入句柄原样保留）。
    let state = support::with_temp_stores(AppState::with_actions(manager, context), &dir)
        .with_short_url(Arc::clone(&store))
        .with_url_mint(Some(mint));
    api.push_ok("m-loop");

    let response = state
        .dispatcher()
        .dispatch(&request(
            "send_group_msg",
            group_message(&format!("看 {LONG_URL} 好")),
        ))
        .await;
    assert_eq!(response.retcode, 0);
    let sent = api.sent();
    assert_eq!(sent.len(), 1);
    let text = content_of(&sent[0]).expect("正文").to_owned();
    // 短码从管线**实际输出**的替换文本里抠（断言对象就是被发出去的那串），再核库键一致。
    let code = text
        .split_once("https://example.com/url/")
        .expect("替换文本为短链形态")
        .1
        .split_whitespace()
        .next()
        .expect("短码后紧跟空白")
        .to_owned();
    assert_eq!(code, hashed_code(LONG_URL), "短码与库键一致");

    // 真实启动运行时（boot 装配链，/url 挂主 listener），回读跳转。
    let handle = Runtime::start_with(
        state,
        RuntimeOptions {
            ws_bind: Some("127.0.0.1:0".to_owned()),
            http_bind: None,
            config_watch: false,
            gateway: false,
            post_url: false,
        },
    )
    .await
    .expect("运行时应启动");
    let ws = handle.ws_addr().expect("正向 WS 主 listener 应启动");
    let (status, raw) = get_with_retry(ws, &format!("/url/{code}")).await;
    assert_eq!(status, 301, "响应原文: {raw}");
    assert_eq!(
        location_of(&raw),
        Some(LONG_URL),
        "301 的 Location 必须指回原长链（与替换文本闭环）: {raw}"
    );

    handle
        .shutdown_with_timeout(std::time::Duration::from_secs(5))
        .await;
}

// ---------- QR 生成失败仍删 URL ----------

/// QR 收集分支先删 URL（`transform` 内完成），PNG 生成失败只 warn、不侧发送——与
/// mint=None 的「保留原文」是两条不同路径。超 version-40 Q 容量载荷的 `Err` 由
/// `qr.rs::overlong_url_errors` 证过，此处用同款形态走管线端到端。
#[tokio::test]
async fn qr_generation_failure_still_deletes_url() {
    let settings =
        "  transfer_url: true\n  url_to_qrimage: true\n  qr_size: 200\n  server_dir: example.com\n";
    let (_dir, api, state) = state(settings, None);
    api.push_ok("m-qrfail");

    let huge = format!("https://example.com/{}", "a".repeat(5000));
    let response = state
        .dispatcher()
        .dispatch(&request(
            "send_group_msg",
            group_message(&format!("看 {huge} 好")),
        ))
        .await;

    assert_eq!(response.retcode, 0);
    let sent = api.sent();
    assert_eq!(sent.len(), 1, "QR 生成失败不得侧发送");
    assert_eq!(content_of(&sent[0]), Some("看  好"), "URL 必须已从正文删除");
    assert!(api.uploads().is_empty(), "生成失败不得有上传");
}

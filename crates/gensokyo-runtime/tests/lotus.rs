//! lotus boot 装配（P55-P4）：gRPC 服务端方向、远程客户端挂载、HTTP 回退桥挂载、
//! `/getid` 三态、`backup_port` 切换与双实例 idmap 一致性。
//!
//! 全部用例注入临时库（`support::with_temp_stores`），不在 cwd 留 redb 文件；
//! `disable_webui: true` 关掉 SPA 静态兜底，未挂载的路由才回 404 而不是首页。
//! `/getid` 判定延续 boot 的「看实际状态」口径：服务端真起来或远程客户端真挂上才隐藏，
//! 降级时保留（Go `main.go:507` 是 `lotus_grpc=true` 一律不挂，差异已在 boot 注释登记）。

mod support;

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gensokyo_runtime::{Runtime, RuntimeOptions};
use tempfile::TempDir;

/// 测试用启动选项：随机本地端口、不拉网关 / 热重载 / 上报。
fn options() -> RuntimeOptions {
    RuntimeOptions {
        ws_bind: Some("127.0.0.1:0".to_owned()),
        http_bind: None,
        config_watch: false,
        gateway: false,
        post_url: false,
    }
}

fn state_with_stores(settings: &str) -> (gensokyo_runtime::AppState, TempDir) {
    let (manager, dir) = support::config(settings);
    let state = support::with_temp_stores(
        gensokyo_runtime::AppState::with_builtin_handlers(manager),
        &dir,
    );
    (state, dir)
}

/// 服务端实例（`lotus=false, lotus_grpc=true`）：gRPC 服务端真起来 → `/getid` 隐藏。
#[tokio::test(flavor = "multi_thread")]
async fn grpc_server_direction_hides_getid() {
    let (state, _dir) = state_with_stores(
        "  app_id: 102000001\n  enable_ws_server: true\n  disable_webui: true\n  port: \"0\"\n  lotus_grpc: true\n  lotus_grpc_port: 0\n",
    );
    let handle = Runtime::start_with(state, options())
        .await
        .expect("服务端实例应启动");
    assert!(
        handle.idmap_grpc_addr().is_some(),
        "lotus=false && lotus_grpc=true 应起 gRPC 服务端"
    );
    let ws = handle.ws_addr().expect("正向 WS 应启动");
    let (status, _) = support::get(ws, "/getid?type=1&id=openid-x").await;
    assert_eq!(status, 404, "服务端接管后 /getid 应隐藏");
    handle.shutdown_with_timeout(Duration::from_secs(5)).await;
}

/// 降级态：gRPC 远端不可达 → 客户端建连失败降级本地 → `/getid` 保持挂载。
#[tokio::test(flavor = "multi_thread")]
async fn degraded_remote_keeps_getid_mounted() {
    // 先占端口再释放，保证「连接被拒」而不是被防火墙静默丢包。
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("占端口");
    let closed_port = listener.local_addr().expect("本地地址").port();
    drop(listener);

    let (state, _dir) = state_with_stores(&format!(
        "  app_id: 102000001\n  enable_ws_server: true\n  disable_webui: true\n  port: \"0\"\n  lotus: true\n  lotus_grpc: true\n  server_dir: \"127.0.0.1\"\n  lotus_grpc_port: {closed_port}\n"
    ));
    let handle = Runtime::start_with(state, options())
        .await
        .expect("降级不应阻断启动");
    assert!(
        handle.idmap_grpc_addr().is_none(),
        "lotus=true 是客户端方向，不应起服务端"
    );
    let ws = handle.ws_addr().expect("正向 WS 应启动");
    let (status, _) = support::get(ws, "/getid?type=1&id=openid-x").await;
    assert_ne!(status, 404, "降级本地后 /getid 应保持挂载");
    handle.shutdown_with_timeout(Duration::from_secs(5)).await;
}

/// `backup_port` 切换（Go `main.go:488-494`）：lotus=true 时主 listener 用 backup_port。
/// 本用例不能覆盖 `ws_bind`（覆盖会绕过配置端口），否则测不到切换本身。
#[tokio::test(flavor = "multi_thread")]
async fn lotus_main_listener_uses_backup_port() {
    let no_override = RuntimeOptions {
        ws_bind: None,
        ..options()
    };
    // 占住 port：若 boot 仍用 port 绑定会直接失败，启动成功即证明切到了 backup_port。
    let occupied = std::net::TcpListener::bind("127.0.0.1:0").expect("占端口");
    let occupied_port = occupied.local_addr().expect("本地地址").port();

    let (state, _dir) = state_with_stores(&format!(
        "  app_id: 102000001\n  enable_ws_server: true\n  disable_webui: true\n  lotus: true\n  port: \"{occupied_port}\"\n  backup_port: \"0\"\n"
    ));
    let handle = Runtime::start_with(state, no_override.clone())
        .await
        .expect("lotus=true 应改用 backup_port 绑定");
    let ws = handle.ws_addr().expect("正向 WS 应启动");
    assert_ne!(ws.port(), occupied_port, "主 listener 不应再绑 port");
    handle.shutdown_with_timeout(Duration::from_secs(5)).await;

    // 反向：lotus=false 用 port，backup_port 被占也不影响。
    let (state, _dir) = state_with_stores(&format!(
        "  app_id: 102000001\n  enable_ws_server: true\n  disable_webui: true\n  port: \"0\"\n  backup_port: \"{occupied_port}\"\n"
    ));
    let handle = Runtime::start_with(state, no_override.clone())
        .await
        .expect("lotus=false 应继续用 port");
    assert!(handle.ws_addr().is_some());
    handle.shutdown_with_timeout(Duration::from_secs(5)).await;

    // 空 backup_port 与空 port 同口径：不另设校验，原样交给绑定层。
    let (state, _dir) = state_with_stores(
        "  app_id: 102000001\n  enable_ws_server: true\n  disable_webui: true\n  lotus: true\n  port: \"0\"\n  backup_port: \"\"\n",
    );
    let backup_empty = Runtime::start_with(state, no_override.clone()).await;
    let (state, _dir) = state_with_stores(
        "  app_id: 102000001\n  enable_ws_server: true\n  disable_webui: true\n  port: \"\"\n",
    );
    let port_empty = Runtime::start_with(state, no_override).await;
    assert_eq!(
        backup_empty.is_ok(),
        port_empty.is_ok(),
        "空 backup_port 与空 port 的绑定口径必须一致"
    );
    if let Ok(handle) = backup_empty {
        handle.shutdown_with_timeout(Duration::from_secs(5)).await;
    }
    if let Ok(handle) = port_empty {
        handle.shutdown_with_timeout(Duration::from_secs(5)).await;
    }
}

/// 双实例集成：服务端实例（lotus=false, lotus_grpc=true）+ 客户端实例
/// （lotus=true, lotus_grpc=true）同进程不同端口；客户端 store 的写入/查询全部落到
/// 服务端库，虚拟 ID 跨实例一致，客户端本地 redb 一行不写。
#[tokio::test(flavor = "multi_thread")]
async fn dual_instance_idmap_consistency() {
    // 服务端实例：注入临时库，gRPC 服务端监听随机端口。
    let (server_state, _server_dir) = state_with_stores(
        "  app_id: 102000001\n  enable_ws_server: true\n  disable_webui: true\n  port: \"0\"\n  lotus_grpc: true\n  lotus_grpc_port: 0\n",
    );
    let server_store = server_state.idmap_arc().expect("服务端 idmap 已注入");
    let server = Runtime::start_with(server_state, options())
        .await
        .expect("服务端实例应启动");
    let grpc_addr = server.idmap_grpc_addr().expect("gRPC 服务端应启动");

    // 客户端实例：server_dir 指向服务端实际端口；注入的独占 store 由 boot 挂上 gRPC 桥。
    let (client_state, _client_dir) = state_with_stores(&format!(
        "  app_id: 102000001\n  enable_ws_server: true\n  disable_webui: true\n  port: \"0\"\n  lotus: true\n  lotus_grpc: true\n  server_dir: \"127.0.0.1\"\n  lotus_grpc_port: {}\n",
        grpc_addr.port()
    ));
    let client = Runtime::start_with(client_state, options())
        .await
        .expect("客户端实例应启动");
    let client_store = client.idmap_arc().expect("客户端 idmap 应装配");

    // 客户端分配虚拟 ID → 服务端库可见且一致。
    let (raw, row) = client_store
        .retrieve_virtual_value("openid-cross-instance")
        .expect("客户端分配虚拟 ID");
    assert_eq!(raw, "openid-cross-instance");
    assert!(row > 0);
    assert_eq!(
        server_store
            .lookup_virtual_value("openid-cross-instance")
            .expect("服务端应已分配"),
        row,
        "虚拟 ID 必须跨实例一致"
    );
    // 客户端反查同样走远端。
    assert_eq!(
        client_store.retrieve_row_by_id(row).expect("客户端反查"),
        "openid-cross-instance"
    );
    // 消息缓存也走远端（StoreCacheV2 / RetrieveRowByCacheV2）。
    let cache_row = client_store.store_cache("real-msg-cross").expect("缓存写入");
    assert_eq!(
        server_store
            .retrieve_row_by_cache(cache_row)
            .expect("服务端缓存反查"),
        "real-msg-cross"
    );
    // 核心断言：远程全命中后客户端本地 redb 一行未写。
    assert!(client_store.is_empty().expect("本地库检查"));

    // 远程客户端真挂上 → 两个实例的 /getid 都隐藏。
    let client_ws = client.ws_addr().expect("客户端正向 WS 应启动");
    let (status, _) = support::get(client_ws, "/getid?type=1&id=openid-x").await;
    assert_eq!(status, 404, "客户端远程接管后 /getid 应隐藏");
    let server_ws = server.ws_addr().expect("服务端正向 WS 应启动");
    let (status, _) = support::get(server_ws, "/getid?type=1&id=openid-x").await;
    assert_eq!(status, 404, "服务端接管后 /getid 应隐藏");

    client.shutdown_with_timeout(Duration::from_secs(5)).await;
    server.shutdown_with_timeout(Duration::from_secs(5)).await;
}

/// 手写 HTTP stub（同 storage http_remote 测试口径）：专职线程 + std TcpListener，
/// keep-alive 复用连接缩小火绒连接中止暴露面；端口取专用段（20000–32767），
/// 避开本机自定义动态端口范围（1024–15000）与出站源端口游标。
struct HttpStub {
    port: u16,
    targets: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl HttpStub {
    fn spawn() -> Self {
        static NEXT: AtomicU16 = AtomicU16::new(20000);
        let listener = loop {
            let port = NEXT.fetch_add(1, Ordering::Relaxed);
            assert!(port < 32768, "测试端口段耗尽");
            if let Ok(listener) = std::net::TcpListener::bind(("127.0.0.1", port)) {
                break listener;
            }
        };
        let port = listener.local_addr().expect("本地地址").port();
        listener.set_nonblocking(true).expect("设置 nonblocking");
        let targets = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let recorded = Arc::clone(&targets);
        let stopped = Arc::clone(&stop);
        let handle = std::thread::spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => serve_conn(stream, &recorded),
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            port,
            targets,
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for HttpStub {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// 只回本用例需要的 type=7 罐头应答（字段名与 Go 服务端逐字一致），其余回错误 JSON。
fn serve_conn(mut stream: std::net::TcpStream, targets: &Mutex<Vec<String>>) {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    // keep-alive：同一连接顺序处理多个请求（同 storage stub 口径）。
    loop {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        // GET 无 body：读到请求头结束即可。
        loop {
            match stream.read(&mut chunk) {
                Ok(0) => return,
                Ok(read) => {
                    buf.extend_from_slice(&chunk[..read]);
                    if buf.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                Err(_) => return,
            }
        }
        let head = String::from_utf8_lossy(&buf);
        let Some(target) = head
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
        else {
            return;
        };
        targets.lock().expect("targets 锁").push(target.to_string());
        let body = if target.starts_with("/getid?type=7&") {
            r#"{"virtual":"101"}"#
        } else {
            r#"{"error":"unexpected request"}"#
        };
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        if stream.write_all(response.as_bytes()).is_err() {
            return;
        }
        let _ = stream.flush();
    }
}

/// HTTP 回退桥真挂上（lotus=true, lotus_grpc=false，stub 远端）：idmap 查询真走
/// 远端 /getid、本地 redb 不写，`/getid` 路由隐藏。
#[tokio::test(flavor = "multi_thread")]
async fn http_remote_mounted_hides_getid() {
    // 测试进程先装 provider：否则桥 spawn 预检失败降级本地，断言方向反。
    let _ = rustls::crypto::ring::default_provider().install_default();
    let stub = HttpStub::spawn();
    // 远端端口用 port 配置（Go GetPortValue）；lotus=true 时主 listener 改用
    // backup_port，port 不再被本地绑定占用。
    let (state, _dir) = state_with_stores(&format!(
        "  app_id: 102000001\n  enable_ws_server: true\n  disable_webui: true\n  lotus: true\n  lotus_grpc: false\n  server_dir: \"127.0.0.1\"\n  port: \"{}\"\n  backup_port: \"0\"\n",
        stub.port
    ));
    let handle = Runtime::start_with(state, options())
        .await
        .expect("HTTP 远程实例应启动");
    assert!(
        handle.idmap_grpc_addr().is_none(),
        "lotus=true 是客户端方向，不应起服务端"
    );
    let store = handle.idmap_arc().expect("idmap 应装配");

    // 连接层重试（环境缓解，同 storage 测试口径）：火绒偶发中止环回连接（10053）。
    // lookup_virtual_value 是纯查询：远程模式只打 type=7，不触发分配写入。
    let mut attempt = 0;
    let row = loop {
        match store.lookup_virtual_value("openid-http-remote") {
            Ok(row) => break row,
            Err(err) if attempt < 3 && err.to_string().contains("failed to send request") => {
                attempt += 1;
            }
            Err(err) => panic!("HTTP 远程查询应命中 stub: {err}"),
        }
    };
    assert_eq!(row, 101);
    assert!(
        stub.targets
            .lock()
            .expect("targets 锁")
            .iter()
            .any(|target| target == "/getid?type=7&id=openid-http-remote"),
        "查询必须真走远端 /getid"
    );
    assert!(
        store.is_empty().expect("本地库检查"),
        "远程命中后本地 redb 不应写入"
    );

    let ws = handle.ws_addr().expect("正向 WS 应启动");
    let (status, _) = support::get(ws, "/getid?type=1&id=openid-x").await;
    assert_eq!(status, 404, "HTTP 远程真挂上后 /getid 应隐藏");
    handle.shutdown_with_timeout(Duration::from_secs(5)).await;
}

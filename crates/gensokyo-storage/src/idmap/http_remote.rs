//! lotus HTTP 回退桥（`lotus=true && lotus_grpc=false && !without_idmaps`，设计 §2.3）。
//!
//! 与 [`super::remote::RemoteBridge`] 同型的专职 OS 线程桥：门面只做 `send(Job)` +
//! `recv_timeout`，HTTP 请求在桥线程内用 `reqwest::blocking` 执行。blocking 客户端
//! 自带内部 runtime，**不得在 async worker 线程构造**——这里统一在专职桥线程内构造，
//! 任意线程（含 async worker）调用门面都安全，不触碰调用线程的 runtime。
//!
//! 行为逐条对齐 Go `idmap/service.go` 17 个 v2 函数的 HTTP 分支：
//! - 协议判定：`port == "443" || force_ssl` → https（service.go:690-693）。
//! - type 表见设计 §1.3；type=11 走 `/getrealvalues`（service.go:1956）。
//! - 30s 总超时 + 4MiB 响应上限（`idmap/http_client.go:9,40-43`）。
//! - **不带 token/timestamp**：Go 的 `buildGetIDURL`（HMAC 签名，service.go:44-93）
//!   全文件零调用者，是死代码；17 个 HTTP 分支全部裸拼 URL。设 `lotus_password` 时
//!   远端 /getid 会 401——存量缺陷照抄，修复另立任务（lead 预验证笔记 §3）。
//!
//! TLS：reqwest `rustls-no-provider`，provider 由二进制侧进程级安装（gensokyo-bin
//! startup 装 ring）。未安装 provider 时构建客户端会 panic，而 release `panic="abort"`
//! 下桥线程 panic 直接中止进程——因此桥线程内先预检
//! [`rustls::crypto::CryptoProvider::get_default`]，未安装时 [`HttpRemoteBridge::spawn`]
//! 与建连失败同口径返回 Err（调用方决定降级本地），不走 panic 路径。

use std::fmt::Write as _;
use std::io::Read;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::blocking::Client;
use serde_json::Value;

use crate::error::StorageError;

/// Go `idmapHTTPClient` 的 30s 总超时（http_client.go:41）。
pub const DEFAULT_HTTP_TIMEOUT: Duration = Duration::from_secs(30);

/// Go `maxIDMapResponseBytes`：4MiB 响应上限（http_client.go:9）。
const MAX_RESPONSE_BYTES: u64 = 4 << 20;

/// 门面等待回复的余量：请求本身已有 `timeout` 上限，回复等待再加 5s 让请求超时
/// 先触发（Go 无门面层，此为有意的安全性差异，同 P1 口径）。
const RECV_MARGIN: Duration = Duration::from_secs(5);

/// 一次 HTTP 请求：完整 URL + 响应处理口径 + 随 Job 走的回复通道。
struct Job {
    url: String,
    style: ResponseStyle,
    reply: mpsc::Sender<Result<Value, StorageError>>,
}

/// Go 各 type 的响应处理分三种口径（逐函数核对 service.go）。
enum ResponseStyle {
    /// 先解码 JSON 再查状态码（type 1/2/7/8/9/11/13/14/16/17）：解码失败优先于
    /// 非 200 报错。`error_from_body=false` 仅 type 6：非 200 用固定文案，不取
    /// error 字段（service.go:1722-1724）。
    JsonFirst { error_from_body: bool },
    /// 只查状态码、不读 body（type 3/5/12/15）：`with_status=true` 仅 type 3/15，
    /// 非 200 带 `resp.Status`（service.go:1215,1327）；type 5/12 固定文案
    /// （service.go:1683,2070）。
    StatusOnly { with_status: bool },
    /// type 4：先查状态码，再限长读 body，取 `value` 字段按 Go `%v` 字符串化
    /// （service.go:1369-1390）。
    ConfigValue,
}

/// 同步门面 → 专职桥线程的 HTTP 回退桥（Go lotus 客户端 `lotus_grpc=false` 分支）。
///
/// 任意线程（含 async worker）可安全调用：门面只走 std mpsc，绝不在调用线程
/// `block_on`；blocking 客户端在桥线程内构造与独占。drop 本结构后发送端断开，
/// 桥线程处理完在途 Job 即退出。
///
/// # Errors
///
/// 门面统一返回 [`StorageError::Remote`]：请求失败/超时、非 200、响应超限/解析
/// 失败、桥线程死亡。
#[derive(Debug)]
pub struct HttpRemoteBridge {
    base: String,
    tx: mpsc::Sender<Job>,
    handle: JoinHandle<()>,
    recv_timeout: Duration,
}

impl HttpRemoteBridge {
    /// 启动专职桥线程。`server_dir` 是远端主机地址（Go `GetServer_dir`），`port`
    /// 是远端 port 配置（Go `GetPortValue`）；协议判定照抄 Go（见 [`protocol`]）。
    ///
    /// 与 gRPC 桥不同，HTTP 客户端**不预建连**（Go `http.Client` 同）：不可达在
    /// 首次请求时才报错。构建客户端失败返回 Err，由调用方决定降级；未安装 rustls
    /// provider 时预检直接回 Err（release `panic="abort"` 不允许桥线程 panic，
    /// 见模块文档）。
    pub fn spawn(
        server_dir: impl Into<String>,
        port: impl Into<String>,
        force_ssl: bool,
        timeout: Duration,
    ) -> Result<Self, StorageError> {
        let server_dir = server_dir.into();
        let port = port.into();
        let base = format!("{}://{}:{}", protocol(&port, force_ssl), server_dir, port);
        let (tx, rx) = mpsc::channel::<Job>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), StorageError>>();
        let handle = std::thread::Builder::new()
            .name("idmap-http-remote-bridge".to_string())
            .spawn(move || {
                // release `panic="abort"` 下桥线程 panic 会中止整个进程：provider
                // 未安装时预检直接回 Err，不走 Client::builder() 的 panic 路径。
                if rustls::crypto::CryptoProvider::get_default().is_none() {
                    let _ = ready_tx.send(Err(StorageError::Remote(
                        "未安装 rustls CryptoProvider，HTTP 客户端不构建（生产由二进制 startup 安装 ring）"
                            .to_string(),
                    )));
                    return;
                }
                // blocking 客户端自带内部 runtime，必须在专职线程（非 async worker）构造。
                let client = match Client::builder().timeout(timeout).build() {
                    Ok(client) => client,
                    Err(err) => {
                        let _ = ready_tx.send(Err(StorageError::Remote(format!(
                            "HTTP 客户端构建失败: {err}"
                        ))));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(()));
                drop(ready_tx);
                while let Ok(job) = rx.recv() {
                    let result = execute(&client, &job.url, job.style);
                    // 回复发送失败（调用方已超时离开）静默丢弃。
                    let _ = job.reply.send(result);
                }
            })
            .map_err(|err| StorageError::Remote(format!("桥线程创建失败: {err}")))?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                base,
                tx,
                handle,
                recv_timeout: timeout + RECV_MARGIN,
            }),
            Ok(Err(err)) => {
                let _ = handle.join();
                Err(err)
            }
            Err(_) => {
                let _ = handle.join();
                Err(StorageError::Remote("桥线程意外退出".to_string()))
            }
        }
    }

    /// 桥线程是否仍在运行。
    #[must_use]
    pub fn is_alive(&self) -> bool {
        !self.handle.is_finished()
    }

    /// 门面公共路径：send(Job) + recv_timeout；超时/桥死亡/发送失败都回 Remote。
    fn call(&self, url: String, style: ResponseStyle) -> Result<Value, StorageError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .send(Job {
                url,
                style,
                reply: reply_tx,
            })
            .map_err(|_| StorageError::Remote("桥线程已死亡: Job 发送失败".to_string()))?;
        match reply_rx.recv_timeout(self.recv_timeout) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => Err(StorageError::Remote(format!(
                "等待桥回复超时（{}s）",
                self.recv_timeout.as_secs()
            ))),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err(StorageError::Remote("桥线程已死亡: 回复通道断开".to_string()))
            }
        }
    }

    /// type=1：`StoreIDv2`（service.go:696），`response["row"].(float64)` → int64。
    pub fn store_idv2(&self, id: impl Into<String>) -> Result<i64, StorageError> {
        let url = format!("{}/getid?type=1&id={}", self.base, id.into());
        let value = self.call(url, ResponseStyle::JsonFirst { error_from_body: true })?;
        number_field(&value, "row")
    }

    /// type=2：`RetrieveRowByIDv2`（service.go:1035），取 `id` 字符串。
    pub fn retrieve_row_by_idv2(&self, rowid: impl Into<String>) -> Result<String, StorageError> {
        let url = format!("{}/getid?type=2&id={}", self.base, rowid.into());
        let value = self.call(url, ResponseStyle::JsonFirst { error_from_body: true })?;
        string_field(&value, "id")
    }

    /// type=3：`WriteConfigv2`（service.go:1199-1206），`url.Values` 表单编码
    /// （键字母序），只查状态码。
    pub fn write_config_v2(
        &self,
        section: impl Into<String>,
        subtype: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<(), StorageError> {
        let url = format!(
            "{}/getid?{}",
            self.base,
            form_query(&[
                ("id", &section.into()),
                ("subtype", &subtype.into()),
                ("type", "3"),
                ("value", &value.into()),
            ])
        );
        self.call(url, ResponseStyle::StatusOnly { with_status: true })?;
        Ok(())
    }

    /// type=4：`ReadConfigv2`（service.go:1356-1390），`value` 字段按 Go `%v`
    /// 字符串化（服务端实际只回字符串）。
    pub fn read_config_v2(
        &self,
        section: impl Into<String>,
        subtype: impl Into<String>,
    ) -> Result<String, StorageError> {
        let url = format!(
            "{}/getid?{}",
            self.base,
            form_query(&[
                ("id", &section.into()),
                ("subtype", &subtype.into()),
                ("type", "4"),
            ])
        );
        let value = self.call(url, ResponseStyle::ConfigValue)?;
        Ok(value.as_str().unwrap_or_default().to_owned())
    }

    /// type=5：`UpdateVirtualValuev2`（service.go:1674），只查状态码。
    pub fn update_virtual_value_v2(
        &self,
        old_row_value: i64,
        new_row_value: i64,
    ) -> Result<(), StorageError> {
        let url = format!(
            "{}/getid?type=5&oldRowValue={old_row_value}&newRowValue={new_row_value}",
            self.base
        );
        self.call(url, ResponseStyle::StatusOnly { with_status: false })?;
        Ok(())
    }

    /// type=6：`RetrieveRealValuev2`（service.go:1710-1730）：只解析 `real`，
    /// 第一项是入参 virtualValue 的原样回显（Go 同，不取响应里的 virtual）。
    pub fn retrieve_real_value_v2(
        &self,
        virtual_value: i64,
    ) -> Result<(String, String), StorageError> {
        let url = format!(
            "{}/getid?type=6&virtualValue={virtual_value}",
            self.base
        );
        let value = self.call(url, ResponseStyle::JsonFirst { error_from_body: false })?;
        let real = string_field(&value, "real")?;
        Ok((virtual_value.to_string(), real))
    }

    /// type=7：`RetrieveVirtualValuev2`（service.go:1764-1785）：取 `virtual`，
    /// 第一项是入参 realValue 的原样回显（Go 同）。
    pub fn retrieve_virtual_value_v2(
        &self,
        real_value: impl Into<String>,
    ) -> Result<(String, String), StorageError> {
        let real_value = real_value.into();
        let url = format!("{}/getid?type=7&id={real_value}", self.base);
        let value = self.call(url, ResponseStyle::JsonFirst { error_from_body: true })?;
        let virtual_value = string_field(&value, "virtual")?;
        Ok((real_value, virtual_value))
    }

    /// type=8：`StoreIDv2Pro`（service.go:794-820），`row` + `subRow`。
    pub fn store_idv2_pro(
        &self,
        id: impl Into<String>,
        subid: impl Into<String>,
    ) -> Result<(i64, i64), StorageError> {
        let url = format!(
            "{}/getid?type=8&id={}&subid={}",
            self.base,
            id.into(),
            subid.into()
        );
        let value = self.call(url, ResponseStyle::JsonFirst { error_from_body: true })?;
        Ok((number_field(&value, "row")?, number_field(&value, "subRow")?))
    }

    /// type=9：`RetrieveRowByIDv2Pro`（service.go:937-963），`id` + `subid`。
    pub fn retrieve_row_by_idv2_pro(
        &self,
        rowid: impl Into<String>,
        sub_rowid: impl Into<String>,
    ) -> Result<(String, String), StorageError> {
        self.retrieve_type9(rowid.into(), sub_rowid.into(), "id", "subid")
    }

    /// type=9：`RetrieveVirtualValuev2Pro`（service.go:1818-1844）——与
    /// `RetrieveRowByIDv2Pro` 同一 URL 同一字段，仅字段错误文案不同
    /// （Go 用 first/second value，service.go:1836,1841）。
    pub fn retrieve_virtual_value_v2_pro(
        &self,
        real_value: impl Into<String>,
        real_value_sub: impl Into<String>,
    ) -> Result<(String, String), StorageError> {
        self.retrieve_type9(
            real_value.into(),
            real_value_sub.into(),
            "first value",
            "second value",
        )
    }

    /// type=9 两门面的共用实现：URL 与响应字段相同，错误文案标签按 Go 各函数原文。
    fn retrieve_type9(
        &self,
        id: String,
        subid: String,
        first_label: &str,
        second_label: &str,
    ) -> Result<(String, String), StorageError> {
        let url = format!("{}/getid?type=9&id={}&subid={}", self.base, id, subid);
        let value = self.call(url, ResponseStyle::JsonFirst { error_from_body: true })?;
        Ok((
            string_field_labeled(&value, "id", first_label)?,
            string_field_labeled(&value, "subid", second_label)?,
        ))
    }

    /// type=11：`RetrieveRealValuesv2Pro`（service.go:1956-1982）——**走
    /// `/getrealvalues`**（不是 /getid），`firstRealValue` + `secondRealValue`。
    pub fn retrieve_real_value_v2_pro(
        &self,
        virtual_value: i64,
        virtual_value_sub: i64,
    ) -> Result<(String, String), StorageError> {
        let url = format!(
            "{}/getrealvalues?type=11&id={virtual_value}&subid={virtual_value_sub}",
            self.base
        );
        let value = self.call(url, ResponseStyle::JsonFirst { error_from_body: true })?;
        Ok((
            string_field_labeled(&value, "firstRealValue", "first real value")?,
            string_field_labeled(&value, "secondRealValue", "second real value")?,
        ))
    }

    /// type=12：`UpdateVirtualValuev2Pro`（service.go:2059-2072），只查状态码。
    pub fn update_virtual_value_v2_pro(
        &self,
        old_virtual_value_1: i64,
        new_virtual_value_1: i64,
        old_virtual_value_2: i64,
        new_virtual_value_2: i64,
    ) -> Result<(), StorageError> {
        let url = format!(
            "{}/getid?type=12&oldVirtualValue1={old_virtual_value_1}&newVirtualValue1={new_virtual_value_1}&oldVirtualValue2={old_virtual_value_2}&newVirtualValue2={new_virtual_value_2}",
            self.base
        );
        self.call(url, ResponseStyle::StatusOnly { with_status: false })?;
        Ok(())
    }

    /// type=13：`SimplifiedStoreIDv2`（service.go:589-610），取 `row`。
    pub fn simplified_store_idv2(&self, id: impl Into<String>) -> Result<i64, StorageError> {
        let url = format!("{}/getid?type=13&id={}", self.base, id.into());
        let value = self.call(url, ResponseStyle::JsonFirst { error_from_body: true })?;
        number_field(&value, "row")
    }

    /// type=14：`FindSubKeysByIdPro`（service.go:2168-2199），`keys` 必须全为字符串。
    pub fn find_sub_keys_by_id_pro(&self, id: impl Into<String>) -> Result<Vec<String>, StorageError> {
        let url = format!("{}/getid?type=14&id={}", self.base, id.into());
        let value = self.call(url, ResponseStyle::JsonFirst { error_from_body: true })?;
        let keys = value
            .get("keys")
            .and_then(Value::as_array)
            .ok_or_else(|| StorageError::Remote("invalid response format for keys".to_string()))?;
        keys.iter()
            .map(|key| {
                key.as_str().map(str::to_owned).ok_or_else(|| {
                    StorageError::Remote("invalid key format in response".to_string())
                })
            })
            .collect()
    }

    /// type=15：`DeleteConfigv2`（service.go:1308-1328），表单编码，只查状态码。
    pub fn delete_config_v2(
        &self,
        section: impl Into<String>,
        subtype: impl Into<String>,
    ) -> Result<(), StorageError> {
        let url = format!(
            "{}/getid?{}",
            self.base,
            form_query(&[
                ("id", &section.into()),
                ("subtype", &subtype.into()),
                ("type", "15"),
            ])
        );
        self.call(url, ResponseStyle::StatusOnly { with_status: true })?;
        Ok(())
    }

    /// type=16：`StoreCachev2`（service.go:745-766），取 `row`。
    pub fn store_cache_v2(&self, id: impl Into<String>) -> Result<i64, StorageError> {
        let url = format!("{}/getid?type=16&id={}", self.base, id.into());
        let value = self.call(url, ResponseStyle::JsonFirst { error_from_body: true })?;
        number_field(&value, "row")
    }

    /// type=17：`RetrieveRowByCachev2`（service.go:1109-1130），取 `id`。
    pub fn retrieve_row_by_cache_v2(&self, rowid: impl Into<String>) -> Result<String, StorageError> {
        let url = format!("{}/getid?type=17&id={}", self.base, rowid.into());
        let value = self.call(url, ResponseStyle::JsonFirst { error_from_body: true })?;
        string_field(&value, "id")
    }
}

/// Go service.go:690-693：`port == "443" || force_ssl` → https，否则 http。
fn protocol(port: &str, force_ssl: bool) -> &'static str {
    if port == "443" || force_ssl {
        "https"
    } else {
        "http"
    }
}

/// Go `url.Values.Encode`：键按字母序、值经 `url.QueryEscape`。调用方按字母序
/// 传入 pairs（type 3/4/15 的键序固定为 id/subtype/type[/value]）。
fn form_query(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(key, value)| format!("{}={}", form_escape(key), form_escape(value)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Go `url.QueryEscape`：仅 A-Za-z0-9 与 `-_.~` 保留，空格 → `+`，其余 %XX。
fn form_escape(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            b' ' => out.push('+'),
            _ => {
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
    out
}

/// 桥线程内的单次请求执行。错误文案沿用 Go 的英文原文（parity 便于对拍）。
fn execute(client: &Client, url: &str, style: ResponseStyle) -> Result<Value, StorageError> {
    let response = client
        .get(url)
        .send()
        .map_err(|err| StorageError::Remote(format!("failed to send request: {err}")))?;
    let status = response.status();
    match style {
        ResponseStyle::StatusOnly { with_status } => {
            if status != StatusCode::OK {
                // Go type 5/12 固定文案（service.go:1683,2070）；type 3/15 带
                // resp.Status（service.go:1215,1327）。
                return Err(StorageError::Remote(if with_status {
                    format!("error response from server: {status}")
                } else {
                    "error response from server".to_string()
                }));
            }
            Ok(Value::Null)
        }
        ResponseStyle::ConfigValue => {
            if status != StatusCode::OK {
                return Err(StorageError::Remote(format!(
                    "error response from server: {status}"
                )));
            }
            let body = read_limited(response)?;
            let value: Value = serde_json::from_slice(&body).map_err(|err| {
                StorageError::Remote(format!("failed to unmarshal response: {err}"))
            })?;
            match value.get("value") {
                Some(found) => Ok(Value::String(go_value_string(found))),
                None => Err(StorageError::Remote("value not found in response".to_string())),
            }
        }
        ResponseStyle::JsonFirst { error_from_body } => {
            let body = read_limited(response)?;
            let value: Value = serde_json::from_slice(&body).map_err(|err| {
                StorageError::Remote(format!("failed to decode response: {err}"))
            })?;
            if status != StatusCode::OK {
                return Err(StorageError::Remote(if error_from_body {
                    let detail = value
                        .get("error")
                        .map(go_value_string)
                        .unwrap_or_else(|| "<nil>".to_owned());
                    format!("error response from server: {detail}")
                } else {
                    "error response from server".to_owned()
                }));
            }
            Ok(value)
        }
    }
}

/// Go `limitedResponseTransport`：`LimitReader(4MiB+1)`，超限报错
/// （http_client.go:33-36；ReadConfigv2 显式判超限 service.go:1373-1379）。
/// 其余 type 在 Go 里靠截断后 JSON 解码失败报错，这里统一显式判超限——同为 Err。
fn read_limited(response: reqwest::blocking::Response) -> Result<Vec<u8>, StorageError> {
    let mut body = Vec::new();
    response
        .take(MAX_RESPONSE_BYTES + 1)
        .read_to_end(&mut body)
        .map_err(|err| StorageError::Remote(format!("failed to read response body: {err}")))?;
    if body.len() as u64 > MAX_RESPONSE_BYTES {
        return Err(StorageError::Remote(format!(
            "response body exceeds {MAX_RESPONSE_BYTES} bytes"
        )));
    }
    Ok(body)
}

/// Go `response["row"].(float64)` → `int64(...)`：JSON 数字按 float64 截断。
/// serde_json 整数直取 i64（比 Go 的 float64 中转更精确，服务端行号远小于 2^53）。
fn number_field(value: &Value, field: &str) -> Result<i64, StorageError> {
    value
        .get(field)
        .and_then(|found| found.as_i64().or_else(|| found.as_f64().map(|num| num as i64)))
        .ok_or_else(|| StorageError::Remote(format!("invalid response format for {field}")))
}

/// Go `response["id"].(string)` 一族的字符串字段断言。
fn string_field(value: &Value, field: &str) -> Result<String, StorageError> {
    string_field_labeled(value, field, field)
}

/// 同 [`string_field`]，但错误文案标签按 Go 各函数原文（未必是字段名，
/// 如 type 9 虚拟变体 / type 11，service.go:1836,1974）。
fn string_field_labeled(value: &Value, field: &str, label: &str) -> Result<String, StorageError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| StorageError::Remote(format!("invalid response format for {label}")))
}

/// Go `fmt.Sprintf("%v", value)`（ReadConfigv2，service.go:1387）：服务端实际只回
/// 字符串；标量逐字对齐，复合值按 Go `%v` 形态兜底（map 键排序、元素空格分隔）。
fn go_value_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Bool(flag) => flag.to_string(),
        Value::Null => "<nil>".to_owned(),
        Value::Number(number) => number.to_string(),
        Value::Array(items) => format!(
            "[{}]",
            items.iter().map(go_value_string).collect::<Vec<_>>().join(" ")
        ),
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            format!(
                "map[{}]",
                keys.iter()
                    .map(|key| format!("{key}:{}", go_value_string(&map[key.as_str()])))
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::idmap::{IdMapOptions, IdMapStore};

    /// 手写 HTTP stub：专职线程 + std TcpListener，不引新依赖。responder 按请求
    /// target（path+query 逐字）决定 (status, body)，可 sleep 模拟慢响应。
    struct Stub {
        addr: SocketAddr,
        requests: Arc<Mutex<Vec<String>>>,
        stop: Arc<AtomicBool>,
        handle: Option<JoinHandle<()>>,
    }

    /// 测试专用端口分配器：从动态端口范围之外的专用段（20000–32767）顺序取端口。
    ///
    /// 本机 TCP 动态端口范围被自定义为 1024–15000（netsh 可查），`bind(0)` 与
    /// reqwest 出站连接的自动源端口共用同一系统级顺序游标，且游标与 TIME_WAIT
    /// 元组跨进程存活，密集测试下存在端口元组复用风险。专用段与出站源端口永不
    /// 重合；顺序分配 + bind 失败递增重试避免 stub 间冲突，并使 unreachable 用例
    /// 释放的端口不会被其他 stub 抢占。该段同时避开默认 Windows（49152+）与
    /// 默认 Linux（32768+）的动态范围。
    fn bind_test_listener() -> TcpListener {
        static NEXT: AtomicU16 = AtomicU16::new(20000);
        loop {
            let port = NEXT.fetch_add(1, Ordering::Relaxed);
            assert!(port < 32768, "测试端口段耗尽");
            if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) {
                return listener;
            }
        }
    }

    impl Stub {
        fn spawn(responder: impl Fn(&str) -> (u16, String) + Send + 'static) -> Self {
            let listener = bind_test_listener();
            let addr = listener.local_addr().unwrap();
            listener.set_nonblocking(true).unwrap();
            let requests = Arc::new(Mutex::new(Vec::new()));
            let stop = Arc::new(AtomicBool::new(false));
            let recorded = Arc::clone(&requests);
            let stopped = Arc::clone(&stop);
            let handle = std::thread::spawn(move || {
                while !stopped.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => handle_conn(stream, &recorded, &responder),
                        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            });
            Self {
                addr,
                requests,
                stop,
                handle: Some(handle),
            }
        }

        fn port(&self) -> String {
            self.addr.port().to_string()
        }

        fn targets(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }

        fn clear_targets(&self) {
            self.requests.lock().unwrap().clear();
        }
    }

    impl Drop for Stub {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }

    fn handle_conn(
        mut stream: TcpStream,
        requests: &Mutex<Vec<String>>,
        responder: &impl Fn(&str) -> (u16, String),
    ) {
        stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
        // keep-alive：同一连接顺序处理多个请求（Go net/http 默认同口径）。
        // 每请求新建连接会在本机触发火绒网络过滤的间歇性连接中止（10053），
        // 复用连接把每测试的建连数降到 1，大幅缩小暴露面。
        loop {
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            // GET 无 body：读到请求头结束即可（hyper 不做管线化，无越界字节）
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
            let Some(line) = head.lines().next() else {
                return;
            };
            let Some(target) = line.split_whitespace().nth(1) else {
                return;
            };
            requests.lock().unwrap().push(target.to_string());
            let (status, body) = responder(target);
            let reason = match status {
                200 => "OK",
                401 => "Unauthorized",
                404 => "Not Found",
                500 => "Internal Server Error",
                _ => "Status",
            };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            if stream.write_all(response.as_bytes()).is_err() {
                return;
            }
            let _ = stream.flush();
        }
    }

    /// 17 个 type 的罐头应答（字段名与 Go 服务端 getIDHandler.go 逐字一致）。
    fn table_response(target: &str) -> (u16, String) {
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let params: HashMap<&str, &str> =
            query.split('&').filter_map(|kv| kv.split_once('=')).collect();
        let ty = params.get("type").copied().unwrap_or("");
        let body = match (path, ty) {
            ("/getid", "1") => r#"{"row":101}"#,
            ("/getid", "2") => r#"{"id":"openid-user-1"}"#,
            ("/getid", "3") => r#"{"status":"success"}"#,
            ("/getid", "4") => r#"{"value":"abc"}"#,
            ("/getid", "5") => r#"{"status":"success"}"#,
            ("/getid", "6") => r#"{"virtual":"101","real":"openid-user-1"}"#,
            ("/getid", "7") => r#"{"virtual":"101"}"#,
            ("/getid", "8") => r#"{"row":201,"subRow":101}"#,
            ("/getid", "9") => r#"{"id":"openid-group-1","subid":"openid-user-1"}"#,
            ("/getrealvalues", "11") => {
                r#"{"firstRealValue":"openid-group-1","secondRealValue":"openid-user-1"}"#
            }
            ("/getid", "12") => r#"{"message":"Virtual values updated successfully"}"#,
            ("/getid", "13") => r#"{"row":301}"#,
            ("/getid", "14") => r#"{"keys":["sub-1","sub-2"]}"#,
            ("/getid", "15") => r#"{"status":"success"}"#,
            ("/getid", "16") => r#"{"row":401}"#,
            ("/getid", "17") => r#"{"id":"real-msg-1"}"#,
            _ => r#"{"error":"unexpected request"}"#,
        };
        (200, body.to_string())
    }

    /// 测试用桥统一入口（provider 安装点：no-provider 路线需要进程级 CryptoProvider，
    /// 见模块文档；二进制侧由 gensokyo-bin startup 安装 ring）。
    fn spawn_test_bridge(port: String, force_ssl: bool, timeout: Duration) -> HttpRemoteBridge {
        let _ = rustls::crypto::ring::default_provider().install_default();
        HttpRemoteBridge::spawn("127.0.0.1", port, force_ssl, timeout).unwrap()
    }

    fn bridge_to(stub: &Stub) -> HttpRemoteBridge {
        spawn_test_bridge(stub.port(), false, Duration::from_secs(5))
    }

    /// 连接层重试（环境缓解，非产品行为）：本机火绒网络过滤驱动（HipsDaemon/
    /// wsctrlsvc）会间歇性中止已建立的环回 TCP 连接，reqwest 报
    /// WSAECONNABORTED（os error 10053），产品桥如实上报 `failed to send request`。
    /// 仅对该连接层错误重试至多 3 次，其余错误（超时/非 200/解析失败）原样上抛。
    fn retry_conn<F, T>(mut f: F) -> Result<T, StorageError>
    where
        F: FnMut() -> Result<T, StorageError>,
    {
        let mut attempt = 0;
        loop {
            match f() {
                Ok(value) => return Ok(value),
                Err(err) => {
                    if attempt < 3 && err.to_string().contains("failed to send request") {
                        attempt += 1;
                        continue;
                    }
                    return Err(err);
                }
            }
        }
    }

    /// 整段重试：供逐字断言目标序列的测试使用。连接层错误时清空 stub 已记录的
    /// 目标并从头重跑整段，保证重试不会在精确序列断言里留下重复/残缺条目；
    /// 断言失败（panic）不受影响，直接上抛。
    fn retry_seq<F>(stub: &Stub, mut f: F)
    where
        F: FnMut() -> Result<(), StorageError>,
    {
        let mut attempt = 0;
        loop {
            stub.clear_targets();
            match f() {
                Ok(()) => return,
                Err(err) => {
                    if attempt < 3 && err.to_string().contains("failed to send request") {
                        attempt += 1;
                        continue;
                    }
                    panic!("{err}");
                }
            }
        }
    }

    /// 验收 a：17 个操作各调一次，URL path+query 逐字断言（含 type=11 的
    /// /getrealvalues），响应解析逐 type 对齐 Go；URL 不含 token/timestamp。
    #[test]
    fn seventeen_operations_urls_and_parsing() {
        let stub = Stub::spawn(table_response);
        let bridge = bridge_to(&stub);

        retry_seq(&stub, || {
            assert_eq!(bridge.store_idv2("openid-user-1")?, 101); // type=1
            assert_eq!(bridge.retrieve_row_by_idv2("101")?, "openid-user-1"); // type=2
            bridge.write_config_v2("lotus", "token", "abc")?; // type=3
            assert_eq!(bridge.read_config_v2("lotus", "token")?, "abc"); // type=4
            bridge.update_virtual_value_v2(101, 201)?; // type=5
            // type=6：第一项是入参回显，不是响应里的 virtual
            assert_eq!(
                bridge.retrieve_real_value_v2(101)?,
                ("101".to_string(), "openid-user-1".to_string())
            );
            // type=7：第一项是入参回显
            assert_eq!(
                bridge.retrieve_virtual_value_v2("openid-user-1")?,
                ("openid-user-1".to_string(), "101".to_string())
            );
            assert_eq!(
                bridge.store_idv2_pro("openid-group-1", "openid-user-1")?,
                (201, 101)
            ); // type=8
            assert_eq!(
                bridge.retrieve_row_by_idv2_pro("201", "101")?,
                ("openid-group-1".to_string(), "openid-user-1".to_string())
            ); // type=9
            assert_eq!(
                bridge
                    .retrieve_virtual_value_v2_pro("openid-group-1", "openid-user-1")?,
                ("openid-group-1".to_string(), "openid-user-1".to_string())
            ); // type=9（同 URL）
            assert_eq!(
                bridge.retrieve_real_value_v2_pro(201, 101)?,
                ("openid-group-1".to_string(), "openid-user-1".to_string())
            ); // type=11
            bridge.update_virtual_value_v2_pro(101, 201, 301, 401)?; // type=12
            assert_eq!(bridge.simplified_store_idv2("openid-user-3")?, 301); // type=13
            assert_eq!(
                bridge.find_sub_keys_by_id_pro("openid-group-1")?,
                vec!["sub-1".to_string(), "sub-2".to_string()]
            ); // type=14
            bridge.delete_config_v2("lotus", "token")?; // type=15
            assert_eq!(bridge.store_cache_v2("real-msg-1")?, 401); // type=16
            assert_eq!(bridge.retrieve_row_by_cache_v2("401")?, "real-msg-1"); // type=17

            let targets = stub.targets();
            let expected = [
                "/getid?type=1&id=openid-user-1",
                "/getid?type=2&id=101",
                "/getid?id=lotus&subtype=token&type=3&value=abc",
                "/getid?id=lotus&subtype=token&type=4",
                "/getid?type=5&oldRowValue=101&newRowValue=201",
                "/getid?type=6&virtualValue=101",
                "/getid?type=7&id=openid-user-1",
                "/getid?type=8&id=openid-group-1&subid=openid-user-1",
                "/getid?type=9&id=201&subid=101",
                "/getid?type=9&id=openid-group-1&subid=openid-user-1",
                "/getrealvalues?type=11&id=201&subid=101",
                "/getid?type=12&oldVirtualValue1=101&newVirtualValue1=201&oldVirtualValue2=301&newVirtualValue2=401",
                "/getid?type=13&id=openid-user-3",
                "/getid?type=14&id=openid-group-1",
                "/getid?id=lotus&subtype=token&type=15",
                "/getid?type=16&id=real-msg-1",
                "/getid?type=17&id=401",
            ];
            assert_eq!(targets, expected);
            // Go 存量缺陷照抄：buildGetIDURL 是死代码，URL 不带 token/timestamp
            for target in &targets {
                assert!(!target.contains("token="), "{target}");
                assert!(!target.contains("timestamp="), "{target}");
            }
            assert!(bridge.is_alive());
            Ok(())
        });
    }

    /// type 3/4/15 的表单编码对齐 Go url.Values.Encode（键字母序、空格→+）。
    #[test]
    fn form_types_escape_like_go_url_values() {
        let stub = Stub::spawn(table_response);
        let bridge = bridge_to(&stub);
        retry_conn(|| bridge.write_config_v2("sec tion", "sub&type", "a b=c&d")).unwrap();
        assert_eq!(
            stub.targets()[0],
            "/getid?id=sec+tion&subtype=sub%26type&type=3&value=a+b%3Dc%26d"
        );
    }

    /// 验收 b：协议判定照抄 Go service.go:690-693。
    #[test]
    fn protocol_selection_matches_go() {
        assert_eq!(protocol("443", false), "https");
        assert_eq!(protocol("443", true), "https");
        assert_eq!(protocol("8080", true), "https");
        assert_eq!(protocol("8080", false), "http");
        assert_eq!(protocol("80", false), "http");
    }

    /// 验收 b'：force_ssl / port=443 构造 https base（只断言 URL，不发请求）。
    #[test]
    fn force_ssl_and_port_443_build_https_base() {
        let bridge = spawn_test_bridge("8080".to_string(), true, Duration::from_secs(5));
        assert!(bridge.base.starts_with("https://127.0.0.1:8080"), "{}", bridge.base);
        let bridge = spawn_test_bridge("443".to_string(), false, Duration::from_secs(5));
        assert!(bridge.base.starts_with("https://127.0.0.1:443"), "{}", bridge.base);
        let bridge = spawn_test_bridge("8080".to_string(), false, Duration::from_secs(5));
        assert!(bridge.base.starts_with("http://127.0.0.1:8080"), "{}", bridge.base);
    }

    /// 验收 c-1：超 4MiB 响应 → Remote；桥不死。
    #[test]
    fn body_over_4mib_returns_err() {
        let stub = Stub::spawn(|_| (200, "x".repeat(4 * 1024 * 1024 + 1)));
        let bridge = bridge_to(&stub);
        let err = retry_conn(|| bridge.store_idv2("openid-x")).unwrap_err();
        assert!(matches!(err, StorageError::Remote(_)), "{err:?}");
        assert!(err.to_string().contains("exceeds"), "{err}");
        assert!(bridge.is_alive());
    }

    /// 验收 c-1'：恰好 4MiB 的合法 JSON 可接受（Go LimitReader(4MiB+1) 同口径）。
    #[test]
    fn body_exactly_4mib_is_accepted() {
        let overhead = r#"{"row":1,"pad":""}"#.len();
        let body = format!(
            r#"{{"row":1,"pad":"{}"}}"#,
            "a".repeat(4 * 1024 * 1024 - overhead)
        );
        assert_eq!(body.len(), 4 * 1024 * 1024);
        let stub = Stub::spawn(move |_| (200, body.clone()));
        let bridge = bridge_to(&stub);
        assert_eq!(retry_conn(|| bridge.store_idv2("openid-x")).unwrap(), 1);
    }

    /// 验收 c-2：请求超时 → Remote；桥不死（Go 30s 总超时同语义，测试用短超时）。
    #[test]
    fn request_timeout_returns_err_and_bridge_survives() {
        let stub = Stub::spawn(|_| {
            std::thread::sleep(Duration::from_secs(3));
            (200, r#"{"row":1}"#.to_string())
        });
        let bridge = spawn_test_bridge(stub.port(), false, Duration::from_millis(300));
        let err = bridge.store_idv2("openid-slow").unwrap_err();
        assert!(matches!(err, StorageError::Remote(_)), "{err:?}");
        assert!(bridge.is_alive());
    }

    /// 验收 c-3：非 200 取响应 error 字段（JsonFirst）；StatusOnly 分两档——
    /// type 3/15 带 resp.Status，type 5/12 固定文案（service.go:1215,1327,1683,2070）。
    #[test]
    fn non_200_error_field_is_propagated() {
        let stub = Stub::spawn(|_| (500, r#"{"error":"boom"}"#.to_string()));
        let bridge = bridge_to(&stub);
        let err = retry_conn(|| bridge.store_idv2("openid-x")).unwrap_err();
        assert!(err.to_string().contains("boom"), "{err}");
        // type 3：带 resp.Status
        let err = retry_conn(|| bridge.write_config_v2("lotus", "token", "abc")).unwrap_err();
        assert!(
            matches!(&err, StorageError::Remote(msg) if msg == "error response from server: 500 Internal Server Error"),
            "{err:?}"
        );
        // type 15：带 resp.Status
        let err = retry_conn(|| bridge.delete_config_v2("lotus", "token")).unwrap_err();
        assert!(
            matches!(&err, StorageError::Remote(msg) if msg == "error response from server: 500 Internal Server Error"),
            "{err:?}"
        );
        // type 5：固定文案，不带状态码
        let err = retry_conn(|| bridge.update_virtual_value_v2(1, 2)).unwrap_err();
        assert!(
            matches!(&err, StorageError::Remote(msg) if msg == "error response from server"),
            "{err:?}"
        );
        // type 12：固定文案，不带状态码
        let err = retry_conn(|| bridge.update_virtual_value_v2_pro(1, 2, 3, 4)).unwrap_err();
        assert!(
            matches!(&err, StorageError::Remote(msg) if msg == "error response from server"),
            "{err:?}"
        );
    }

    /// Go 口径：先解码再查状态码——非 200 且 body 非法 JSON 时解码错误优先。
    #[test]
    fn non_200_with_invalid_json_fails_decode_first() {
        let stub = Stub::spawn(|_| (500, "not json".to_string()));
        let bridge = bridge_to(&stub);
        let err = retry_conn(|| bridge.store_idv2("openid-x")).unwrap_err();
        assert!(err.to_string().contains("decode"), "{err}");
    }

    /// 非 200 且无 error 字段 → Go `%s` 的 `<nil>` 文案。
    #[test]
    fn non_200_without_error_field_reports_nil() {
        let stub = Stub::spawn(|_| (404, r#"{}"#.to_string()));
        let bridge = bridge_to(&stub);
        let err = retry_conn(|| bridge.retrieve_row_by_idv2("999")).unwrap_err();
        assert!(err.to_string().contains("<nil>"), "{err}");
    }

    /// 200 但缺字段 → invalid response format（Go 类型断言失败同语义）。
    #[test]
    fn missing_row_field_is_invalid_format() {
        let stub = Stub::spawn(|_| (200, r#"{"id":"x"}"#.to_string()));
        let bridge = bridge_to(&stub);
        let err = retry_conn(|| bridge.store_idv2("openid-x")).unwrap_err();
        assert!(err.to_string().contains("invalid response format"), "{err}");
    }

    /// 验收 c-4：桥死亡 → is_alive=false，门面回 Remote（同 P1 拆发送端手法）。
    #[test]
    fn dead_bridge_facades_return_err() {
        let stub = Stub::spawn(table_response);
        let bridge = bridge_to(&stub);
        let HttpRemoteBridge {
            base,
            tx,
            handle,
            recv_timeout,
        } = bridge;
        drop(tx);
        while !handle.is_finished() {
            std::thread::yield_now();
        }
        let (dead_tx, dead_rx) = mpsc::channel::<Job>();
        drop(dead_rx);
        let bridge = HttpRemoteBridge {
            base,
            tx: dead_tx,
            handle,
            recv_timeout,
        };
        assert!(!bridge.is_alive());
        let err = bridge.store_idv2("openid-dead").unwrap_err();
        assert!(matches!(err, StorageError::Remote(_)), "{err:?}");
    }

    /// 不可达远端：HTTP 不预建连（Go 同），spawn 成功、首次请求回 Remote。
    #[test]
    fn unreachable_remote_errors_on_first_call() {
        let listener = bind_test_listener();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let bridge = spawn_test_bridge(addr.port().to_string(), false, Duration::from_secs(2));
        let err = bridge.store_idv2("openid-x").unwrap_err();
        assert!(matches!(err, StorageError::Remote(_)), "{err:?}");
        assert!(bridge.is_alive());
    }

    /// 验收 d：with_http_remote 后 11 个分支点全走 stub 远端，本地 redb 全表为空；
    /// #[tokio::test] 默认 current_thread flavor——门面阻塞唯一 worker 也不 panic
    /// （stub 在独立 OS 线程，不依赖调用方 runtime）。
    #[tokio::test]
    async fn with_http_remote_routes_to_stub_and_never_writes_local() {
        let stub = Stub::spawn(table_response);
        let bridge = bridge_to(&stub);
        let dir = tempfile::tempdir().unwrap();
        let store = IdMapStore::open(
            IdMapOptions::new(
                dir.path().join("openid-map.redb"),
                dir.path().join("msgid-map.redb"),
            )
            .with_app_id("10000001"),
        )
        .unwrap()
        .with_http_remote(bridge);

        // type=1：分配在远端；归一化身份串本地派生（同 P2 口径）
        let (raw, row) = retry_conn(|| store.retrieve_virtual_value("openid-user-1")).unwrap();
        assert_eq!(raw, "openid-user-1");
        assert_eq!(row, 101);
        // type=2
        assert_eq!(
            retry_conn(|| store.retrieve_row_by_id(101)).unwrap(),
            "openid-user-1"
        );
        // type=7：只查不分配
        assert_eq!(
            retry_conn(|| store.lookup_virtual_value("openid-user-1")).unwrap(),
            101
        );
        // type=3/4/15
        retry_conn(|| store.write_config("lotus", "token", "abc")).unwrap();
        assert_eq!(retry_conn(|| store.read_config("lotus", "token")).unwrap(), "abc");
        assert!(retry_conn(|| store.delete_config("lotus", "token")).unwrap());
        // type=5 / type=12
        retry_conn(|| store.update_virtual_value(101, 201)).unwrap();
        retry_conn(|| store.update_virtual_value_pro(101, 201, 301, 401)).unwrap();
        // type=14
        assert_eq!(
            retry_conn(|| store.find_sub_keys_by_id("openid-group-1")).unwrap(),
            vec!["sub-1".to_string(), "sub-2".to_string()]
        );
        // type=16/17（store_cache 委托 store_cache_with_ttl，TTL 由服务端决定）
        assert_eq!(retry_conn(|| store.store_cache("real-msg-1")).unwrap(), 401);
        assert_eq!(
            retry_conn(|| store.retrieve_row_by_cache(401)).unwrap(),
            "real-msg-1"
        );
        assert_eq!(
            retry_conn(|| store.store_cache_with_ttl("real-msg-2", Duration::from_secs(60)))
                .unwrap(),
            401
        );

        // 核心断言：远程全命中后本地 redb 一行未写（is_empty 查全部数据表）
        assert!(store.is_empty().unwrap());

        // 本地-only 方法不受影响（Go 同样无远程分支）
        store.store_user_name(101, "Alice");
        assert_eq!(store.get_user_name(101).as_deref(), Some("Alice"));
        store.store_latest_msg_id("group-1", "user-1", "msg-1").unwrap();
        assert_eq!(store.get_latest_msg_id("group-1", "user-1").unwrap(), "msg-1");

        // 请求确实到了 stub：11 个分支点只用 /getid（type=11 不在分支点内）
        let targets = stub.targets();
        assert!(targets.iter().any(|t| t == "/getid?type=1&id=openid-user-1"));
        assert!(targets.iter().any(|t| t == "/getid?type=16&id=real-msg-1"));
        assert!(targets.iter().all(|t| t.starts_with("/getid?")));
    }

    /// 验收 d'：远程错误直接上抛 Remote，不做运行时本地回退（同 P2 口径）。
    #[tokio::test]
    async fn with_http_remote_error_does_not_fall_back_to_local() {
        let stub = Stub::spawn(|_| (500, r#"{"error":"remote down"}"#.to_string()));
        let bridge = bridge_to(&stub);
        let dir = tempfile::tempdir().unwrap();
        let store = IdMapStore::open(
            IdMapOptions::new(
                dir.path().join("openid-map.redb"),
                dir.path().join("msgid-map.redb"),
            )
            .with_app_id("10000001"),
        )
        .unwrap()
        .with_http_remote(bridge);
        let err = store.retrieve_virtual_value("openid-x").unwrap_err();
        assert!(matches!(err, StorageError::Remote(_)), "{err:?}");
        assert!(store.is_empty().unwrap());
    }
}
//! 远程 idmap 桥（方案 C）：专职 OS 线程 + 独立 `current_thread` tokio runtime。
//!
//! [`IdMapClient`] 的 17 个方法是 async 且要 `&mut self`，而 idmap 的调用面全同步、
//! 且常发生在 async worker 内（入站/deferred 管线）。桥线程独占客户端与 runtime，
//! 同步门面只做 `send(Job)` + `recv_timeout`，不触碰调用线程的 runtime——没有
//! `Handle::block_on` 在 async 上下文的 panic，也没有 `block_in_place` 的 flavor 限制。
//!
//! 超时口径：单次 RPC 超时由 [`RemoteBridge::spawn`] 的 `rpc_timeout` 经
//! [`Endpoint::timeout`] 生效；门面等待回复 = `rpc_timeout` + 5s 余量（对齐 HTTP 桥
//! `http_remote::RECV_MARGIN` 口径，排队 Job 不误报门面超时）。
//! Go gRPC 用 `context.Background()` 无超时，Rust 加超时是有意的安全性差异
//! （lotus gRPC 客户端设计 §2.2）。
//!
//! 建连失败返回 Err，由调用方决定降级本地；运行期远端不可达只回 Err，不自动切本地
//! （避免双写分裂）。tonic Channel 自带断线重连。

use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use tonic::transport::Endpoint;

use super::grpc::{IdMapClient, pb};
use crate::error::StorageError;

/// 门面等待回复的余量：单次 RPC 已有 `rpc_timeout` 上限，回复等待再加 5s 让 RPC
/// 超时先触发，并发 + 慢远端下排队 Job 不误报门面超时（同 HTTP 桥 RECV_MARGIN 口径）。
const RECV_MARGIN: Duration = Duration::from_secs(5);

/// 一次 RPC 请求：方法参数 + 随 Job 走的回复通道，桥线程不持全局状态。
enum Job {
    StoreIdv2 {
        id_or_row: String,
        reply: mpsc::Sender<Result<pb::StoreIdResponse, StorageError>>,
    },
    RetrieveRowByIdv2 {
        id_or_row: String,
        reply: mpsc::Sender<Result<pb::RetrieveRowByIdResponse, StorageError>>,
    },
    WriteConfigV2 {
        section: String,
        subtype: String,
        value: String,
        reply: mpsc::Sender<Result<pb::WriteConfigResponse, StorageError>>,
    },
    ReadConfigV2 {
        section: String,
        subtype: String,
        reply: mpsc::Sender<Result<pb::ReadConfigResponse, StorageError>>,
    },
    UpdateVirtualValueV2 {
        old_virtual_value: i64,
        new_virtual_value: i64,
        reply: mpsc::Sender<Result<pb::UpdateVirtualValueResponse, StorageError>>,
    },
    RetrieveRealValueV2 {
        virtual_value: i64,
        reply: mpsc::Sender<Result<pb::RetrieveRealValueResponse, StorageError>>,
    },
    RetrieveRealValueV2Pro {
        virtual_value: i64,
        virtual_value_sub: i64,
        reply: mpsc::Sender<Result<pb::RetrieveRealValueResponsePro, StorageError>>,
    },
    RetrieveVirtualValueV2 {
        real_value: String,
        reply: mpsc::Sender<Result<pb::RetrieveVirtualValueResponse, StorageError>>,
    },
    StoreIdv2Pro {
        id_or_row: String,
        subid: String,
        reply: mpsc::Sender<Result<pb::StoreIdProResponse, StorageError>>,
    },
    RetrieveRowByIdv2Pro {
        id_or_row: String,
        subid: String,
        reply: mpsc::Sender<Result<pb::RetrieveRowByIdProResponse, StorageError>>,
    },
    RetrieveVirtualValueV2Pro {
        id_or_row: String,
        subid: String,
        reply: mpsc::Sender<Result<pb::RetrieveVirtualValueProResponse, StorageError>>,
    },
    UpdateVirtualValueV2Pro {
        old_virtual_value_1: i64,
        new_virtual_value_1: i64,
        old_virtual_value_2: i64,
        new_virtual_value_2: i64,
        reply: mpsc::Sender<Result<pb::UpdateVirtualValueProResponse, StorageError>>,
    },
    SimplifiedStoreIdv2 {
        id_or_row: String,
        reply: mpsc::Sender<Result<pb::SimplifiedStoreIdResponse, StorageError>>,
    },
    FindSubKeysByIdPro {
        id: String,
        reply: mpsc::Sender<Result<pb::FindSubKeysResponse, StorageError>>,
    },
    DeleteConfigV2 {
        section: String,
        subtype: String,
        reply: mpsc::Sender<Result<pb::DeleteConfigResponse, StorageError>>,
    },
    StoreCacheV2 {
        id_or_row: String,
        reply: mpsc::Sender<Result<pb::StoreCacheResponse, StorageError>>,
    },
    RetrieveRowByCacheV2 {
        id_or_row: String,
        reply: mpsc::Sender<Result<pb::RetrieveRowByCacheResponse, StorageError>>,
    },
}

/// 同步门面 → 专职桥线程的 gRPC 桥。
///
/// 任意线程（含 async worker）可安全调用：门面只走 std mpsc，绝不在调用线程
/// `block_on`；桥线程独占 [`IdMapClient`]，天然满足其 `&mut self`，无需 Mutex。
/// drop 本结构后发送端断开，桥线程处理完在途 Job 即退出。
///
/// # Errors
///
/// 门面统一返回 [`StorageError::Remote`]：gRPC Status、RPC/等待超时、桥线程死亡。
#[derive(Debug)]
pub struct RemoteBridge {
    tx: mpsc::Sender<Job>,
    handle: JoinHandle<()>,
    recv_timeout: Duration,
}

impl RemoteBridge {
    /// 在专职 OS 线程内建连并启动桥循环；建连失败返回 Err（调用方决定降级）。
    ///
    /// `rpc_timeout` 作用于每次 RPC（[`Endpoint::timeout`]）；门面等待回复 =
    /// `rpc_timeout` + 5s 余量（[`RECV_MARGIN`]）。
    pub fn spawn(endpoint: Endpoint, rpc_timeout: Duration) -> Result<Self, StorageError> {
        let (tx, rx) = mpsc::channel::<Job>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), StorageError>>();
        let handle = std::thread::Builder::new()
            .name("idmap-remote-bridge".to_string())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(err) => {
                        let _ = ready_tx.send(Err(StorageError::Remote(format!(
                            "桥 runtime 创建失败: {err}"
                        ))));
                        return;
                    }
                };
                let client =
                    match runtime.block_on(IdMapClient::connect(endpoint.timeout(rpc_timeout))) {
                        Ok(client) => client,
                        Err(err) => {
                            let _ = ready_tx.send(Err(StorageError::Remote(format!(
                                "远端建连失败: {err}"
                            ))));
                            return;
                        }
                    };
                let _ = ready_tx.send(Ok(()));
                drop(ready_tx);
                bridge_loop(&runtime, rx, client);
            })
            .map_err(|err| StorageError::Remote(format!("桥线程创建失败: {err}")))?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                tx,
                handle,
                recv_timeout: rpc_timeout + RECV_MARGIN,
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
    fn call<T>(
        &self,
        make_job: impl FnOnce(mpsc::Sender<Result<T, StorageError>>) -> Job,
    ) -> Result<T, StorageError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .send(make_job(reply_tx))
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

    /// RPC 1：`StoreIDV2` 的同步门面。
    pub fn store_idv2(
        &self,
        id_or_row: impl Into<String>,
    ) -> Result<pb::StoreIdResponse, StorageError> {
        let id_or_row = id_or_row.into();
        self.call(|reply| Job::StoreIdv2 { id_or_row, reply })
    }

    /// RPC 2：`RetrieveRowByIDV2` 的同步门面。
    pub fn retrieve_row_by_idv2(
        &self,
        id_or_row: impl Into<String>,
    ) -> Result<pb::RetrieveRowByIdResponse, StorageError> {
        let id_or_row = id_or_row.into();
        self.call(|reply| Job::RetrieveRowByIdv2 { id_or_row, reply })
    }

    /// RPC 3：`WriteConfigV2` 的同步门面。
    pub fn write_config_v2(
        &self,
        section: impl Into<String>,
        subtype: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<pb::WriteConfigResponse, StorageError> {
        let section = section.into();
        let subtype = subtype.into();
        let value = value.into();
        self.call(|reply| Job::WriteConfigV2 {
            section,
            subtype,
            value,
            reply,
        })
    }

    /// RPC 4：`ReadConfigV2` 的同步门面。
    pub fn read_config_v2(
        &self,
        section: impl Into<String>,
        subtype: impl Into<String>,
    ) -> Result<pb::ReadConfigResponse, StorageError> {
        let section = section.into();
        let subtype = subtype.into();
        self.call(|reply| Job::ReadConfigV2 {
            section,
            subtype,
            reply,
        })
    }

    /// RPC 5：`UpdateVirtualValueV2` 的同步门面。
    pub fn update_virtual_value_v2(
        &self,
        old_virtual_value: i64,
        new_virtual_value: i64,
    ) -> Result<pb::UpdateVirtualValueResponse, StorageError> {
        self.call(|reply| Job::UpdateVirtualValueV2 {
            old_virtual_value,
            new_virtual_value,
            reply,
        })
    }

    /// RPC 6：`RetrieveRealValueV2` 的同步门面。
    pub fn retrieve_real_value_v2(
        &self,
        virtual_value: i64,
    ) -> Result<pb::RetrieveRealValueResponse, StorageError> {
        self.call(|reply| Job::RetrieveRealValueV2 {
            virtual_value,
            reply,
        })
    }

    /// RPC 7：`RetrieveRealValueV2Pro` 的同步门面。
    pub fn retrieve_real_value_v2_pro(
        &self,
        virtual_value: i64,
        virtual_value_sub: i64,
    ) -> Result<pb::RetrieveRealValueResponsePro, StorageError> {
        self.call(|reply| Job::RetrieveRealValueV2Pro {
            virtual_value,
            virtual_value_sub,
            reply,
        })
    }

    /// RPC 8：`RetrieveVirtualValueV2` 的同步门面。
    pub fn retrieve_virtual_value_v2(
        &self,
        real_value: impl Into<String>,
    ) -> Result<pb::RetrieveVirtualValueResponse, StorageError> {
        let real_value = real_value.into();
        self.call(|reply| Job::RetrieveVirtualValueV2 { real_value, reply })
    }

    /// RPC 9：`StoreIDV2Pro` 的同步门面。
    pub fn store_idv2_pro(
        &self,
        id_or_row: impl Into<String>,
        subid: impl Into<String>,
    ) -> Result<pb::StoreIdProResponse, StorageError> {
        let id_or_row = id_or_row.into();
        let subid = subid.into();
        self.call(|reply| Job::StoreIdv2Pro {
            id_or_row,
            subid,
            reply,
        })
    }

    /// RPC 10：`RetrieveRowByIDV2Pro` 的同步门面。
    pub fn retrieve_row_by_idv2_pro(
        &self,
        id_or_row: impl Into<String>,
        subid: impl Into<String>,
    ) -> Result<pb::RetrieveRowByIdProResponse, StorageError> {
        let id_or_row = id_or_row.into();
        let subid = subid.into();
        self.call(|reply| Job::RetrieveRowByIdv2Pro {
            id_or_row,
            subid,
            reply,
        })
    }

    /// RPC 11：`RetrieveVirtualValueV2Pro` 的同步门面。
    pub fn retrieve_virtual_value_v2_pro(
        &self,
        id_or_row: impl Into<String>,
        subid: impl Into<String>,
    ) -> Result<pb::RetrieveVirtualValueProResponse, StorageError> {
        let id_or_row = id_or_row.into();
        let subid = subid.into();
        self.call(|reply| Job::RetrieveVirtualValueV2Pro {
            id_or_row,
            subid,
            reply,
        })
    }

    /// RPC 12：`UpdateVirtualValueV2Pro` 的同步门面。
    pub fn update_virtual_value_v2_pro(
        &self,
        old_virtual_value_1: i64,
        new_virtual_value_1: i64,
        old_virtual_value_2: i64,
        new_virtual_value_2: i64,
    ) -> Result<pb::UpdateVirtualValueProResponse, StorageError> {
        self.call(|reply| Job::UpdateVirtualValueV2Pro {
            old_virtual_value_1,
            new_virtual_value_1,
            old_virtual_value_2,
            new_virtual_value_2,
            reply,
        })
    }

    /// RPC 13：`SimplifiedStoreIDV2` 的同步门面。
    pub fn simplified_store_idv2(
        &self,
        id_or_row: impl Into<String>,
    ) -> Result<pb::SimplifiedStoreIdResponse, StorageError> {
        let id_or_row = id_or_row.into();
        self.call(|reply| Job::SimplifiedStoreIdv2 { id_or_row, reply })
    }

    /// RPC 14：`FindSubKeysByIdPro` 的同步门面。
    pub fn find_sub_keys_by_id_pro(
        &self,
        id: impl Into<String>,
    ) -> Result<pb::FindSubKeysResponse, StorageError> {
        let id = id.into();
        self.call(|reply| Job::FindSubKeysByIdPro { id, reply })
    }

    /// RPC 15：`DeleteConfigV2` 的同步门面。
    pub fn delete_config_v2(
        &self,
        section: impl Into<String>,
        subtype: impl Into<String>,
    ) -> Result<pb::DeleteConfigResponse, StorageError> {
        let section = section.into();
        let subtype = subtype.into();
        self.call(|reply| Job::DeleteConfigV2 {
            section,
            subtype,
            reply,
        })
    }

    /// RPC 16：`StoreCacheV2` 的同步门面。
    pub fn store_cache_v2(
        &self,
        id_or_row: impl Into<String>,
    ) -> Result<pb::StoreCacheResponse, StorageError> {
        let id_or_row = id_or_row.into();
        self.call(|reply| Job::StoreCacheV2 { id_or_row, reply })
    }

    /// RPC 17：`RetrieveRowByCacheV2` 的同步门面。
    pub fn retrieve_row_by_cache_v2(
        &self,
        id_or_row: impl Into<String>,
    ) -> Result<pb::RetrieveRowByCacheResponse, StorageError> {
        let id_or_row = id_or_row.into();
        self.call(|reply| Job::RetrieveRowByCacheV2 { id_or_row, reply })
    }
}

/// idmap 远程模式（设计 §3.1）：只由三个配置开关决定；gRPC 服务端的启动是 boot 的事，
/// 不在本判定内。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdMapRemoteMode {
    /// 全本地：lotus 关闭，或 lotus+HTTP 且 `without_idmaps`。
    Local,
    /// gRPC 客户端：可远程方法经 [`RemoteBridge`] 走远端。
    GrpcClient,
    /// HTTP 远端 `/getid`（`lotus && !lotus_grpc && !without_idmaps`）。
    HttpRemote,
}

/// 按配置开关判定 idmap 远程模式（Go `idmap/service.go` 各 v2 函数的分支判定顺序）。
///
/// **Go 怪癖，照抄**：gRPC 分支不检查 `without_idmaps`——`lotus && lotus_grpc` 时即使
/// `without_idmaps=true` 仍返回 [`IdMapRemoteMode::GrpcClient`]（Go `service.go:676`
/// 只看 `GetLotusGrpc() && GetLotusValue()`）；`without_idmaps` 只挡 HTTP 分支。
/// 该组合的 warn 日志留给 boot 装配处。
#[must_use]
pub fn idmap_remote_mode(lotus: bool, lotus_grpc: bool, without_idmaps: bool) -> IdMapRemoteMode {
    if !lotus {
        return IdMapRemoteMode::Local;
    }
    if lotus_grpc {
        return IdMapRemoteMode::GrpcClient;
    }
    if without_idmaps {
        IdMapRemoteMode::Local
    } else {
        IdMapRemoteMode::HttpRemote
    }
}

/// 桥循环：std mpsc 的阻塞 recv 放到 blocking 池，current_thread runtime 空闲时
/// 仍能驱动 tonic 连接（重连/keepalive）；Job 到达后独占 `&mut client` 逐个 await。
fn bridge_loop(runtime: &tokio::runtime::Runtime, rx: mpsc::Receiver<Job>, mut client: IdMapClient) {
    runtime.block_on(async move {
        let mut rx = rx;
        loop {
            let received = tokio::task::spawn_blocking(move || {
                let job = rx.recv();
                (job, rx)
            })
            .await;
            let (job, returned) = match received {
                Ok(pair) => pair,
                Err(_) => break,
            };
            rx = returned;
            match job {
                Ok(job) => dispatch(&mut client, job).await,
                // 所有发送端已 drop：优雅退出
                Err(_) => break,
            }
        }
    });
}

/// 按 Job 分发到对应的 async 方法；回复发送失败（调用方已超时离开）静默丢弃。
async fn dispatch(client: &mut IdMapClient, job: Job) {
    match job {
        Job::StoreIdv2 { id_or_row, reply } => {
            let _ = reply.send(client.store_idv2(id_or_row).await.map_err(status_error));
        }
        Job::RetrieveRowByIdv2 { id_or_row, reply } => {
            let _ = reply.send(
                client
                    .retrieve_row_by_idv2(id_or_row)
                    .await
                    .map_err(status_error),
            );
        }
        Job::WriteConfigV2 {
            section,
            subtype,
            value,
            reply,
        } => {
            let _ = reply.send(
                client
                    .write_config_v2(section, subtype, value)
                    .await
                    .map_err(status_error),
            );
        }
        Job::ReadConfigV2 {
            section,
            subtype,
            reply,
        } => {
            let _ = reply.send(
                client
                    .read_config_v2(section, subtype)
                    .await
                    .map_err(status_error),
            );
        }
        Job::UpdateVirtualValueV2 {
            old_virtual_value,
            new_virtual_value,
            reply,
        } => {
            let _ = reply.send(
                client
                    .update_virtual_value_v2(old_virtual_value, new_virtual_value)
                    .await
                    .map_err(status_error),
            );
        }
        Job::RetrieveRealValueV2 {
            virtual_value,
            reply,
        } => {
            let _ = reply.send(
                client
                    .retrieve_real_value_v2(virtual_value)
                    .await
                    .map_err(status_error),
            );
        }
        Job::RetrieveRealValueV2Pro {
            virtual_value,
            virtual_value_sub,
            reply,
        } => {
            let _ = reply.send(
                client
                    .retrieve_real_value_v2_pro(virtual_value, virtual_value_sub)
                    .await
                    .map_err(status_error),
            );
        }
        Job::RetrieveVirtualValueV2 { real_value, reply } => {
            let _ = reply.send(
                client
                    .retrieve_virtual_value_v2(real_value)
                    .await
                    .map_err(status_error),
            );
        }
        Job::StoreIdv2Pro {
            id_or_row,
            subid,
            reply,
        } => {
            let _ = reply.send(
                client
                    .store_idv2_pro(id_or_row, subid)
                    .await
                    .map_err(status_error),
            );
        }
        Job::RetrieveRowByIdv2Pro {
            id_or_row,
            subid,
            reply,
        } => {
            let _ = reply.send(
                client
                    .retrieve_row_by_idv2_pro(id_or_row, subid)
                    .await
                    .map_err(status_error),
            );
        }
        Job::RetrieveVirtualValueV2Pro {
            id_or_row,
            subid,
            reply,
        } => {
            let _ = reply.send(
                client
                    .retrieve_virtual_value_v2_pro(id_or_row, subid)
                    .await
                    .map_err(status_error),
            );
        }
        Job::UpdateVirtualValueV2Pro {
            old_virtual_value_1,
            new_virtual_value_1,
            old_virtual_value_2,
            new_virtual_value_2,
            reply,
        } => {
            let _ = reply.send(
                client
                    .update_virtual_value_v2_pro(
                        old_virtual_value_1,
                        new_virtual_value_1,
                        old_virtual_value_2,
                        new_virtual_value_2,
                    )
                    .await
                    .map_err(status_error),
            );
        }
        Job::SimplifiedStoreIdv2 { id_or_row, reply } => {
            let _ = reply.send(
                client
                    .simplified_store_idv2(id_or_row)
                    .await
                    .map_err(status_error),
            );
        }
        Job::FindSubKeysByIdPro { id, reply } => {
            let _ = reply.send(
                client
                    .find_sub_keys_by_id_pro(id)
                    .await
                    .map_err(status_error),
            );
        }
        Job::DeleteConfigV2 {
            section,
            subtype,
            reply,
        } => {
            let _ = reply.send(
                client
                    .delete_config_v2(section, subtype)
                    .await
                    .map_err(status_error),
            );
        }
        Job::StoreCacheV2 { id_or_row, reply } => {
            let _ = reply.send(client.store_cache_v2(id_or_row).await.map_err(status_error));
        }
        Job::RetrieveRowByCacheV2 { id_or_row, reply } => {
            let _ = reply.send(
                client
                    .retrieve_row_by_cache_v2(id_or_row)
                    .await
                    .map_err(status_error),
            );
        }
    }
}

/// gRPC Status → 桥错误：保留 code 与 message，调用点按既有 Err 分支回退。
fn status_error(status: tonic::Status) -> StorageError {
    StorageError::Remote(format!("gRPC {:?}: {}", status.code(), status.message()))
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::sync::Arc;
    use std::time::Duration;

    use tokio_stream::wrappers::TcpListenerStream;
    use tonic::transport::{Endpoint, Server};

    use super::{IdMapRemoteMode, Job, RemoteBridge, idmap_remote_mode};
    use crate::error::StorageError;
    use crate::idmap::grpc::tests::{Harness, harness};
    use crate::idmap::grpc::{IdMapService, pb};
    use crate::idmap::{IdMapOptions, IdMapStore};

    /// 起真服务端（复用 grpc.rs 的 Harness）+ 建桥。
    async fn bridge_harness() -> Result<(Harness, RemoteBridge), Box<dyn Error>> {
        let harness = harness().await?;
        let endpoint = Endpoint::from_shared(format!("http://{}", harness.addr))?;
        let bridge = RemoteBridge::spawn(endpoint, Duration::from_secs(30))?;
        Ok((harness, bridge))
    }

    /// 验收 a：17 个同步门面各调一次，环回真服务端全通。
    #[tokio::test(flavor = "multi_thread")]
    async fn seventeen_facades_round_trip() -> Result<(), Box<dyn Error>> {
        let (h, bridge) = bridge_harness().await?;

        // RPC 1/2
        let row = bridge.store_idv2("openid-user-1")?.row;
        assert!(row > 0);
        assert_eq!(
            bridge.retrieve_row_by_idv2(row.to_string())?.id,
            "openid-user-1"
        );
        // RPC 3/4/15
        assert_eq!(
            bridge.write_config_v2("lotus", "token", "abc")?.status,
            "success"
        );
        assert_eq!(bridge.read_config_v2("lotus", "token")?.value, "abc");
        assert_eq!(h.store.read_config("lotus", "token")?, "abc");
        bridge.delete_config_v2("lotus", "token")?;
        assert!(bridge.read_config_v2("lotus", "token").is_err());
        // RPC 8
        let virtual_value = bridge.retrieve_virtual_value_v2("openid-user-1")?;
        assert_eq!(virtual_value.real, "openid-user-1");
        assert_eq!(virtual_value.r#virtual, row.to_string());
        // RPC 6
        let real = bridge.retrieve_real_value_v2(row)?;
        assert_eq!(real.r#virtual, row.to_string());
        assert_eq!(real.real, "openid-user-1");
        // RPC 9/10/11
        let pro = bridge.store_idv2_pro("openid-group-1", "openid-user-1")?;
        assert!(pro.row > 0);
        assert_eq!(pro.sub_row, row);
        let pro_back = bridge.retrieve_row_by_idv2_pro(pro.row.to_string(), row.to_string())?;
        assert_eq!(pro_back.id, "openid-group-1");
        assert_eq!(pro_back.subid, "openid-user-1");
        let pro_values = bridge.retrieve_virtual_value_v2_pro("openid-group-1", "openid-user-1")?;
        assert_eq!(pro_values.first_value, pro.row.to_string());
        assert_eq!(pro_values.second_value, row.to_string());
        // RPC 7：服务端回两个身份串（virtual=第一个、real=第二个，见 grpc.rs 实现）
        let real_pro = bridge.retrieve_real_value_v2_pro(pro.row, row)?;
        assert_eq!(real_pro.r#virtual, "openid-group-1");
        assert_eq!(real_pro.real, "openid-user-1");
        // RPC 5/12
        let second = bridge.store_idv2("openid-user-2")?.row;
        let first_new = row + 100;
        assert_eq!(
            bridge.update_virtual_value_v2(row, first_new)?.status,
            "success"
        );
        assert_eq!(h.store.lookup_virtual_value("openid-user-1")?, first_new);
        let second_new = second + 100;
        let pro_update = bridge.update_virtual_value_v2_pro(
            second,
            second_new,
            first_new,
            first_new + 100,
        )?;
        assert_eq!(pro_update.message, "Virtual values updated successfully");
        assert_eq!(h.store.lookup_virtual_value("openid-user-2")?, second_new);
        // RPC 13
        let simplified = bridge.simplified_store_idv2("openid-user-3")?;
        assert!(simplified.row > 0);
        // RPC 14：现代 schema 恒空（与 grpc.rs 测试同口径）
        assert!(
            bridge
                .find_sub_keys_by_id_pro("openid-user-1")?
                .keys
                .is_empty()
        );
        // RPC 16/17
        let cache_row = bridge.store_cache_v2("real-msg-1")?.row;
        assert_eq!(
            bridge.retrieve_row_by_cache_v2(cache_row.to_string())?.id,
            "real-msg-1"
        );
        assert!(bridge.is_alive());
        Ok(())
    }

    /// 验收 b：current_thread flavor 的 async 上下文里调用门面不 panic
    ///（最严格的 flavor：block_in_place 在这里会直接炸，本桥不触碰调用线程 runtime）。
    /// 门面会阻塞调用方唯一的 worker，进程内服务端必须跑在独立 runtime 上才不会
    /// 饿死（生产里服务端是远端进程，不存在此问题）。
    #[tokio::test]
    async fn facades_safe_in_current_thread_async_context() -> Result<(), Box<dyn Error>> {
        let (addr_tx, addr_rx) = std::sync::mpsc::channel();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let server = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                let dir = tempfile::tempdir().unwrap();
                let store = Arc::new(
                    IdMapStore::open(
                        IdMapOptions::new(
                            dir.path().join("openid-map.redb"),
                            dir.path().join("msgid-map.redb"),
                        )
                        .with_app_id("10000001"),
                    )
                    .unwrap(),
                );
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let addr = listener.local_addr().unwrap();
                let service = pb::id_map_service_server::IdMapServiceServer::new(
                    IdMapService::new(store),
                );
                addr_tx.send(addr).unwrap();
                let incoming = TcpListenerStream::new(listener);
                let _ = Server::builder()
                    .add_service(service)
                    .serve_with_incoming_shutdown(incoming, async {
                        let _ = shutdown_rx.await;
                    })
                    .await;
            });
        });
        let addr = addr_rx.recv()?;
        let endpoint = Endpoint::from_shared(format!("http://{addr}"))?;
        let bridge = RemoteBridge::spawn(endpoint, Duration::from_secs(30))?;
        let row = bridge.store_idv2("openid-async")?.row;
        assert_eq!(
            bridge.retrieve_row_by_idv2(row.to_string())?.id,
            "openid-async"
        );
        drop(bridge);
        let _ = shutdown_tx.send(());
        server.join().unwrap();
        Ok(())
    }

    /// 验收 c-1：极短 rpc_timeout → 门面回 Remote 错误，桥本身不死。
    #[tokio::test(flavor = "multi_thread")]
    async fn rpc_timeout_returns_err() -> Result<(), Box<dyn Error>> {
        let (_h, bridge) = {
            let harness = harness().await?;
            let endpoint = Endpoint::from_shared(format!("http://{}", harness.addr))?;
            let bridge = RemoteBridge::spawn(endpoint, Duration::from_nanos(1))?;
            (harness, bridge)
        };
        let err = bridge.store_idv2("openid-timeout").unwrap_err();
        assert!(matches!(err, StorageError::Remote(_)), "{err:?}");
        assert!(bridge.is_alive());
        Ok(())
    }

    /// 验收 c-2：不可达端口 → spawn 建连失败返回 Err（调用方决定降级）。
    #[test]
    fn spawn_unreachable_endpoint_returns_err() {
        // 先占端口再释放，保证"连接被拒"而不是被防火墙静默丢包。
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let endpoint = Endpoint::from_shared(format!("http://{addr}"))
            .unwrap()
            .connect_timeout(Duration::from_secs(2));
        let err = RemoteBridge::spawn(endpoint, Duration::from_secs(1)).unwrap_err();
        assert!(matches!(err, StorageError::Remote(_)), "{err:?}");
    }

    /// 验收 c-3：桥死亡 → is_alive=false，门面回 Remote 错误。
    #[tokio::test(flavor = "multi_thread")]
    async fn dead_bridge_facades_return_err() -> Result<(), Box<dyn Error>> {
        let (_h, bridge) = bridge_harness().await?;
        // 拆掉真实发送端 → 桥线程 recv 断开后自行退出；再换一个接收端已 drop 的
        // 发送端，走"桥已死"时门面的 send 失败路径（同模块可访问私有字段）。
        let RemoteBridge {
            tx,
            handle,
            recv_timeout,
        } = bridge;
        drop(tx);
        // 等桥线程收尾（不消耗 handle，join 会 move 它）；发送端全断后必然退出。
        while !handle.is_finished() {
            std::thread::yield_now();
        }
        let (dead_tx, dead_rx) = std::sync::mpsc::channel::<Job>();
        drop(dead_rx);
        let bridge = RemoteBridge {
            tx: dead_tx,
            handle,
            recv_timeout,
        };
        assert!(!bridge.is_alive());
        let err = bridge.store_idv2("openid-dead").unwrap_err();
        assert!(matches!(err, StorageError::Remote(_)), "{err:?}");
        Ok(())
    }

    /// 验收 P2-a：模式矩阵全 8 种开关组合，含 Go 怪癖行。
    #[test]
    fn remote_mode_matrix_all_eight_combinations() {
        assert_eq!(idmap_remote_mode(false, false, false), IdMapRemoteMode::Local);
        assert_eq!(idmap_remote_mode(false, false, true), IdMapRemoteMode::Local);
        assert_eq!(idmap_remote_mode(false, true, false), IdMapRemoteMode::Local);
        assert_eq!(idmap_remote_mode(false, true, true), IdMapRemoteMode::Local);
        assert_eq!(
            idmap_remote_mode(true, false, false),
            IdMapRemoteMode::HttpRemote
        );
        assert_eq!(idmap_remote_mode(true, false, true), IdMapRemoteMode::Local);
        assert_eq!(
            idmap_remote_mode(true, true, false),
            IdMapRemoteMode::GrpcClient
        );
        // Go 怪癖：gRPC 分支不查 without_idmaps（service.go:676），照抄。
        assert_eq!(
            idmap_remote_mode(true, true, true),
            IdMapRemoteMode::GrpcClient
        );
    }

    /// 验收 P2-b：with_remote 后 11 个可远程方法（覆盖 17 RPC 语义）全走远端，
    /// 本地 redb 一行不写；本地-only 方法不受影响。
    #[tokio::test(flavor = "multi_thread")]
    async fn with_remote_routes_to_server_and_never_writes_local() -> Result<(), Box<dyn Error>> {
        let (h, bridge) = bridge_harness().await?;
        let dir = tempfile::tempdir()?;
        let store = IdMapStore::open(
            IdMapOptions::new(
                dir.path().join("openid-map.redb"),
                dir.path().join("msgid-map.redb"),
            )
            .with_app_id("10000001"),
        )?
        .with_remote(bridge);

        // RPC 1（StoreIDv2）：分配在远端
        let (raw, row) = store.retrieve_virtual_value("openid-user-1")?;
        assert_eq!(raw, "openid-user-1");
        assert!(row > 0);
        assert_eq!(h.store.lookup_virtual_value("openid-user-1")?, row);
        // RPC 2（RetrieveRowByIDv2）
        assert_eq!(store.retrieve_row_by_id(row)?, "openid-user-1");
        // RPC 8（RetrieveVirtualValueV2）：只查不分配
        assert_eq!(store.lookup_virtual_value("openid-user-1")?, row);
        assert!(store.lookup_virtual_value("openid-unknown").is_err());
        // RPC 3/4/15（配置写/读/删）
        store.write_config("lotus", "token", "abc")?;
        assert_eq!(store.read_config("lotus", "token")?, "abc");
        assert_eq!(h.store.read_config("lotus", "token")?, "abc");
        assert!(store.delete_config("lotus", "token")?);
        assert!(store.read_config("lotus", "token").is_err());
        // RPC 5（UpdateVirtualValueV2）
        let new_row = row + 100;
        store.update_virtual_value(row, new_row)?;
        assert_eq!(h.store.lookup_virtual_value("openid-user-1")?, new_row);
        // RPC 12（UpdateVirtualValueV2Pro）：单次 RPC
        let (_, row2) = store.retrieve_virtual_value("openid-user-2")?;
        store.update_virtual_value_pro(row2, row2 + 100, new_row, new_row + 100)?;
        assert_eq!(h.store.lookup_virtual_value("openid-user-2")?, row2 + 100);
        assert_eq!(h.store.lookup_virtual_value("openid-user-1")?, new_row + 100);
        // RPC 14（FindSubKeysByIdPro）：现代 schema 恒空
        assert!(store.find_sub_keys_by_id("openid-user-1")?.is_empty());
        // RPC 16/17（StoreCacheV2 / RetrieveRowByCacheV2）
        let cache_row = store.store_cache("real-msg-1")?;
        assert_eq!(store.retrieve_row_by_cache(cache_row)?, "real-msg-1");
        assert_eq!(h.store.retrieve_row_by_cache(cache_row)?, "real-msg-1");
        // store_cache_with_ttl 同样走远程（TTL 由服务端决定）
        let cache_row2 = store.store_cache_with_ttl("real-msg-2", Duration::from_secs(60))?;
        assert_eq!(h.store.retrieve_row_by_cache(cache_row2)?, "real-msg-2");

        // 核心断言：远程全命中后本地 redb 一行未写（is_empty 查全部数据表）
        assert!(store.is_empty()?);

        // 本地-only 方法不受影响（Go 同样无远程分支）
        store.store_user_name(row, "Alice");
        assert_eq!(store.get_user_name(row).as_deref(), Some("Alice"));
        store.store_latest_msg_id("group-1", "user-1", "msg-1")?;
        assert_eq!(store.get_latest_msg_id("group-1", "user-1")?, "msg-1");
        assert!(store.list_identities()?.is_empty());
        assert_eq!(store.last_seen(row)?, None);
        assert!(store.find_config_sections("token", "abc")?.is_empty());
        assert_eq!(store.force_unbind("openid-nobody")?, 0);
        Ok(())
    }

    /// 验收 P2-b'：远程错误直接上抛 Remote，不做运行时本地回退（设计 §4，避免双写分裂）。
    #[tokio::test(flavor = "multi_thread")]
    async fn with_remote_error_does_not_fall_back_to_local() -> Result<(), Box<dyn Error>> {
        let harness = harness().await?;
        let endpoint = Endpoint::from_shared(format!("http://{}", harness.addr))?;
        let bridge = RemoteBridge::spawn(endpoint, Duration::from_nanos(1))?;
        let dir = tempfile::tempdir()?;
        let store = IdMapStore::open(
            IdMapOptions::new(
                dir.path().join("openid-map.redb"),
                dir.path().join("msgid-map.redb"),
            )
            .with_app_id("10000001"),
        )?
        .with_remote(bridge);
        let err = store.retrieve_virtual_value("openid-timeout").unwrap_err();
        assert!(matches!(err, StorageError::Remote(_)), "{err:?}");
        // 没有回退本地分配
        assert!(store.is_empty()?);
        Ok(())
    }

    /// 验收 P2-d：current_thread flavor 的 async 上下文里经 with_remote 调用不 panic。
    /// 门面阻塞调用方唯一 worker，进程内服务端必须跑在独立 runtime 上才不会饿死
    /// （生产里服务端是远端进程，不存在此问题）。
    #[tokio::test]
    async fn with_remote_safe_in_current_thread_async_context() -> Result<(), Box<dyn Error>> {
        let (addr_tx, addr_rx) = std::sync::mpsc::channel();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let server = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                let dir = tempfile::tempdir().unwrap();
                let store = Arc::new(
                    IdMapStore::open(
                        IdMapOptions::new(
                            dir.path().join("openid-map.redb"),
                            dir.path().join("msgid-map.redb"),
                        )
                        .with_app_id("10000001"),
                    )
                    .unwrap(),
                );
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let addr = listener.local_addr().unwrap();
                let service = pb::id_map_service_server::IdMapServiceServer::new(
                    IdMapService::new(store),
                );
                addr_tx.send(addr).unwrap();
                let incoming = TcpListenerStream::new(listener);
                let _ = Server::builder()
                    .add_service(service)
                    .serve_with_incoming_shutdown(incoming, async {
                        let _ = shutdown_rx.await;
                    })
                    .await;
            });
        });
        let addr = addr_rx.recv()?;
        let endpoint = Endpoint::from_shared(format!("http://{addr}"))?;
        let bridge = RemoteBridge::spawn(endpoint, Duration::from_secs(30))?;
        let dir = tempfile::tempdir()?;
        let store = IdMapStore::open(
            IdMapOptions::new(
                dir.path().join("openid-map.redb"),
                dir.path().join("msgid-map.redb"),
            )
            .with_app_id("10000001"),
        )?
        .with_remote(bridge);
        let (raw, row) = store.retrieve_virtual_value("openid-async")?;
        assert_eq!(raw, "openid-async");
        assert_eq!(store.retrieve_row_by_id(row)?, "openid-async");
        assert_eq!(store.lookup_virtual_value("openid-async")?, row);
        let cache_row = store.store_cache("msg-async")?;
        assert_eq!(store.retrieve_row_by_cache(cache_row)?, "msg-async");
        assert!(store.is_empty()?);
        drop(store);
        let _ = shutdown_tx.send(());
        server.join().unwrap();
        Ok(())
    }
}
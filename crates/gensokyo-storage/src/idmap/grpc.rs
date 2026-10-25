//! IDMapService 的 tonic 适配层：17 个 RPC 的服务端与客户端。
//!
//! 线上契约来自仓库根的 `proto/idmap.proto`（与 Go 共享同一份文件）：**service 全名、
//! 消息名与字段号都不可改**，否则 Go 侧 `proto/idmap.pb.go` 生成的客户端会立刻失配。
//! 该 proto 没有 `package` 声明，prost 因此把生成文件命名为 `_.rs`（见 [`pb`]）。
//!
//! # RPC 映射
//!
//! 逐条对齐 Go `idmap/grpc.go`（服务端）+ `idmap/service.go`（语义）：
//!
//! | RPC | 本 crate 的落点 |
//! |-----|------------------|
//! | `StoreIDV2` / `SimplifiedStoreIDV2` | [`IdMapStore::retrieve_virtual_value`]（Go `storeIdentity`：查不到就分配） |
//! | `RetrieveRowByIDV2` | [`IdMapStore::retrieve_row_by_id`]（请求里的 `id_or_row` 按十进制 vUIN 解析） |
//! | `RetrieveRealValueV2` | 同上；响应 `virtual` 回填请求里的十进制 vUIN、`real` 是身份串 |
//! | `WriteConfigV2` / `ReadConfigV2` / `DeleteConfigV2` | [`IdMapStore::write_config`] / [`IdMapStore::read_config`] / [`IdMapStore::delete_config`]（身份库 `config` 表，key `"<section>:<subtype>"`） |
//! | `RetrieveVirtualValueV2` | [`IdMapStore::lookup_virtual_value`]（Go `RetrieveVirtualValue`：**只查不分配**，查不到即 `NotFound`；响应 `real` 回填入参原串、`virtual` 是十进制 vUIN） |
//! | `StoreIDV2Pro` | 两次 [`IdMapStore::retrieve_virtual_value`]（Go 的 Pro 就是两次独立 `StoreIDv2`，v2 路径没有复合键） |
//! | `RetrieveRowByIDV2Pro` / `RetrieveRealValueV2Pro` | 两次 [`IdMapStore::retrieve_row_by_id`] |
//! | `RetrieveVirtualValueV2Pro` | 两次 [`IdMapStore::lookup_virtual_value`]，只回两个 vUIN |
//! | `StoreCacheV2` / `RetrieveRowByCacheV2` | [`IdMapStore::store_cache`] / [`IdMapStore::retrieve_row_by_cache`] |
//! | `UpdateVirtualValueV2` / `UpdateVirtualValueV2Pro` | **未实现** → `Status::unimplemented`（见下） |
//! | `FindSubKeysByIdPro` | **未实现** → `Status::unimplemented`（见下） |
//!
//! # 未做的部分（明确列出）
//!
//! - `UpdateVirtualValueV2` / `UpdateVirtualValueV2Pro`：需要 `IdMapStore` 的 rebind 能力。
//!   Go `UpdateVirtualValue`（`idmap/service.go:1404`）在隔离模式下写的是"新 vUIN → 旧 vUIN"
//!   的 alias，非隔离模式下还要处理 `newRowValue == 0`（彻底解绑，扫删所有指向同一身份的
//!   反查条目）。redb 身份库的反查 key 是 `"<vUIN>:<identity_key>"`，与 Go 的 `uin:row-<N>`
//!   不同构，必须先把这套语义搬进 store 才谈得上接线，本里程碑不猜实现。
//! - `FindSubKeysByIdPro`：Go 读的是**已退役** bbolt `ids` 桶里的 `id:sub` 复合键
//!   （`idmap/service.go:2112`），redb 身份库没有等价表结构。
//! - TLS/mTLS 与鉴权：不做。与 Go 侧一致（明文 h2c，内网/反向代理兜底）；监听地址、连接数、
//!   超时由调用方用 `tonic::transport::Server` / `Endpoint` 自行配置，本模块不接管生命周期。
//!
//! # 与 Go 的差异
//!
//! - **错误码**：Go handler 直接 `return nil, err`（gRPC 侧落到 `Unknown`）；本 crate 按
//!   [`StorageError`] 分类映射为 `NotFound` / `InvalidArgument` / `Internal`。
//! - **非数字行号**：Go `RetrieveRowByIDv2` 对解析失败回 `ErrKeyNotFound`；本 crate 回
//!   `InvalidArgument`，调用方传错类型时更容易定位。
//! - **`RetrieveRealValueV2Pro`**：Go 的**服务端** handler 调的是遗留 `RetrieveRealValuePro`
//!   （读退役 `ids` 桶的 `"<v1>:<v2>"` 复合键），而 Go 的**客户端**封装走
//!   `RetrieveRealValuesv2Pro`（两次独立身份反查）。本 crate 对齐后者——前者在 redb 布局里
//!   没有落点。
//! - **阻塞**：每个 handler 在 tonic 的工作线程上同步跑一次 redb 事务（微秒级），与 Go 在
//!   handler 内同步调 bbolt 同构，因此不做 `spawn_blocking`。

use std::sync::Arc;

use tonic::{Request, Response, Status};

use super::IdMapStore;
use crate::StorageError;

/// `build.rs` 生成的 protobuf 类型与 tonic 客户端/服务端桩。
///
/// 生成文件是 `OUT_DIR/_.rs`（proto 无 `package`，见模块文档）。生成代码不保证通过
/// 仓库的 clippy 门禁，因此整体关掉 lint。
pub mod pb {
    #![allow(clippy::all, missing_docs, rust_2018_idioms)]

    include!(concat!(env!("OUT_DIR"), "/_.rs"));
}

/// `IDMapService` 服务端：把 proto 请求映射到 [`IdMapStore`]。
///
/// 每个处理器都在调用方线程上同步执行一次 redb 事务（微秒级，见模块文档的"阻塞"一条），
/// 因此可以直接 `Arc` 共享给多个连接。
#[derive(Debug, Clone)]
pub struct IdMapService {
    store: Arc<IdMapStore>,
}

impl IdMapService {
    /// 用共享的 [`IdMapStore`] 构造服务。
    #[must_use]
    pub fn new(store: Arc<IdMapStore>) -> Self {
        Self { store }
    }

    /// 共享存储（同进程复用 / 测试断言用）。
    #[must_use]
    pub fn store(&self) -> &Arc<IdMapStore> {
        &self.store
    }

    /// Go `storeIdentity`：查身份对应的 vUIN，查不到则分配。
    fn allocate(&self, identity: &str) -> Result<i64, Status> {
        self.store
            .retrieve_virtual_value(identity)
            .map(|(_, vuin)| vuin)
            .map_err(status)
    }

    /// Go `retrieveIdentity`：vUIN 反查身份串。
    fn identity_of(&self, vuin: i64) -> Result<String, Status> {
        self.store.retrieve_row_by_id(vuin).map_err(status)
    }
}

#[tonic::async_trait]
impl pb::id_map_service_server::IdMapService for IdMapService {
    async fn store_idv2(
        &self,
        request: Request<pb::StoreIdRequest>,
    ) -> Result<Response<pb::StoreIdResponse>, Status> {
        let request = request.into_inner();
        let row = self.allocate(&request.id_or_row)?;
        Ok(Response::new(pb::StoreIdResponse { row }))
    }

    async fn retrieve_row_by_idv2(
        &self,
        request: Request<pb::RetrieveRowByIdRequest>,
    ) -> Result<Response<pb::RetrieveRowByIdResponse>, Status> {
        let request = request.into_inner();
        let id = self.identity_of(parse_vuin(&request.id_or_row)?)?;
        Ok(Response::new(pb::RetrieveRowByIdResponse { id }))
    }

    async fn write_config_v2(
        &self,
        request: Request<pb::WriteConfigRequest>,
    ) -> Result<Response<pb::WriteConfigResponse>, Status> {
        let request = request.into_inner();
        self.store
            .write_config(&request.section, &request.subtype, &request.value)
            .map_err(status)?;
        Ok(Response::new(pb::WriteConfigResponse {
            status: "success".to_owned(),
        }))
    }

    async fn read_config_v2(
        &self,
        request: Request<pb::ReadConfigRequest>,
    ) -> Result<Response<pb::ReadConfigResponse>, Status> {
        let request = request.into_inner();
        let value = self
            .store
            .read_config(&request.section, &request.subtype)
            .map_err(status)?;
        Ok(Response::new(pb::ReadConfigResponse { value }))
    }

    async fn update_virtual_value_v2(
        &self,
        _request: Request<pb::UpdateVirtualValueRequest>,
    ) -> Result<Response<pb::UpdateVirtualValueResponse>, Status> {
        Err(Status::unimplemented(REBIND_UNIMPLEMENTED))
    }

    async fn retrieve_real_value_v2(
        &self,
        request: Request<pb::RetrieveRealValueRequest>,
    ) -> Result<Response<pb::RetrieveRealValueResponse>, Status> {
        let request = request.into_inner();
        let real = self.identity_of(request.virtual_value)?;
        Ok(Response::new(pb::RetrieveRealValueResponse {
            r#virtual: request.virtual_value.to_string(),
            real,
        }))
    }

    async fn retrieve_real_value_v2_pro(
        &self,
        request: Request<pb::RetrieveRealValueRequestPro>,
    ) -> Result<Response<pb::RetrieveRealValueResponsePro>, Status> {
        let request = request.into_inner();
        let first = self.identity_of(request.virtual_value)?;
        let second = self.identity_of(request.virtual_value_sub)?;
        Ok(Response::new(pb::RetrieveRealValueResponsePro {
            r#virtual: first,
            real: second,
        }))
    }

    async fn retrieve_virtual_value_v2(
        &self,
        request: Request<pb::RetrieveVirtualValueRequest>,
    ) -> Result<Response<pb::RetrieveVirtualValueResponse>, Status> {
        let request = request.into_inner();
        // Go `RetrieveVirtualValuev2` → `RetrieveVirtualValue`：只查不分配，查不到就是错误；
        // 响应里的 `real` 回填调用方传进来的原始串（Go 同样回 `realValue`，不做归一化）。
        let vuin = self
            .store
            .lookup_virtual_value(&request.real_value)
            .map_err(status)?;
        Ok(Response::new(pb::RetrieveVirtualValueResponse {
            real: request.real_value,
            r#virtual: vuin.to_string(),
        }))
    }

    async fn store_idv2_pro(
        &self,
        request: Request<pb::StoreIdProRequest>,
    ) -> Result<Response<pb::StoreIdProResponse>, Status> {
        let request = request.into_inner();
        let row = self.allocate(&request.id_or_row)?;
        let sub_row = self.allocate(&request.subid)?;
        Ok(Response::new(pb::StoreIdProResponse { row, sub_row }))
    }

    async fn retrieve_row_by_idv2_pro(
        &self,
        request: Request<pb::RetrieveRowByIdProRequest>,
    ) -> Result<Response<pb::RetrieveRowByIdProResponse>, Status> {
        let request = request.into_inner();
        let id = self.identity_of(parse_vuin(&request.id_or_row)?)?;
        let subid = self.identity_of(parse_vuin(&request.subid)?)?;
        Ok(Response::new(pb::RetrieveRowByIdProResponse { id, subid }))
    }

    async fn retrieve_virtual_value_v2_pro(
        &self,
        request: Request<pb::RetrieveVirtualValueProRequest>,
    ) -> Result<Response<pb::RetrieveVirtualValueProResponse>, Status> {
        let request = request.into_inner();
        let first_value = self
            .store
            .lookup_virtual_value(&request.id_or_row)
            .map_err(status)?;
        let second_value = self
            .store
            .lookup_virtual_value(&request.subid)
            .map_err(status)?;
        Ok(Response::new(pb::RetrieveVirtualValueProResponse {
            first_value: first_value.to_string(),
            second_value: second_value.to_string(),
        }))
    }

    async fn update_virtual_value_v2_pro(
        &self,
        _request: Request<pb::UpdateVirtualValueProRequest>,
    ) -> Result<Response<pb::UpdateVirtualValueProResponse>, Status> {
        Err(Status::unimplemented(REBIND_UNIMPLEMENTED))
    }

    async fn simplified_store_idv2(
        &self,
        request: Request<pb::SimplifiedStoreIdRequest>,
    ) -> Result<Response<pb::SimplifiedStoreIdResponse>, Status> {
        let request = request.into_inner();
        let row = self.allocate(&request.id_or_row)?;
        Ok(Response::new(pb::SimplifiedStoreIdResponse { row }))
    }

    async fn find_sub_keys_by_id_pro(
        &self,
        _request: Request<pb::FindSubKeysRequest>,
    ) -> Result<Response<pb::FindSubKeysResponse>, Status> {
        Err(Status::unimplemented(SUB_KEYS_UNIMPLEMENTED))
    }

    async fn delete_config_v2(
        &self,
        request: Request<pb::DeleteConfigRequest>,
    ) -> Result<Response<pb::DeleteConfigResponse>, Status> {
        let request = request.into_inner();
        self.store
            .delete_config(&request.section, &request.subtype)
            .map_err(status)?;
        Ok(Response::new(pb::DeleteConfigResponse {
            status: "success".to_owned(),
        }))
    }

    async fn store_cache_v2(
        &self,
        request: Request<pb::StoreCacheRequest>,
    ) -> Result<Response<pb::StoreCacheResponse>, Status> {
        let request = request.into_inner();
        let row = self.store.store_cache(&request.id_or_row).map_err(status)?;
        Ok(Response::new(pb::StoreCacheResponse { row }))
    }

    async fn retrieve_row_by_cache_v2(
        &self,
        request: Request<pb::RetrieveRowByCacheRequest>,
    ) -> Result<Response<pb::RetrieveRowByCacheResponse>, Status> {
        let request = request.into_inner();
        let id = self
            .store
            .retrieve_row_by_cache(parse_vuin(&request.id_or_row)?)
            .map_err(status)?;
        Ok(Response::new(pb::RetrieveRowByCacheResponse { id }))
    }
}

/// `UpdateVirtualValueV2` / `UpdateVirtualValueV2Pro` 的拒绝理由。
const REBIND_UNIMPLEMENTED: &str = "UpdateVirtualValueV2 未实现：IdMapStore 尚无 rebind —— \
     Go UpdateVirtualValue（idmap/service.go:1404）的隔离 alias 与 newRowValue==0 解绑语义、\
     以及 redb 反查 key 布局（\"<vUIN>:<identity_key>\"）尚未搬进 store";

/// `FindSubKeysByIdPro` 的拒绝理由。
const SUB_KEYS_UNIMPLEMENTED: &str = "FindSubKeysByIdPro 未实现：Go 读的是已退役 bbolt `ids` \
     桶的 `id:sub` 复合键（idmap/service.go:2112），redb 身份库没有等价表结构";

/// `IDMapService` 客户端：对 tonic 生成的 `IdMapServiceClient` 的薄封装。
///
/// 只做"建连接 + 17 个方法转发"，不吞 `tonic::Status`；重试、超时、TLS 由调用方通过
/// [`tonic::transport::Endpoint`] 决定（本里程碑不做 TLS/mTLS 与鉴权）。
#[derive(Debug, Clone)]
pub struct IdMapClient {
    inner: pb::id_map_service_client::IdMapServiceClient<tonic::transport::Channel>,
}

impl IdMapClient {
    /// 按端点建连（明文 h2c，同 Go `grpc.Dial(..., insecure.NewCredentials())`）。
    ///
    /// # Errors
    ///
    /// 传输层建连失败时返回 [`tonic::transport::Error`]。
    pub async fn connect(
        endpoint: tonic::transport::Endpoint,
    ) -> Result<Self, tonic::transport::Error> {
        Ok(Self {
            inner: pb::id_map_service_client::IdMapServiceClient::connect(endpoint).await?,
        })
    }

    /// 包装已有 [`tonic::transport::Channel`]（共享连接或自定义 connector）。
    #[must_use]
    pub fn from_channel(channel: tonic::transport::Channel) -> Self {
        Self {
            inner: pb::id_map_service_client::IdMapServiceClient::new(channel),
        }
    }

    /// 取回生成的客户端（需要 `with_origin` / 拦截器 / 裸 `tonic::Request` 时用）。
    #[must_use]
    pub fn inner(
        &self,
    ) -> &pb::id_map_service_client::IdMapServiceClient<tonic::transport::Channel> {
        &self.inner
    }

    /// 取回可变的生成客户端（可挂拦截器）。
    pub fn inner_mut(
        &mut self,
    ) -> &mut pb::id_map_service_client::IdMapServiceClient<tonic::transport::Channel> {
        &mut self.inner
    }

    /// RPC 1：`StoreIDV2`。
    ///
    /// # Errors
    ///
    /// 服务端拒绝或传输失败时返回 [`Status`]。
    pub async fn store_idv2(
        &mut self,
        id_or_row: impl Into<String>,
    ) -> Result<pb::StoreIdResponse, Status> {
        let request = pb::StoreIdRequest {
            id_or_row: id_or_row.into(),
        };
        self.inner
            .store_idv2(request)
            .await
            .map(Response::into_inner)
    }

    /// RPC 2：`RetrieveRowByIDV2`。
    ///
    /// # Errors
    ///
    /// 服务端拒绝或传输失败时返回 [`Status`]。
    pub async fn retrieve_row_by_idv2(
        &mut self,
        id_or_row: impl Into<String>,
    ) -> Result<pb::RetrieveRowByIdResponse, Status> {
        let request = pb::RetrieveRowByIdRequest {
            id_or_row: id_or_row.into(),
        };
        self.inner
            .retrieve_row_by_idv2(request)
            .await
            .map(Response::into_inner)
    }

    /// RPC 3：`WriteConfigV2`。
    ///
    /// # Errors
    ///
    /// 服务端拒绝或传输失败时返回 [`Status`]。
    pub async fn write_config_v2(
        &mut self,
        section: impl Into<String>,
        subtype: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<pb::WriteConfigResponse, Status> {
        let request = pb::WriteConfigRequest {
            section: section.into(),
            subtype: subtype.into(),
            value: value.into(),
        };
        self.inner
            .write_config_v2(request)
            .await
            .map(Response::into_inner)
    }

    /// RPC 4：`ReadConfigV2`。
    ///
    /// # Errors
    ///
    /// 服务端拒绝或传输失败时返回 [`Status`]。
    pub async fn read_config_v2(
        &mut self,
        section: impl Into<String>,
        subtype: impl Into<String>,
    ) -> Result<pb::ReadConfigResponse, Status> {
        let request = pb::ReadConfigRequest {
            section: section.into(),
            subtype: subtype.into(),
        };
        self.inner
            .read_config_v2(request)
            .await
            .map(Response::into_inner)
    }

    /// RPC 5：`UpdateVirtualValueV2`（服务端当前恒回 `Unimplemented`）。
    ///
    /// # Errors
    ///
    /// 服务端拒绝或传输失败时返回 [`Status`]。
    pub async fn update_virtual_value_v2(
        &mut self,
        old_virtual_value: i64,
        new_virtual_value: i64,
    ) -> Result<pb::UpdateVirtualValueResponse, Status> {
        let request = pb::UpdateVirtualValueRequest {
            old_virtual_value,
            new_virtual_value,
        };
        self.inner
            .update_virtual_value_v2(request)
            .await
            .map(Response::into_inner)
    }

    /// RPC 6：`RetrieveRealValueV2`。
    ///
    /// # Errors
    ///
    /// 服务端拒绝或传输失败时返回 [`Status`]。
    pub async fn retrieve_real_value_v2(
        &mut self,
        virtual_value: i64,
    ) -> Result<pb::RetrieveRealValueResponse, Status> {
        let request = pb::RetrieveRealValueRequest { virtual_value };
        self.inner
            .retrieve_real_value_v2(request)
            .await
            .map(Response::into_inner)
    }

    /// RPC 7：`RetrieveRealValueV2Pro`。
    ///
    /// # Errors
    ///
    /// 服务端拒绝或传输失败时返回 [`Status`]。
    pub async fn retrieve_real_value_v2_pro(
        &mut self,
        virtual_value: i64,
        virtual_value_sub: i64,
    ) -> Result<pb::RetrieveRealValueResponsePro, Status> {
        let request = pb::RetrieveRealValueRequestPro {
            virtual_value,
            virtual_value_sub,
        };
        self.inner
            .retrieve_real_value_v2_pro(request)
            .await
            .map(Response::into_inner)
    }

    /// RPC 8：`RetrieveVirtualValueV2`。
    ///
    /// # Errors
    ///
    /// 服务端拒绝或传输失败时返回 [`Status`]。
    pub async fn retrieve_virtual_value_v2(
        &mut self,
        real_value: impl Into<String>,
    ) -> Result<pb::RetrieveVirtualValueResponse, Status> {
        let request = pb::RetrieveVirtualValueRequest {
            real_value: real_value.into(),
        };
        self.inner
            .retrieve_virtual_value_v2(request)
            .await
            .map(Response::into_inner)
    }

    /// RPC 9：`StoreIDV2Pro`。
    ///
    /// # Errors
    ///
    /// 服务端拒绝或传输失败时返回 [`Status`]。
    pub async fn store_idv2_pro(
        &mut self,
        id_or_row: impl Into<String>,
        subid: impl Into<String>,
    ) -> Result<pb::StoreIdProResponse, Status> {
        let request = pb::StoreIdProRequest {
            id_or_row: id_or_row.into(),
            subid: subid.into(),
        };
        self.inner
            .store_idv2_pro(request)
            .await
            .map(Response::into_inner)
    }

    /// RPC 10：`RetrieveRowByIDV2Pro`。
    ///
    /// # Errors
    ///
    /// 服务端拒绝或传输失败时返回 [`Status`]。
    pub async fn retrieve_row_by_idv2_pro(
        &mut self,
        id_or_row: impl Into<String>,
        subid: impl Into<String>,
    ) -> Result<pb::RetrieveRowByIdProResponse, Status> {
        let request = pb::RetrieveRowByIdProRequest {
            id_or_row: id_or_row.into(),
            subid: subid.into(),
        };
        self.inner
            .retrieve_row_by_idv2_pro(request)
            .await
            .map(Response::into_inner)
    }

    /// RPC 11：`RetrieveVirtualValueV2Pro`。
    ///
    /// # Errors
    ///
    /// 服务端拒绝或传输失败时返回 [`Status`]。
    pub async fn retrieve_virtual_value_v2_pro(
        &mut self,
        id_or_row: impl Into<String>,
        subid: impl Into<String>,
    ) -> Result<pb::RetrieveVirtualValueProResponse, Status> {
        let request = pb::RetrieveVirtualValueProRequest {
            id_or_row: id_or_row.into(),
            subid: subid.into(),
        };
        self.inner
            .retrieve_virtual_value_v2_pro(request)
            .await
            .map(Response::into_inner)
    }

    /// RPC 12：`UpdateVirtualValueV2Pro`（服务端当前恒回 `Unimplemented`）。
    ///
    /// # Errors
    ///
    /// 服务端拒绝或传输失败时返回 [`Status`]。
    pub async fn update_virtual_value_v2_pro(
        &mut self,
        old_virtual_value_1: i64,
        new_virtual_value_1: i64,
        old_virtual_value_2: i64,
        new_virtual_value_2: i64,
    ) -> Result<pb::UpdateVirtualValueProResponse, Status> {
        let request = pb::UpdateVirtualValueProRequest {
            old_virtual_value_1,
            new_virtual_value_1,
            old_virtual_value_2,
            new_virtual_value_2,
        };
        self.inner
            .update_virtual_value_v2_pro(request)
            .await
            .map(Response::into_inner)
    }

    /// RPC 13：`SimplifiedStoreIDV2`。
    ///
    /// # Errors
    ///
    /// 服务端拒绝或传输失败时返回 [`Status`]。
    pub async fn simplified_store_idv2(
        &mut self,
        id_or_row: impl Into<String>,
    ) -> Result<pb::SimplifiedStoreIdResponse, Status> {
        let request = pb::SimplifiedStoreIdRequest {
            id_or_row: id_or_row.into(),
        };
        self.inner
            .simplified_store_idv2(request)
            .await
            .map(Response::into_inner)
    }

    /// RPC 14：`FindSubKeysByIdPro`（服务端当前恒回 `Unimplemented`）。
    ///
    /// # Errors
    ///
    /// 服务端拒绝或传输失败时返回 [`Status`]。
    pub async fn find_sub_keys_by_id_pro(
        &mut self,
        id: impl Into<String>,
    ) -> Result<pb::FindSubKeysResponse, Status> {
        let request = pb::FindSubKeysRequest { id: id.into() };
        self.inner
            .find_sub_keys_by_id_pro(request)
            .await
            .map(Response::into_inner)
    }

    /// RPC 15：`DeleteConfigV2`。
    ///
    /// # Errors
    ///
    /// 服务端拒绝或传输失败时返回 [`Status`]。
    pub async fn delete_config_v2(
        &mut self,
        section: impl Into<String>,
        subtype: impl Into<String>,
    ) -> Result<pb::DeleteConfigResponse, Status> {
        let request = pb::DeleteConfigRequest {
            section: section.into(),
            subtype: subtype.into(),
        };
        self.inner
            .delete_config_v2(request)
            .await
            .map(Response::into_inner)
    }

    /// RPC 16：`StoreCacheV2`。
    ///
    /// # Errors
    ///
    /// 服务端拒绝或传输失败时返回 [`Status`]。
    pub async fn store_cache_v2(
        &mut self,
        id_or_row: impl Into<String>,
    ) -> Result<pb::StoreCacheResponse, Status> {
        let request = pb::StoreCacheRequest {
            id_or_row: id_or_row.into(),
        };
        self.inner
            .store_cache_v2(request)
            .await
            .map(Response::into_inner)
    }

    /// RPC 17：`RetrieveRowByCacheV2`。
    ///
    /// # Errors
    ///
    /// 服务端拒绝或传输失败时返回 [`Status`]。
    pub async fn retrieve_row_by_cache_v2(
        &mut self,
        id_or_row: impl Into<String>,
    ) -> Result<pb::RetrieveRowByCacheResponse, Status> {
        let request = pb::RetrieveRowByCacheRequest {
            id_or_row: id_or_row.into(),
        };
        self.inner
            .retrieve_row_by_cache_v2(request)
            .await
            .map(Response::into_inner)
    }
}

/// 把十进制 vUIN 解析成 `i64`；非数字按调用方错误处理（见模块文档的差异说明）。
fn parse_vuin(value: &str) -> Result<i64, Status> {
    value
        .parse::<i64>()
        .map_err(|_| Status::invalid_argument(format!("行号不是十进制整数: {value}")))
}

/// [`StorageError`] → gRPC status 的分类映射。
fn status(err: StorageError) -> Status {
    let message = err.to_string();
    match err {
        StorageError::NotFound(_) | StorageError::Expired(_) => Status::not_found(message),
        StorageError::Corrupt(_) => Status::invalid_argument(message),
        StorageError::Redb(_) => Status::internal(message),
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::sync::Arc;

    use tokio_stream::wrappers::TcpListenerStream;
    use tonic::transport::{Endpoint, Server};

    use super::pb;
    use super::{IdMapClient, IdMapService};
    use crate::IdMapOptions;
    use crate::idmap::IdMapStore;

    /// 只监听 `127.0.0.1:0` 的测试实例；drop 时 abort 服务端任务并删临时目录。
    #[derive(Debug)]
    struct Harness {
        _dir: tempfile::TempDir,
        store: Arc<IdMapStore>,
        client: IdMapClient,
        server: tokio::task::JoinHandle<()>,
    }

    impl Drop for Harness {
        fn drop(&mut self) {
            self.server.abort();
        }
    }

    async fn harness() -> Result<Harness, Box<dyn Error>> {
        let dir = tempfile::tempdir()?;
        let store = Arc::new(IdMapStore::open(
            IdMapOptions::new(
                dir.path().join("openid-map.redb"),
                dir.path().join("msgid-map.redb"),
            )
            .with_app_id("10000001"),
        )?);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let service = pb::id_map_service_server::IdMapServiceServer::new(IdMapService::new(
            Arc::clone(&store),
        ));
        let server = tokio::spawn(async move {
            let incoming = TcpListenerStream::new(listener);
            if let Err(err) = Server::builder()
                .add_service(service)
                .serve_with_incoming(incoming)
                .await
            {
                eprintln!("测试用 gRPC 服务端退出: {err}");
            }
        });
        let client = IdMapClient::connect(Endpoint::from_shared(format!("http://{addr}"))?).await?;
        Ok(Harness {
            _dir: dir,
            store,
            client,
            server,
        })
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn id_map_rpc_round_trip() -> Result<(), Box<dyn Error>> {
        let mut harness = harness().await?;

        let row = harness.client.store_idv2("openid-user-1").await?.row;
        assert!(row > 0);
        let id = harness
            .client
            .retrieve_row_by_idv2(row.to_string())
            .await?
            .id;
        assert_eq!(id, "openid-user-1");

        let virtual_value = harness
            .client
            .retrieve_virtual_value_v2("openid-user-1")
            .await?;
        assert_eq!(virtual_value.real, "openid-user-1");
        assert_eq!(virtual_value.r#virtual, row.to_string());

        // Pro 变体是两次独立分配，不是复合键。
        let pro = harness
            .client
            .store_idv2_pro("openid-group-1", "openid-user-1")
            .await?;
        assert!(pro.row > 0);
        assert_eq!(pro.sub_row, row);
        let pro_back = harness
            .client
            .retrieve_row_by_idv2_pro(pro.row.to_string(), row.to_string())
            .await?;
        assert_eq!(pro_back.id, "openid-group-1");
        assert_eq!(pro_back.subid, "openid-user-1");

        let pro_values = harness
            .client
            .retrieve_virtual_value_v2_pro("openid-group-1", "openid-user-1")
            .await?;
        assert_eq!(pro_values.first_value, pro.row.to_string());
        assert_eq!(pro_values.second_value, row.to_string());

        // Go `RetrieveVirtualValue` 只查不分配：未登记的身份必须报 NotFound，不能凭空建映射。
        let err = harness
            .client
            .retrieve_virtual_value_v2("openid-unknown-9")
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::NotFound);

        let real = harness.client.retrieve_real_value_v2(row).await?;
        assert_eq!(real.r#virtual, row.to_string());
        assert_eq!(real.real, "openid-user-1");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn config_and_cache_rpc_round_trip() -> Result<(), Box<dyn Error>> {
        let mut harness = harness().await?;

        let written = harness
            .client
            .write_config_v2("lotus", "token", "abc")
            .await?;
        assert_eq!(written.status, "success");
        assert_eq!(
            harness.client.read_config_v2("lotus", "token").await?.value,
            "abc"
        );
        // 确认真的落到 store，而不是只回显。
        assert_eq!(harness.store.read_config("lotus", "token")?, "abc");

        harness.client.delete_config_v2("lotus", "token").await?;
        let err = harness
            .client
            .read_config_v2("lotus", "token")
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::NotFound);

        let virtual_id = harness.client.store_cache_v2("real-msg-1").await?.row;
        let real = harness
            .client
            .retrieve_row_by_cache_v2(virtual_id.to_string())
            .await?
            .id;
        assert_eq!(real, "real-msg-1");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn unimplemented_rpcs_report_unimplemented() -> Result<(), Box<dyn Error>> {
        let mut harness = harness().await?;

        let err = harness
            .client
            .update_virtual_value_v2(1, 2)
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unimplemented);
        let err = harness
            .client
            .find_sub_keys_by_id_pro("id")
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unimplemented);
        Ok(())
    }
}

//! 编译 `proto/idmap.proto`（17 个 RPC）为 tonic 服务端/客户端桩，产物写入 `OUT_DIR`。
//!
//! - proto 与 Go 共享同一份文件：**service 全名、消息名与字段号都不可以改**，否则 Go 侧
//!   `proto/idmap.pb.go` 生成的线上契约立刻失配。
//! - 该 proto 没有 `package` 声明，prost 因此把产出的文件名定为 `_.rs`
//!   （`src/idmap/grpc.rs` 的 `include!` 依赖这个事实）；也不要为了"好名字"给 proto 补
//!   `package` ——那会改掉 `IDMapService` 的完全限定名。
//! - 构建机不预装 `protoc`：改用 `protoc-bin-vendored` 的预编译二进制，经
//!   `prost_build::Config::protoc_executable` 传入。edition 2024 下
//!   `std::env::set_var` 是 unsafe，而 workspace 全局 `forbid(unsafe_code)`。

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../proto");
    let proto = proto_dir.join("idmap.proto");
    println!("cargo:rerun-if-changed={}", proto.display());

    let mut prost = prost_build::Config::new();
    prost.protoc_executable(protoc_bin_vendored::protoc_bin_path()?);
    tonic_prost_build::configure()
        .build_client(true)
        .build_server(true)
        .compile_with_config(prost, &[proto], &[proto_dir])?;
    Ok(())
}

#!/usr/bin/env bash
# 生成脚本（proto 等 generated code）
# 用法: ./scripts/generate.sh
# 需要: protoc + protoc-gen-go（proto 生成）；缺少时跳过并提示
set -euo pipefail
cd "$(dirname "$0")/.."

# 1. proto → Go（idmap gRPC）
if command -v protoc >/dev/null 2>&1; then
  echo "[generate] regenerating proto..."
  protoc --go_out=. --go_opt=paths=source_relative \
    --go-grpc_out=. --go-grpc_opt=paths=source_relative \
    -I proto proto/idmap.proto
  echo "[generate] proto done."
else
  echo "[generate] protoc not found; skipping proto generation (generated files already committed)."
fi

echo "[generate] done."

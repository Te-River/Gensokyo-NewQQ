# 生成脚本（proto 等 generated code）
# 用法: pwsh scripts/generate.ps1
# 需要: protoc + protoc-gen-go（proto 生成）；缺少时跳过并提示

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

# 1. proto → Go（idmap gRPC）
$protoc = Get-Command protoc -ErrorAction SilentlyContinue
if ($protoc) {
    Write-Host "[generate] regenerating proto..."
    protoc --go_out=. --go_opt=paths=source_relative `
        --go-grpc_out=. --go-grpc_opt=paths=source_relative `
        -I proto proto/idmap.proto
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    Write-Host "[generate] proto done."
} else {
    Write-Host "[generate] protoc not found; skipping proto generation (generated files already committed)."
}

Write-Host "[generate] done."

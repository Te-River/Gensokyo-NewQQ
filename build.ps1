# Gensokyo 本地构建脚本
# 使用 goproxy.cn 加速依赖下载

$ErrorActionPreference = "Stop"

# 设置 Go 模块代理（国内高速镜像）
$env:GOPROXY = "https://goproxy.cn,direct"
$env:GOFLAGS = "-mod=mod"

Write-Host "=== Gensokyo 本地构建 ===" -ForegroundColor Cyan
Write-Host "Go Proxy: $env:GOPROXY" -ForegroundColor Gray

# 参数
$targetOS = if ($args[0]) { $args[0] } else { (go env GOOS) }
$targetArch = if ($args[1]) { $args[1] } else { (go env GOARCH) }
$upxLevel = if ($args[2]) { $args[2] } else { "7" }

$ext = ""
if ($targetOS -eq "windows") { $ext = ".exe" }

$env:GOOS = $targetOS
$env:GOARCH = $targetArch
$env:CGO_ENABLED = "0"

$output = "gensokyo-$targetOS-$targetArch$ext"

Write-Host "Target: $targetOS/$targetArch"
Write-Host "Output: $output"

# 下载依赖
Write-Host "`n[1/3] 下载依赖..." -ForegroundColor Yellow
go mod tidy

# 编译
Write-Host "[2/3] 编译..." -ForegroundColor Yellow
go build -trimpath -ldflags="-s -w" -v -o $output .

if ($LASTEXITCODE -ne 0) {
    Write-Host "编译失败！" -ForegroundColor Red
    exit 1
}

Write-Host "编译成功: $output" -ForegroundColor Green

# UPX 压缩（固定等级 7）
Write-Host "[3/3] UPX 压缩..." -ForegroundColor Yellow
$upx = Get-Command "upx" -ErrorAction SilentlyContinue
if ($upx) {
    & $upx.Source "-7" $output
    Write-Host "UPX 完成" -ForegroundColor Green
} else {
    Write-Host "UPX 未安装，跳过压缩。安装: winget install upx" -ForegroundColor Gray
}

Write-Host "`n=== 构建完成 ===" -ForegroundColor Cyan

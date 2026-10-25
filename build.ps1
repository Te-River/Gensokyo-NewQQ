# Gensokyo build script —— Rust 薄包装
# 实际构建逻辑在 `cargo xtask build`（见 docs/rust-build.md），本脚本只做“Go 命名 → Rust 三元组”
# 的参数翻译：不再调用 Go 工具链，也不再自己创建 webui/dist 占位目录（xtask 负责）。
#
# 单目标（缺省宿主）：
#   .\build.ps1
#   .\build.ps1 linux amd64
#   .\build.ps1 -NoWebUI
# 全部目标（矩阵 × 双版本，与 CI cross_compile.yml 对齐）：
#   .\build.ps1 -All
#   .\build.ps1 -LinuxOnly
#
# 注意：cargo 由 PATH 决定，会遵守 rust-toolchain.toml（1.99.1 / 宿主 abi）。本机若没有
# MSVC 的 link.exe，先用 `rustup override set 1.99.1-x86_64-pc-windows-gnu` 切到 GNU 工具链。

param(
    [Parameter(Position = 0)]
    [string]$TargetOS = "",

    [Parameter(Position = 1)]
    [string]$TargetArch = "",

    [Parameter(Position = 2)]
    [int]$UPXLevel = 7,

    [switch]$All,
    [switch]$LinuxOnly,
    [switch]$NoUPX,
    [switch]$NoWebUI
)

$ErrorActionPreference = 'Stop'

# 兼容 --xxx 写法（PowerShell 标准是 -xxx）
if ($TargetOS -match '^--?[Aa]ll$') { $All = $true; $TargetOS = '' }
if ($TargetOS -match '^--?[Nn]o[Ww]eb[Uu][Ii]$') { $NoWebUI = $true; $TargetOS = '' }
if ($TargetArch -match '^--?[Nn]o[Ww]eb[Uu][Ii]$') { $NoWebUI = $true; $TargetArch = '' }

if ($TargetOS -eq 'all') {
    $All = $true
    $TargetOS = ''
}

# Go 命名 → Rust 三元组。只覆盖“单目标构建”会点名的平台；全量走 xtask 自己的矩阵。
$TripleMap = @{
    'linux-amd64'   = 'x86_64-unknown-linux-gnu'
    'linux-386'     = 'i686-unknown-linux-gnu'
    'linux-arm'     = 'armv7-unknown-linux-gnueabihf'
    'linux-arm64'   = 'aarch64-unknown-linux-gnu'
    'darwin-amd64'  = 'x86_64-apple-darwin'
    'darwin-arm64'  = 'aarch64-apple-darwin'
    'windows-amd64' = 'x86_64-pc-windows-msvc'
    'windows-386'   = 'i686-pc-windows-msvc'
    'android-arm64' = 'aarch64-linux-android'
}

function Get-HostOS {
    switch ([System.Environment]::OSVersion.Platform.ToString()) {
        'Win32NT' { return 'windows' }
        'MacOSX' { return 'darwin' }
        default { return 'linux' }
    }
}

function Get-HostArch {
    switch ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()) {
        'X64' { return 'amd64' }
        'X86' { return '386' }
        'Arm64' { return 'arm64' }
        default { throw "无法识别宿主架构: $([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture)" }
    }
}

$triple = ''
if (-not $All -and -not $LinuxOnly) {
    if ($TargetOS -or $TargetArch) {
        # 只给了一个维度时，另一个按宿主补齐（保留 Go 版 `.\build.ps1 linux` 的用法）
        $osName = if ($TargetOS) { $TargetOS } else { Get-HostOS }
        $archName = if ($TargetArch) { $TargetArch } else { Get-HostArch }
        $key = "$osName-$archName".ToLowerInvariant()
        if (-not $TripleMap.ContainsKey($key)) {
            $available = ($TripleMap.Keys | Sort-Object) -join ', '
            throw "不支持的单目标: $key`n可用: $available`n全量构建请用 -All / -LinuxOnly"
        }
        $triple = $TripleMap[$key]
    }
    # 两者都没给：宿主构建不传 --target，交给 cargo 自己选（避免强绑 msvc/gnu abi）
}

$cargoArgs = @('xtask', 'build')
if ($All) { $cargoArgs += '--all' }
if ($LinuxOnly) { $cargoArgs += '--linux-only' }
if ($NoWebUI) { $cargoArgs += '--no-webui' }
if ($NoUPX) { $cargoArgs += '--no-upx' } else { $cargoArgs += @('--upx-level', "$UPXLevel") }
if ($triple) { $cargoArgs += @('--target', $triple) }

Write-Host "=== Gensokyo Build ===" -ForegroundColor Cyan
Write-Host "cargo $($cargoArgs -join ' ')" -ForegroundColor Gray

& cargo @cargoArgs
if ($LASTEXITCODE -ne 0) {
    Write-Host "构建失败（退出码 $LASTEXITCODE）" -ForegroundColor Red
    exit $LASTEXITCODE
}

Write-Host "产物见 release/（已被 .gitignore 忽略）" -ForegroundColor Green

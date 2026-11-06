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
# 工具链自动选择（仅 Windows 宿主）：检测到 MSVC（link.exe / vswhere 定位到 C++ 工具集）时
# 保持 rust-toolchain.toml 默认（msvc 宿主）；未检测到时回退已安装的 GNU 工具链并前置
# msys2 的 dlltool/gcc 到 PATH；两者都不可用则打印安装指引并退出（见 Select 区注释）。

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

# ================= Rust 工具链选择（Windows 宿主）=================
# rust-toolchain.toml 钉 1.99.1，Windows 上默认解析到 msvc 宿主；本机若无 VS Build Tools，
# cargo 会报 `error: linker `link.exe` not found`。策略：
#   1. 检测到 MSVC（PATH 里的 link.exe，或 vswhere 能定位 VS/Build Tools 的 C++ 工具集）
#      → 保持默认工具链，不做任何改动；
#   2. 无 MSVC 但装有 GNU 工具链 → 把 msys2（dlltool 在 usr\bin、gcc 在 mingw64\bin）前置
#      PATH，改走 `cargo +<gnu>`。注意 rustup 自带的 self-contained dlltool 链接
#      windows-sys 会 CreateProcess 失败，必须用 msys2 的；
#   3. 两者都不满足 → 打印两条安装路径指引后 exit 1。
function Test-MsvcAvailable {
    # link.exe 在 PATH 说明开发者命令行环境已就绪；但 msys 的 coreutils link.exe 不算 MSVC 信号
    $link = Get-Command link.exe -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($link -and $link.Source -notmatch 'msys') { return $true }
    # -products * 才覆盖 Build Tools（只查 VS IDE 会漏装独立 Build Tools 的机器）
    # 部分环境进程里 ProgramFiles(x64) 未注入，逐级回退，避免 Join-Path 收到 null
    $pf64 = ${env:ProgramFiles(x64)}
    if (-not $pf64) { $pf64 = $env:ProgramFiles }
    if (-not $pf64) { $pf64 = 'C:\Program Files' }
    $vswhere = Join-Path $pf64 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (Test-Path $vswhere) {
        $found = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath 2>$null
        if ($found) { return $true }
    }
    return $false
}

function Get-GnuToolchainName {
    # 从 `rustup toolchain list` 找已安装的 x86_64-pc-windows-gnu 工具链；
    # 优先与 rust-toolchain.toml 的 channel 同版本，避免误选 stable-gnu 造成版本漂移。
    if (-not (Get-Command rustup -ErrorAction SilentlyContinue)) { return $null }
    $list = & rustup toolchain list 2>$null
    if (-not $list) { return $null }
    $gnu = $list | ForEach-Object { ($_ -split '\s+')[0] } | Where-Object { $_ -like '*-x86_64-pc-windows-gnu' }
    $gnu = @($gnu)
    if ($gnu.Count -eq 0) { return $null }
    $pinned = ''
    $toml = Join-Path $PSScriptRoot 'rust-toolchain.toml'
    if (Test-Path $toml) {
        $m = Select-String -Path $toml -Pattern 'channel\s*=\s*"([^"]+)"' | Select-Object -First 1
        if ($m) { $pinned = $m.Matches[0].Groups[1].Value }
    }
    $exact = @($gnu | Where-Object { $_ -eq "$pinned-x86_64-pc-windows-gnu" })
    if ($exact.Count -gt 0) { return $exact[0] }
    return $gnu[0]
}

function Find-GnuBinDirs {
    # GNU 链接依赖：dlltool 在 usr\bin，gcc 在 mingw64\bin。按 C:\msys64 → C:\msys2 →
    # where.exe dlltool 的顺序探测（排除 rustup self-contained 目录），找不到返回空数组。
    $dirs = @()
    foreach ($root in 'C:\msys64', 'C:\msys2') {
        $usr = Join-Path $root 'usr\bin'
        $mingw = Join-Path $root 'mingw64\bin'
        if (Test-Path $usr) { $dirs += $usr }
        if (Test-Path $mingw) { $dirs += $mingw }
    }
    if ($dirs.Count -eq 0) {
        $hit = @(where.exe dlltool 2>$null) | Where-Object { $_ -notmatch 'self-contained' } | Select-Object -First 1
        if ($hit) { $dirs += Split-Path $hit[0] }
    }
    return $dirs
}

function Show-ToolchainGuidance {
    Write-Host '无法选择可用的 Rust 工具链，请任选其一安装后重试：' -ForegroundColor Red
    Write-Host '  1) 安装 VS Build Tools（勾选“使用 C++ 的桌面开发”，提供 link.exe）：' -ForegroundColor Yellow
    Write-Host '     winget install --id Microsoft.VisualStudio.2022.BuildTools --override "--wait --add Microsoft.VisualStudio.Workload.VCTools"' -ForegroundColor Yellow
    Write-Host '  2) 安装 GNU 工具链与 msys2（提供 dlltool/gcc）：' -ForegroundColor Yellow
    Write-Host '     rustup toolchain install stable-x86_64-pc-windows-gnu' -ForegroundColor Yellow
    Write-Host '     winget install --id MSYS2.MSYS2' -ForegroundColor Yellow
    Write-Host '     然后 C:\msys64\usr\bin\pacman.exe -S mingw-w64-x86_64-gcc（保持默认 C:\msys64）' -ForegroundColor Yellow
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

# 调用 cargo 前按本机实况选择工具链；非 Windows 宿主不存在 msvc/gnu 之争，不处理。
$cargoToolchain = @()
if ((Get-HostOS) -eq 'windows') {
    if (Test-MsvcAvailable) {
        # MSVC 可用：保持 rust-toolchain.toml 默认（msvc 宿主），不改动
    } else {
        $gnuTc = Get-GnuToolchainName
        $gnuDirs = @(Find-GnuBinDirs)
        if (-not $gnuTc -or $gnuDirs.Count -eq 0) {
            Write-Host '未检测到 MSVC（PATH 无 link.exe 且 vswhere 定位不到 C++ 工具集），也没有可用的 GNU 工具链回退。' -ForegroundColor Red
            Show-ToolchainGuidance
            exit 1
        }
        # GNU 工具链链接 windows-sys 需要 msys2 的 dlltool/gcc，前置到本进程 PATH
        $pathParts = @($env:PATH -split ';')
        foreach ($d in $gnuDirs) {
            if ($pathParts -notcontains $d) {
                $env:PATH = "$d;$env:PATH"
                $pathParts += $d
            }
        }
        Write-Host "未检测到 MSVC，已回退 GNU 工具链: $gnuTc（前置 PATH: $($gnuDirs -join '; ')）" -ForegroundColor Yellow
        $cargoToolchain = @("+$gnuTc")
    }
}

Write-Host "=== Gensokyo Build ===" -ForegroundColor Cyan
Write-Host "cargo $((($cargoToolchain + $cargoArgs) -join ' '))" -ForegroundColor Gray

& cargo @cargoToolchain @cargoArgs
if ($LASTEXITCODE -ne 0) {
    Write-Host "构建失败（退出码 $LASTEXITCODE）" -ForegroundColor Red
    exit $LASTEXITCODE
}

Write-Host "产物见 release/（已被 .gitignore 忽略）" -ForegroundColor Green

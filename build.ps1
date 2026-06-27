# Gensokyo build script
# Single target:
#   .\build.ps1
#   .\build.ps1 linux amd64
# All default targets:
#   .\build.ps1 -All
#   .\build.ps1 -LinuxOnly

param(
    [Parameter(Position = 0)]
    [string]$TargetOS = "",

    [Parameter(Position = 1)]
    [string]$TargetArch = "",

    [Parameter(Position = 2)]
    [string]$UPXLevel = "7",

    [switch]$All,
    [switch]$LinuxOnly,
    [switch]$NoUPX
)

$ErrorActionPreference = 'Stop'

$env:GOPROXY = 'https://goproxy.cn,direct'
$env:GOFLAGS = '-mod=mod'
$env:CGO_ENABLED = '0'

if ($TargetOS -eq 'all') {
    $All = $true
    $TargetOS = ''
}
if ($LinuxOnly) {
    $All = $true
}

function Ensure-WebUIDist {
    $webuiDist = 'webui/dist'
    if (-not (Test-Path "$webuiDist/css/style.css")) {
        $null = New-Item -ItemType Directory -Force -Path "$webuiDist/css", "$webuiDist/fonts", "$webuiDist/icons", "$webuiDist/js" 2>$null
        Set-Content -Path "$webuiDist/placeholder.html" -Value '' -NoNewline
        Set-Content -Path "$webuiDist/css/placeholder.css" -Value '' -NoNewline
        Set-Content -Path "$webuiDist/fonts/placeholder.txt" -Value '' -NoNewline
        Set-Content -Path "$webuiDist/icons/placeholder.txt" -Value '' -NoNewline
        Set-Content -Path "$webuiDist/js/placeholder.js" -Value '' -NoNewline
    }
}

function Get-BuildLdflags {
    $gitCommit = ''
    try {
        $gitCommit = (git rev-parse --short HEAD 2>$null).Trim()
    } catch {
        $gitCommit = ''
    }

    if ($gitCommit) {
        $buildType = 'git'
        $buildSpec = $gitCommit
    } else {
        $buildType = 'dev'
        $epoch = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
        $buildSpec = ('{0}.{1:D3}' -f [math]::Floor($epoch / 1000), ($epoch % 1000))
    }

    Write-Host "Build info: $buildType-$buildSpec" -ForegroundColor Gray
    return "-s -w -X github.com/hoshinonyaruko/gensokyo/buildinfo.BuildType=$buildType -X github.com/hoshinonyaruko/gensokyo/buildinfo.BuildSpec=$buildSpec"
}

function Invoke-GensokyoBuild {
    param(
        [Parameter(Mandatory = $true)]
        [hashtable]$Target,

        [Parameter(Mandatory = $true)]
        [string]$Ldflags
    )

    $env:GOOS = $Target.GOOS
    $env:GOARCH = $Target.GOARCH

    $ext = if ($Target.GOOS -eq 'windows') { '.exe' } else { '' }
    $outName = "gensokyo-$($Target.OS)-$($Target.Arch)$ext"

    Write-Host "[build] $($Target.GOOS)/$($Target.GOARCH) -> $outName" -ForegroundColor Yellow
    go build -trimpath -ldflags="$Ldflags" -v -o $outName .

    if ($LASTEXITCODE -ne 0) {
        throw "Build failed: $($Target.GOOS)/$($Target.GOARCH)"
    }

    Write-Host "  OK: $outName" -ForegroundColor Green
    return $outName
}

function Invoke-Upx {
    param(
        [Parameter(Mandatory = $true)]
        [string[]]$Outputs
    )

    if ($NoUPX) {
        Write-Host "UPX disabled." -ForegroundColor Gray
        return
    }

    $upx = Get-Command 'upx' -ErrorAction SilentlyContinue
    if (-not $upx) {
        Write-Host 'UPX not found, skip compression.' -ForegroundColor Gray
        Write-Host 'Install: winget install upx' -ForegroundColor Gray
        return
    }

    foreach ($output in $Outputs) {
        Write-Host "[upx] Compressing $output..." -ForegroundColor Yellow
        & $upx.Source "-$UPXLevel" $output
        if ($LASTEXITCODE -ne 0) {
            throw "UPX failed: $output"
        }
    }
}

$targets = @()
if ($All) {
    $targets = @(
        @{GOOS = 'linux';   GOARCH = 'amd64'; OS = 'linux';   Arch = 'amd64' }
        @{GOOS = 'linux';   GOARCH = '386';   OS = 'linux';   Arch = '386' }
        @{GOOS = 'linux';   GOARCH = 'arm64'; OS = 'linux';   Arch = 'arm64' }
        @{GOOS = 'windows'; GOARCH = 'amd64'; OS = 'windows'; Arch = 'amd64' }
        @{GOOS = 'windows'; GOARCH = '386';   OS = 'windows'; Arch = '386' }
    )
    if ($LinuxOnly) {
        $targets = $targets | Where-Object { $_.GOOS -eq 'linux' }
    }
} else {
    $targetOSValue = if ($TargetOS) { $TargetOS } else { (go env GOOS) }
    $targetArchValue = if ($TargetArch) { $TargetArch } else { (go env GOARCH) }
    $targets = @(@{GOOS = $targetOSValue; GOARCH = $targetArchValue; OS = $targetOSValue; Arch = $targetArchValue })
}

Write-Host '=== Gensokyo Build ===' -ForegroundColor Cyan
Write-Host "Go Proxy: $env:GOPROXY" -ForegroundColor Gray
Write-Host "Targets : $($targets.Count) platform(s)" -ForegroundColor Gray

$ldflags = Get-BuildLdflags

Write-Host "`n[deps] Downloading dependencies..." -ForegroundColor Yellow
go mod tidy

Ensure-WebUIDist

$outputs = @()
$failed = @()
foreach ($target in $targets) {
    try {
        $outputs += Invoke-GensokyoBuild -Target $target -Ldflags $ldflags
    } catch {
        $failed += "$($target.GOOS)/$($target.GOARCH)"
        Write-Host "  FAILED: $($target.GOOS)/$($target.GOARCH)" -ForegroundColor Red
        Write-Host "  $_" -ForegroundColor Red
        if (-not $All) {
            exit 1
        }
    }
}

if ($outputs.Count -gt 0) {
    Invoke-Upx -Outputs $outputs
}

Write-Host "`n=== Build Complete: $($outputs.Count) succeeded ===" -ForegroundColor Cyan
foreach ($output in $outputs) {
    $size = (Get-Item $output).Length / 1MB
    Write-Host "  $output  ($([math]::Round($size, 1)) MB)" -ForegroundColor White
}

if ($failed.Count -gt 0) {
    Write-Host "Failed targets: $($failed -join ', ')" -ForegroundColor Red
    exit 1
}

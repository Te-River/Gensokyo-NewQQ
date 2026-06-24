# Gensokyo multi-platform build script
# Builds for all target platforms at once
# Usage: .\build_all.ps1              # build all default targets
#        .\build_all.ps1 -LinuxOnly   # build Linux targets only
#        .\build_all.ps1 -NoUPX       # skip UPX compression

param(
    [switch]$LinuxOnly,
    [switch]$NoUPX
)

$ErrorActionPreference = 'Stop'

$env:GOPROXY = 'https://goproxy.cn,direct'
$env:GOFLAGS = '-mod=mod'
$env:CGO_ENABLED = '0'

# ===================== Build Targets =====================
# Matching .github/workflows/build.yml matrix
$targets = @(
    @{GOOS = 'linux';   GOARCH = 'amd64'; OS = 'linux';   Arch = 'amd64' }
    @{GOOS = 'linux';   GOARCH = '386';   OS = 'linux';   Arch = '386' }
    @{GOOS = 'linux';   GOARCH = 'arm64'; OS = 'linux';   Arch = 'arm64' }
    @{GOOS = 'windows'; GOARCH = 'amd64'; OS = 'windows'; Arch = 'amd64' }
    @{GOOS = 'windows'; GOARCH = '386';   OS = 'windows'; Arch = '386' }
)

if ($LinuxOnly) {
    $targets = $targets | Where-Object { $_.GOOS -ne 'windows' }
}

# ===================== Pre-build: ensure webui placeholders =====================
$webuiDist = 'webui/dist'
if (-not (Test-Path "$webuiDist/css/style.css")) {
    $null = New-Item -ItemType Directory -Force -Path "$webuiDist/css", "$webuiDist/fonts", "$webuiDist/icons", "$webuiDist/js" 2>$null
    Set-Content -Path "$webuiDist/placeholder.html" -Value '' -NoNewline
    Set-Content -Path "$webuiDist/css/placeholder.css" -Value '' -NoNewline
    Set-Content -Path "$webuiDist/fonts/placeholder.txt" -Value '' -NoNewline
    Set-Content -Path "$webuiDist/icons/placeholder.txt" -Value '' -NoNewline
    Set-Content -Path "$webuiDist/js/placeholder.js" -Value '' -NoNewline
}

Write-Host "=== Gensokyo Build All ===" -ForegroundColor Cyan
Write-Host "Go Proxy: $env:GOPROXY" -ForegroundColor Gray
Write-Host "Targets : $($targets.Count) platform(s)`n" -ForegroundColor Gray

# ===================== Deps (once) =====================
Write-Host "[deps] Downloading dependencies..." -ForegroundColor Yellow
go mod tidy
Write-Host ""

# ===================== Build each target =====================
$outputs = @()
foreach ($t in $targets) {
    $env:GOOS = $t.GOOS
    $env:GOARCH = $t.GOARCH
    $ext = if ($t.GOOS -eq 'windows') { '.exe' } else { '' }
    $outName = "gensokyo-$($t.OS)-$($t.Arch)$ext"

    Write-Host "[build] $($t.GOOS)/$($t.GOARCH) -> $outName" -ForegroundColor Yellow
    go build -trimpath -ldflags="-s -w" -v -o $outName .

    if ($LASTEXITCODE -ne 0) {
        Write-Host "  FAILED: $($t.GOOS)/$($t.GOARCH)" -ForegroundColor Red
        continue
    }

    $outputs += $outName
    Write-Host "  OK: $outName" -ForegroundColor Green
}

# ===================== UPX compress =====================
if (-not $NoUPX) {
    $upx = Get-Command 'upx' -ErrorAction SilentlyContinue
    if ($upx) {
        Write-Host ""
        foreach ($o in $outputs) {
            Write-Host "[upx] Compressing $o..." -ForegroundColor Yellow
            & $upx.Source '-7' $o
        }
    } else {
        Write-Host "`nUPX not found, skip compression." -ForegroundColor Gray
        Write-Host "Install: winget install upx" -ForegroundColor Gray
    }
}

Write-Host "`n=== Build Complete: $($outputs.Count) succeeded ===" -ForegroundColor Cyan
foreach ($o in $outputs) {
    $size = (Get-Item $o).Length / 1MB
    Write-Host "  $o  ($([math]::Round($size, 1)) MB)" -ForegroundColor White
}

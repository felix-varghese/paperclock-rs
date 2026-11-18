<#
.SYNOPSIS
  Clean paperclock-rs build caches - correctly, unlike a bare `cargo clean`.

.DESCRIPTION
  `cargo clean -p esp-idf-sys` on its own only cleans the HOST-side build
  script cache (a few MB) - it silently does NOT touch the actual
  target-specific OUT_DIR (under xtensa-esp32s3-espidf/release/), which is
  where the real CMake build, sdkconfig, and component list live. Config
  changes (Cargo.toml metadata, sdkconfig.defaults, components/) are
  silently ignored by a plain clean/rebuild because that target-specific
  CMake cache never gets invalidated. The fix is the --target flag below.

.PARAMETER Target
  Clean esp-idf-sys's target-specific build output (the actual fix for
  "I changed Cargo.toml / sdkconfig.defaults / components/ and it didn't
  take effect"). This is usually what you want after touching:
    - Cargo.toml's [package.metadata.esp-idf-sys] section (extra_components etc.)
    - sdkconfig.defaults
    - anything under components/

.PARAMETER Full
  Nuclear option: wipes the entire build-cache directory (target-dir,
  ESP-IDF SDK clone, GCC toolchain download - see env.ps1). Forces a full
  ESP-IDF re-clone and rebuild, 10-20+ minutes. Only needed if -Target
  isn't enough or that directory itself is suspected corrupt.
#>
param(
    [switch]$Target,
    [switch]$Full
)

$ProjectRoot = Split-Path -Parent $PSScriptRoot
. "$ProjectRoot\env.ps1"
Set-Location "$ProjectRoot\firmware"

if ($Full) {
    Write-Host "Wiping $BuildRoot (build cache + ESP-IDF SDK + toolchain checkout)..." -ForegroundColor Yellow
    Get-ChildItem "$BuildRoot" -Force -ErrorAction SilentlyContinue | ForEach-Object {
        Remove-Item -Recurse -Force $_.FullName -ErrorAction SilentlyContinue
    }
    Write-Host "Done. Next build will re-clone ESP-IDF (~10-20 min)." -ForegroundColor Yellow
} elseif ($Target) {
    Write-Host "Cleaning esp-idf-sys's target-specific build output..." -ForegroundColor Cyan
    cargo clean -p esp-idf-sys --release --target xtensa-esp32s3-espidf
    Write-Host "Done. Next build will reconfigure CMake and pick up config/component changes."  -ForegroundColor Cyan
} else {
    Write-Host "Plain cargo clean (host-side only - won't fix stale CMake config)." -ForegroundColor DarkGray
    cargo clean --release
    Write-Host "`nIf you changed Cargo.toml metadata, sdkconfig.defaults, or components/," -ForegroundColor Yellow
    Write-Host "re-run with -Target instead - that's almost certainly what you need." -ForegroundColor Yellow
}

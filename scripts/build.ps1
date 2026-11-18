<#
.SYNOPSIS
  Build the paperclock-rs firmware (release).

.DESCRIPTION
  Just `. env.ps1; cd firmware; cargo build --release`, wrapped so you
  don't have to remember the dot-sourcing step. Pass -Verbose to also
  print the last N lines of the log even on success, useful for
  eyeballing warnings.
#>
param(
    [switch]$Verbose
)

$ProjectRoot = Split-Path -Parent $PSScriptRoot
. "$ProjectRoot\env.ps1"

Set-Location "$ProjectRoot\firmware"
cargo build --release
$exitCode = $LASTEXITCODE

if ($exitCode -ne 0) {
    Write-Host "`nBuild failed (exit $exitCode)." -ForegroundColor Red
    Write-Host "If you changed anything under components/, Cargo.toml's" -ForegroundColor Yellow
    Write-Host "[package.metadata.esp-idf-sys] section, or sdkconfig.defaults," -ForegroundColor Yellow
    Write-Host "a plain rebuild may be using a stale cache. Try clean.ps1 -Target first." -ForegroundColor Yellow
}

exit $exitCode

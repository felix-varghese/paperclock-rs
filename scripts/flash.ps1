<#
.SYNOPSIS
  Flash paperclock-rs to the board and watch the serial console.

.DESCRIPTION
  Always does flash+monitor as a single combined espflash invocation.
  Doing them as two separate steps (flash, then a fresh `espflash monitor`)
  reliably lost the boot log on this board: the ESP32-S3's native USB
  peripheral briefly disconnects/re-enumerates on reset, and a monitor
  session opened afterward either misses the gap entirely or, if opened
  too early, errors out with "The device does not recognize the command"
  (Windows tearing down the handle mid-reconnect). Combined flash+monitor
  keeps one connection open across the reset and reliably captures
  everything from the ROM banner onward.

.PARAMETER Port
  Serial port. Auto-detected if omitted (looks for "USB Serial Device").

.PARAMETER CaptureSeconds
  If set, runs non-interactively: captures this many seconds of output to
  a timestamped log under .toolchain/logs/ and returns, instead of
  blocking in the foreground. Leave unset for normal interactive use
  (Ctrl+C to exit).
#>
param(
    [string]$Port,
    [int]$CaptureSeconds = 0
)

$ProjectRoot = Split-Path -Parent $PSScriptRoot
. "$ProjectRoot\env.ps1"

$ElfPath = "$BuildRoot\xtensa-esp32s3-espidf\release\paperclock-rs"
if (-not (Test-Path $ElfPath)) {
    Write-Host "No build found at $ElfPath - run build.ps1 first." -ForegroundColor Red
    exit 1
}

if (-not $Port) {
    $dev = Get-CimInstance -ClassName Win32_PnPEntity |
        Where-Object { $_.Name -match "USB Serial Device \((COM\d+)\)" } |
        Select-Object -First 1
    if ($dev -and ($dev.Name -match "\((COM\d+)\)")) {
        $Port = $Matches[1]
        Write-Host "Auto-detected board on $Port" -ForegroundColor DarkGray
    } else {
        Write-Host "Couldn't auto-detect the board's serial port. Pass -Port COMx explicitly." -ForegroundColor Red
        Write-Host "(Looked for a device named 'USB Serial Device (COMx)' - if the board" -ForegroundColor Yellow
        Write-Host " isn't showing up at all, try unplugging/replugging the USB-C cable.)" -ForegroundColor Yellow
        exit 1
    }
}

if ($CaptureSeconds -gt 0) {
    $logDir = "$ProjectRoot\.toolchain\logs"
    New-Item -ItemType Directory -Force -Path $logDir | Out-Null
    $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
    $outLog = "$logDir\flash-$stamp.out.log"
    $errLog = "$logDir\flash-$stamp.err.log"

    $proc = Start-Process -FilePath "espflash" `
        -ArgumentList "flash --monitor --port $Port `"$ElfPath`"" `
        -RedirectStandardOutput $outLog -RedirectStandardError $errLog -PassThru -NoNewWindow
    Start-Sleep -Seconds $CaptureSeconds
    Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue

    Write-Host "--- stdout ($outLog) ---"
    Get-Content $outLog -ErrorAction SilentlyContinue
    Write-Host "--- stderr ($errLog) ---"
    Get-Content $errLog -ErrorAction SilentlyContinue
} else {
    espflash flash --monitor --port $Port $ElfPath
}

# Use PULSE from anywhere through a Cloudflare quick tunnel (no Cloudflare account).
# The https://<random>.trycloudflare.com address changes every run. Close this window to stop sharing.
#
#   cd D:\setup\rhcs ; .\scripts\remote-cloudflare.ps1
#
# Needs cloudflared.exe in D:\setup\toolchains\cloudflared\ (this script never downloads anything).
# rhcsd must already be running on port 7878.

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'remote-common.ps1')

$token = Get-OrCreate-Token
if (-not $token -or $token.Length -lt 20) { Write-Host "No usable RHCSD_TOKEN: refusing to start a tunnel." -ForegroundColor Red; exit 1 }
if ($script:TokenCreated) {
    Write-Host "Restart rhcsd so it picks up the new token (Get-Process rhcsd | Stop-Process, then .\target\debug\rhcsd.exe), then run this script again." -ForegroundColor Yellow
    exit 0
}
Assert-Rhcsd

$dir = 'D:\setup\toolchains\cloudflared'
$exe = Join-Path $dir 'cloudflared.exe'
if (-not (Test-Path $exe)) {
    Write-Host "cloudflared.exe not found at $exe" -ForegroundColor Red
    Write-Host ""
    Write-Host "1. Open  https://github.com/cloudflare/cloudflared/releases/latest"
    Write-Host "2. Download the file for Windows 64-bit: cloudflared-windows-amd64.exe"
    Write-Host "3. Create the folder $dir and save it there as cloudflared.exe"
    Write-Host "4. Run this script again."
    exit 1
}

$logDir = Join-Path $script:Root 'logs'
New-Item -ItemType Directory -Force $logDir | Out-Null
$log = Join-Path $logDir 'cloudflared.log'
Remove-Item $log, "$log.out" -ErrorAction SilentlyContinue

Write-Host "Starting the quick tunnel to http://localhost:$($script:Port) ..."
$p = Start-Process -FilePath $exe -ArgumentList @('tunnel', '--url', "http://localhost:$($script:Port)", '--no-autoupdate') `
    -NoNewWindow -PassThru -RedirectStandardError $log -RedirectStandardOutput "$log.out"
try {
    $url = $null
    for ($i = 0; $i -lt 60 -and -not $url; $i++) {
        Start-Sleep -Seconds 1
        if ($p.HasExited) { break }
        if (Test-Path $log) {
            $m = Select-String -Path $log -Pattern 'https://[a-z0-9-]+\.trycloudflare\.com' -List -ErrorAction SilentlyContinue
            if ($m) { $url = $m.Matches[0].Value }
        }
    }
    if (-not $url) {
        Write-Host "No tunnel address appeared. See $log" -ForegroundColor Red
        exit 1
    }
    Write-Host "Tunnel address: $url" -ForegroundColor Green
    Show-Pairing $url $token
    Write-Host "Keep this window open. Press Ctrl+C or close it to stop sharing." -ForegroundColor Yellow
    Wait-Process -Id $p.Id
} finally {
    if ($p -and -not $p.HasExited) { Stop-Process -Id $p.Id -Force }
}

# Use PULSE from anywhere through Tailscale (`tailscale serve`, private to your tailnet).
#
#   cd D:\setup\rhcs ; .\scripts\remote-tailscale.ps1
#
# Needs: a free Tailscale account, Tailscale installed and signed in on this PC and on the phone.
# Tradeoff: the Windows installer goes to C: (no supported D: install). This script installs nothing.
# To stop sharing:  tailscale serve reset
# First use may print a link to enable HTTPS certificates for your tailnet: open it once.

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'remote-common.ps1')

$token = Get-OrCreate-Token
if (-not $token -or $token.Length -lt 20) { Write-Host "No usable RHCSD_TOKEN: refusing to publish." -ForegroundColor Red; exit 1 }
if ($script:TokenCreated) {
    Write-Host "Restart rhcsd so it picks up the new token (Get-Process rhcsd | Stop-Process, then .\target\debug\rhcsd.exe), then run this script again." -ForegroundColor Yellow
    exit 0
}
Assert-Rhcsd

$ts = (Get-Command tailscale -ErrorAction SilentlyContinue).Source
if (-not $ts -and (Test-Path 'C:\Program Files\Tailscale\tailscale.exe')) { $ts = 'C:\Program Files\Tailscale\tailscale.exe' }
if (-not $ts) {
    Write-Host "Tailscale is not installed." -ForegroundColor Red
    Write-Host ""
    Write-Host "1. Create a free account and download the Windows app: https://tailscale.com/download"
    Write-Host "   (the installer puts it on C:; it cannot be moved to D:)"
    Write-Host "2. Sign in, and install + sign in to Tailscale on the phone with the same account."
    Write-Host "3. Run this script again."
    exit 1
}

& $ts status | Out-Null
if ($LASTEXITCODE -ne 0) { Write-Host "Tailscale is installed but not signed in. Open the Tailscale app and log in, then retry." -ForegroundColor Red; exit 1 }

Write-Host "Publishing http://localhost:$($script:Port) inside your tailnet ..."
& $ts serve --bg $script:Port
if ($LASTEXITCODE -ne 0) {
    Write-Host "'tailscale serve' failed. Check 'tailscale serve --help' for the syntax of your version." -ForegroundColor Red
    exit 1
}
$status = (& $ts serve status) -join "`n"
$m = [regex]::Match($status, 'https://[A-Za-z0-9.-]+\.ts\.net')
if (-not $m.Success) {
    Write-Host "Could not read the address from 'tailscale serve status':" -ForegroundColor Red
    Write-Host $status
    exit 1
}
Write-Host "Tailscale address: $($m.Value)" -ForegroundColor Green
Show-Pairing $m.Value $token
Write-Host "Open the link on a phone that is signed in to the same Tailscale account." -ForegroundColor Yellow
Write-Host "Stop sharing with:  tailscale serve reset" -ForegroundColor Yellow

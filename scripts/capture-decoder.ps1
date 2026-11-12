# Records what a directly cabled device (the decoder) sends, using Windows'
# built-in packet monitor (pktmon). Read-only: it only listens.
#
# Run in an ADMIN PowerShell:
#   powershell -ExecutionPolicy Bypass -File D:\setup\rhcs\scripts\capture-decoder.ps1
# Optional: -Seconds 120   -Mac 00-11-22-33-44-55
#
# Output (git-ignored): logs\decoder-capture.etl / .txt / .pcapng
param(
    [int]$Seconds = 90,
    [string]$Mac = ""
)

$root = Split-Path $PSScriptRoot -Parent
$logs = Join-Path $root "logs"
New-Item -ItemType Directory -Force $logs | Out-Null

if (-not $Mac) {
    $line = Get-Content (Join-Path $root ".env") -ErrorAction SilentlyContinue | Where-Object { $_ -match '^DECODER_MAC=' }
    if ($line) { $Mac = ($line -split '=', 2)[1].Trim() }
}
if (-not $Mac) { throw "Set DECODER_MAC in .env or pass -Mac" }
$Mac = $Mac.ToUpper().Replace(':', '-')

$etl = Join-Path $logs "decoder-capture.etl"
Remove-Item "$logs\decoder-capture.*" -ErrorAction SilentlyContinue

pktmon stop 2>$null | Out-Null
pktmon filter remove | Out-Null
pktmon filter add decoder -m $Mac | Out-Null
Write-Host "Capturing packets from/to $Mac for $Seconds s." -ForegroundColor Cyan
Write-Host "Now: power-cycle the decoder, set DHCP ON, press SCAN, View List, Firmware Update (check only)." -ForegroundColor Cyan
pktmon start --capture --pkt-size 0 --file-name $etl | Out-Null

for ($s = $Seconds; $s -gt 0; $s -= 10) {
    Write-Host ("  {0,3} s left..." -f $s)
    Start-Sleep -Seconds ([Math]::Min(10, $s))
}

pktmon stop | Out-Null
pktmon filter remove | Out-Null
pktmon etl2txt $etl --out (Join-Path $logs "decoder-capture.txt") | Out-Null
pktmon etl2pcap $etl --out (Join-Path $logs "decoder-capture.pcapng") | Out-Null
$count = (Select-String -Path (Join-Path $logs "decoder-capture.txt") -Pattern $Mac -SimpleMatch).Count
Write-Host "Done. $count packet lines mention $Mac. Files in $logs\decoder-capture.*" -ForegroundColor Green

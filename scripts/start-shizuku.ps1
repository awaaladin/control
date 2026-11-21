# Starts Shizuku on the USB-connected phone. Android stops it at every restart, so run this once after the phone restarts.
# Needs: USB debugging on, Shizuku installed and opened once. Nothing is changed on the phone except starting Shizuku.
$adb = (Get-Command adb -ErrorAction SilentlyContinue).Source
if (-not $adb) { $adb = Join-Path $env:LOCALAPPDATA 'Android\Sdk\platform-tools\adb.exe' }
if (-not (Test-Path $adb)) { Write-Host 'adb not found. Install Android platform-tools (on D:) and run again.'; exit 1 }
$path = (& $adb shell pm path moe.shizuku.privileged.api) -replace 'package:', '' -replace '\s', ''
if (-not $path) { Write-Host 'Shizuku is not installed on the phone.'; exit 1 }
$lib = ($path -replace '/base\.apk$', '') + '/lib/arm64/libshizuku.so'
& $adb shell $lib
Write-Host 'Done. Open PULSE > Boost > Privileged mode: it should say Ready.'

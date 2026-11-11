# Dot-source this in a terminal before building:   . D:\setup\rhcs\scripts\dev.ps1
# Puts the MinGW toolchain (on the flash drive) first on PATH.
#
# Code, build output (target/) and MinGW all live on D:. The Cargo crate cache
# (CARGO_HOME) has to stay on C:, because crate archives contain 1970
# timestamps and FAT32 cannot store dates before 1980 (os error 87).
$mingw = "D:\setup\toolchains\mingw64\bin"
if (-not (Test-Path $mingw)) { Write-Warning "MinGW not found at $mingw"; return }
if (-not ($env:PATH -split ';' -contains $mingw)) { $env:PATH = "$mingw;$env:PATH" }
Remove-Item Env:CARGO_HOME -ErrorAction SilentlyContinue
# FAT32 has no hard links, so the incremental cache only adds copying. Off = faster here.
$env:CARGO_INCREMENTAL = "0"
Set-Location D:\setup\rhcs
Write-Host "rhcs dev shell: gcc $((gcc -dumpversion) 2>$null)" -ForegroundColor Cyan

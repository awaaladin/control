@echo off
rem Stops the PULSE engine of THIS install (not other programs named rhcsd.exe).
setlocal
set "RHCS_EXE=%~dp0rhcsd.exe"
powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "Get-Process -Name rhcsd -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $env:RHCS_EXE } | Stop-Process -Force"
exit /b 0

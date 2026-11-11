# Run once in an ADMIN PowerShell:
#   powershell -ExecutionPolicy Bypass -File D:\setup\rhcs\scripts\ethernet-lan-only.ps1
#
# Makes Windows prefer Wi-Fi (phone hotspot) for internet, while the Ethernet
# cable still reaches the router at 192.168.0.1. Lower metric = preferred.
# Undo with:  ... -File ethernet-lan-only.ps1 -Undo
param([switch]$Undo)

if ($Undo) {
    Set-NetIPInterface -InterfaceAlias "Ethernet" -AddressFamily IPv4 -AutomaticMetric Enabled
    Write-Host "Ethernet metric back to automatic." -ForegroundColor Yellow
} else {
    Set-NetIPInterface -InterfaceAlias "Ethernet" -AddressFamily IPv4 -AutomaticMetric Disabled -InterfaceMetric 500
    Write-Host "Ethernet metric = 500: internet stays on Wi-Fi, router still reachable on 192.168.0.1." -ForegroundColor Green
}
Get-NetIPInterface -AddressFamily IPv4 | Where-Object InterfaceAlias -in 'Ethernet','WiFi' |
    Format-Table InterfaceAlias, InterfaceMetric, AutomaticMetric, ConnectionState -AutoSize

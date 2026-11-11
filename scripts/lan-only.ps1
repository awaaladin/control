# Make a network adapter "local only": Windows stops sending internet traffic
# through it, but you can still reach devices on that network (e.g. the
# router at 192.168.0.1). Lower metric = preferred, so we raise it to 500.
#
# Run in an ADMIN PowerShell:
#   powershell -ExecutionPolicy Bypass -File scripts\lan-only.ps1 -Interface WiFi
#   powershell -ExecutionPolicy Bypass -File scripts\lan-only.ps1 -Interface Ethernet
# Undo:
#   powershell -ExecutionPolicy Bypass -File scripts\lan-only.ps1 -Interface WiFi -Undo
#
# Typical setup: Wi-Fi on the router (no data plan), internet from the phone
# over USB tethering. Run with -Interface WiFi.
param(
    [string]$Interface = "Ethernet",
    [switch]$Undo
)

if ($Undo) {
    Set-NetIPInterface -InterfaceAlias $Interface -AddressFamily IPv4 -AutomaticMetric Enabled
    Write-Host "$Interface metric back to automatic." -ForegroundColor Yellow
} else {
    Set-NetIPInterface -InterfaceAlias $Interface -AddressFamily IPv4 -AutomaticMetric Disabled -InterfaceMetric 500
    Write-Host "$Interface metric = 500: used for its local network only, not for internet." -ForegroundColor Green
}
Get-NetIPInterface -AddressFamily IPv4 -ConnectionState Connected |
    Sort-Object InterfaceMetric |
    Format-Table InterfaceAlias, InterfaceMetric, AutomaticMetric -AutoSize

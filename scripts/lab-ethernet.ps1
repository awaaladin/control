# Prepare the laptop's Ethernet port for a device plugged in directly (e.g. the
# decoder), so scripts\decoder-lab.ps1 can act as its DHCP + DNS server.
#
# Run in an ADMIN PowerShell:
#   powershell -ExecutionPolicy Bypass -File D:\setup\rhcs\scripts\lab-ethernet.ps1
# Undo (back to normal automatic Ethernet):
#   powershell -ExecutionPolicy Bypass -File D:\setup\rhcs\scripts\lab-ethernet.ps1 -Undo
#
# Sets Ethernet to 192.168.50.1/24 with NO gateway (internet keeps using your
# other connection) and allows DHCP/DNS/SSDP traffic in on Ethernet only.
param(
    [string]$Interface = "Ethernet",
    [string]$Ip = "192.168.50.1",
    [int]$Prefix = 24,
    [switch]$Undo
)

$rule = "rhcs lab (decoder on $Interface)"
if ($Undo) {
    Get-NetIPAddress -InterfaceAlias $Interface -AddressFamily IPv4 -ErrorAction SilentlyContinue |
        Remove-NetIPAddress -Confirm:$false -ErrorAction SilentlyContinue
    Set-NetIPInterface -InterfaceAlias $Interface -Dhcp Enabled
    Remove-NetFirewallRule -DisplayName $rule -ErrorAction SilentlyContinue
    Write-Host "$Interface back to automatic (DHCP). Lab firewall rule removed." -ForegroundColor Yellow
} else {
    Set-NetIPInterface -InterfaceAlias $Interface -Dhcp Disabled
    Get-NetIPAddress -InterfaceAlias $Interface -AddressFamily IPv4 -ErrorAction SilentlyContinue |
        Remove-NetIPAddress -Confirm:$false -ErrorAction SilentlyContinue
    New-NetIPAddress -InterfaceAlias $Interface -IPAddress $Ip -PrefixLength $Prefix | Out-Null
    Remove-NetFirewallRule -DisplayName $rule -ErrorAction SilentlyContinue
    New-NetFirewallRule -DisplayName $rule -Direction Inbound -Protocol UDP -LocalPort 53, 67, 1900 `
        -InterfaceAlias $Interface -Action Allow | Out-Null
    Write-Host "$Interface = $Ip/$Prefix (no gateway). UDP 53/67/1900 allowed in on $Interface." -ForegroundColor Green
}
Get-NetIPAddress -InterfaceAlias $Interface -AddressFamily IPv4 -ErrorAction SilentlyContinue |
    Format-Table InterfaceAlias, IPAddress, PrefixLength, PrefixOrigin -AutoSize

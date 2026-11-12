# Observe a device plugged straight into the laptop's Ethernet port.
# Acts as its DHCP server and DNS server and listens for UPnP/SSDP, logging
# everything it says. Read-only: it hands out one IP address and relays DNS
# answers from a real resolver; it never serves files or firmware.
#
# Prerequisite (admin, once):  scripts\lab-ethernet.ps1
# Run (normal PowerShell):     powershell -ExecutionPolicy Bypass -File scripts\decoder-lab.ps1
# Stop with Ctrl+C. Log: logs\decoder-lab-<date>.log
param(
    [string]$ServerIp = "192.168.50.1",
    [string]$ClientIp = "192.168.50.10",
    [string]$Upstream = "8.8.8.8",
    # Answer DHCP only for this MAC. Defaults to DECODER_MAC from .env.
    [string]$OnlyMac = ""
)

$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
New-Item -ItemType Directory -Force (Join-Path $root "logs") | Out-Null
$logFile = Join-Path $root ("logs\decoder-lab-{0}.log" -f (Get-Date -Format yyyy-MM-dd))
if (-not $OnlyMac) {
    $line = Get-Content (Join-Path $root ".env") -ErrorAction SilentlyContinue | Where-Object { $_ -match '^DECODER_MAC=' }
    if ($line) { $OnlyMac = ($line -split '=', 2)[1].Trim() }
}
if (-not $OnlyMac) { throw "Set DECODER_MAC in .env or pass -OnlyMac; refusing to answer DHCP for any device." }

function Log([string]$kind, [string]$msg, [string]$color = "Gray") {
    $line = "{0} [{1}] {2}" -f (Get-Date -Format HH:mm:ss), $kind, $msg
    Write-Host $line -ForegroundColor $color
    Add-Content -Path $logFile -Value $line -Encoding UTF8
}

$server = [System.Net.IPAddress]::Parse($ServerIp)
$clientBytes = [System.Net.IPAddress]::Parse($ClientIp).GetAddressBytes()
$serverBytes = $server.GetAddressBytes()

# ---- sockets -------------------------------------------------------------
$dhcp = New-Object System.Net.Sockets.UdpClient
$dhcp.Client.SetSocketOption("Socket", "ReuseAddress", $true)
$dhcp.EnableBroadcast = $true
# Bound to all addresses: Windows doesn't reliably deliver 0.0.0.0 -> 255.255.255.255
# broadcasts (what a DHCP client without an address sends) to an address-bound socket.
$dhcp.Client.Bind((New-Object System.Net.IPEndPoint([System.Net.IPAddress]::Any, 67)))

$dns = New-Object System.Net.Sockets.UdpClient (New-Object System.Net.IPEndPoint($server, 53))

$ssdp = New-Object System.Net.Sockets.UdpClient
$ssdp.Client.SetSocketOption("Socket", "ReuseAddress", $true)
$ssdp.Client.Bind((New-Object System.Net.IPEndPoint([System.Net.IPAddress]::Any, 1900)))
$ssdp.JoinMulticastGroup([System.Net.IPAddress]::Parse("239.255.255.250"), $server)

Log "lab" "listening on $ServerIp (DHCP 67, DNS 53, SSDP 1900); will offer $ClientIp. Log: $logFile" "Cyan"
Log "lab" "On the decoder: IP Settings -> DHCP ON, then try SCAN, View List, Firmware Update (check only)." "Cyan"

# ---- DHCP ----------------------------------------------------------------
function Get-Options([byte[]]$p) {
    $opts = @{}
    $i = 240
    while ($i -lt $p.Length) {
        $code = $p[$i]
        if ($code -eq 255) { break }
        if ($code -eq 0) { $i++; continue }
        $len = $p[$i + 1]
        $opts[[int]$code] = [byte[]]$p[($i + 2)..($i + 1 + $len)]
        $i += 2 + $len
    }
    $opts
}

function New-DhcpReply([byte[]]$req, [byte]$msgType) {
    $r = New-Object byte[] 300
    $r[0] = 2; $r[1] = 1; $r[2] = 6
    [Array]::Copy($req, 4, $r, 4, 4)      # xid
    [Array]::Copy($req, 10, $r, 10, 2)    # flags
    [Array]::Copy($clientBytes, 0, $r, 16, 4)   # yiaddr
    [Array]::Copy($serverBytes, 0, $r, 20, 4)   # siaddr
    [Array]::Copy($req, 28, $r, 28, 16)   # chaddr
    $r[236] = 99; $r[237] = 130; $r[238] = 83; $r[239] = 99   # magic cookie
    $o = 240
    $opts = @(
        @(53, @($msgType)),
        @(54, $serverBytes),
        @(51, @(0, 0, 14, 16)),            # lease 3600 s
        @(1, @(255, 255, 255, 0)),
        @(3, $serverBytes),
        @(6, $serverBytes)
    )
    foreach ($opt in $opts) {
        $r[$o] = [byte]$opt[0]; $r[$o + 1] = [byte]$opt[1].Count
        for ($k = 0; $k -lt $opt[1].Count; $k++) { $r[$o + 2 + $k] = [byte]$opt[1][$k] }
        $o += 2 + $opt[1].Count
    }
    $r[$o] = 255
    , $r
}

function Handle-Dhcp([byte[]]$p) {
    if ($p.Length -lt 241 -or $p[0] -ne 1) { return }
    $opts = Get-Options $p
    $type = if ($opts.ContainsKey(53)) { $opts[53][0] } else { 0 }
    $mac = ($p[28..33] | ForEach-Object { $_.ToString("x2") }) -join ":"
    $names = @{ 1 = "DISCOVER"; 3 = "REQUEST"; 4 = "DECLINE"; 7 = "RELEASE"; 8 = "INFORM" }
    $info = "DHCP $($names[[int]$type]) from $mac"
    if ($opts.ContainsKey(12)) { $info += " | hostname: " + [Text.Encoding]::ASCII.GetString($opts[12]) }
    if ($opts.ContainsKey(60)) { $info += " | vendor class: " + [Text.Encoding]::ASCII.GetString($opts[60]) }
    if ($opts.ContainsKey(55)) { $info += " | asks for options: " + ($opts[55] -join ",") }
    if ($opts.ContainsKey(50)) { $info += " | wants IP: " + (([System.Net.IPAddress]::new($opts[50])).ToString()) }
    Log "dhcp" $info "Green"

    $reply = switch ($type) { 1 { 2 } 3 { 5 } default { 0 } }
    if ($reply -eq 0) { return }
    # Only ever answer the device we're studying, never other clients on other networks.
    if ($OnlyMac -and $mac -ne $OnlyMac.ToLower()) {
        Log "dhcp" "  ignored (not $OnlyMac)" "DarkGray"
        return
    }
    $bytes = New-DhcpReply $p ([byte]$reply)
    # Limited broadcast may leave through another adapter on Windows; the subnet
    # broadcast can only leave through this one. DHCP clients accept either.
    $subnetBcast = [System.Net.IPAddress]::Parse($ServerIp.Substring(0, $ServerIp.LastIndexOf(".")) + ".255")
    foreach ($target in @([System.Net.IPAddress]::Broadcast, $subnetBcast)) {
        [void]$dhcp.Send($bytes, $bytes.Length, (New-Object System.Net.IPEndPoint($target, 68)))
    }
    Log "dhcp" ("sent {0} -> {1}" -f $(if ($reply -eq 2) { "OFFER" } else { "ACK" }), $ClientIp)
}

# ---- DNS -----------------------------------------------------------------
function Read-QName([byte[]]$p) {
    $labels = @(); $i = 12
    while ($i -lt $p.Length -and $p[$i] -ne 0) {
        $len = $p[$i]
        $labels += [Text.Encoding]::ASCII.GetString($p, $i + 1, $len)
        $i += 1 + $len
    }
    $qtype = if ($i + 2 -lt $p.Length) { ($p[$i + 1] * 256) + $p[$i + 2] } else { 0 }
    @(($labels -join "."), $qtype)
}

function Handle-Dns([byte[]]$p, $from) {
    if ($p.Length -lt 17) { return }
    $q = Read-QName $p
    $types = @{ 1 = "A"; 28 = "AAAA"; 12 = "PTR"; 33 = "SRV"; 16 = "TXT"; 5 = "CNAME" }
    $t = if ($types.ContainsKey([int]$q[1])) { $types[[int]$q[1]] } else { "type $($q[1])" }
    Log "dns" ("{0} asks {1} ({2})" -f $from.Address, $q[0], $t) "Yellow"
    try {
        $up = New-Object System.Net.Sockets.UdpClient
        $up.Client.ReceiveTimeout = 2500
        [void]$up.Send($p, $p.Length, $Upstream, 53)
        $any = New-Object System.Net.IPEndPoint([System.Net.IPAddress]::Any, 0)
        $answer = $up.Receive([ref]$any)
        $up.Close()
        [void]$dns.Send($answer, $answer.Length, $from)
    } catch {
        Log "dns" "  (no upstream answer: $($_.Exception.Message))"
    }
}

# ---- SSDP ----------------------------------------------------------------
function Handle-Ssdp([byte[]]$p, $from) {
    if (-not $from.Address.ToString().StartsWith($ServerIp.Substring(0, $ServerIp.LastIndexOf(".")))) { return }
    $text = [Text.Encoding]::ASCII.GetString($p)
    $lines = $text -split "`r?`n"
    $keep = $lines | Where-Object { $_ -match '^(M-SEARCH|NOTIFY|ST:|NT:|USER-AGENT:|SERVER:|LOCATION:|MAN:)' }
    Log "ssdp" ("{0}: {1}" -f $from.Address, ($keep -join " | ")) "Magenta"
}

# ---- loop ----------------------------------------------------------------
$lastPing = Get-Date
$wasUp = $false
try {
    while ($true) {
        $idle = $true
        foreach ($pair in @(@($dhcp, "dhcp"), @($dns, "dns"), @($ssdp, "ssdp"))) {
            $sock = $pair[0]
            while ($sock.Available -gt 0) {
                $idle = $false
                $from = New-Object System.Net.IPEndPoint([System.Net.IPAddress]::Any, 0)
                $data = $sock.Receive([ref]$from)
                switch ($pair[1]) {
                    "dhcp" { Handle-Dhcp $data }
                    "dns" { Handle-Dns $data $from }
                    "ssdp" { Handle-Ssdp $data $from }
                }
            }
        }
        if (((Get-Date) - $lastPing).TotalSeconds -ge 10) {
            $lastPing = Get-Date
            $up = Test-Connection $ClientIp -Count 1 -Quiet -ErrorAction SilentlyContinue
            if ($up -ne $wasUp) {
                Log "link" ("{0} {1}" -f $ClientIp, $(if ($up) { "answers ping" } else { "does not answer ping" })) $(if ($up) { "Green" } else { "DarkGray" })
                $wasUp = $up
            }
        }
        if ($idle) { Start-Sleep -Milliseconds 100 }
    }
} finally {
    $dhcp.Close(); $dns.Close(); $ssdp.Close()
    Log "lab" "stopped"
}

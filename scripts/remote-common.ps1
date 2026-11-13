# Shared helpers for remote-cloudflare.ps1 and remote-tailscale.ps1 (dot-sourced).
# Nothing here starts a tunnel or installs anything.

$script:Root = Split-Path -Parent $PSScriptRoot
$script:EnvFile = Join-Path $script:Root '.env'
$script:PulseUrl = 'https://rhcs-pulse.vercel.app'
$script:Port = 7878

function New-RandomToken([int]$Length = 32) {
    $chars = 'ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789'
    $rng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
    $buf = New-Object byte[] 1
    $sb = New-Object System.Text.StringBuilder
    # rejection sampling so every character is equally likely
    $limit = 256 - (256 % $chars.Length)
    while ($sb.Length -lt $Length) {
        $rng.GetBytes($buf)
        if ($buf[0] -lt $limit) { [void]$sb.Append($chars[$buf[0] % $chars.Length]) }
    }
    $rng.Dispose()
    $sb.ToString()
}

# Returns the token from .env, creating one (>= 20 chars) if it is missing or short.
# Sets $script:TokenCreated when a new one was written.
function Get-OrCreate-Token {
    $script:TokenCreated = $false
    $lines = @()
    if (Test-Path $script:EnvFile) { $lines = [System.IO.File]::ReadAllLines($script:EnvFile) }
    $idx = -1
    $current = ''
    for ($i = 0; $i -lt $lines.Count; $i++) {
        if ($lines[$i] -match '^\s*RHCSD_TOKEN\s*=\s*(.*)$') { $idx = $i; $current = $Matches[1].Trim().Trim('"').Trim("'") }
    }
    if ($current.Length -ge 20) { return $current }

    $new = New-RandomToken 32
    if ($idx -ge 0) { $lines[$idx] = "RHCSD_TOKEN=$new" } else { $lines = @($lines) + "RHCSD_TOKEN=$new" }
    [System.IO.File]::WriteAllLines($script:EnvFile, $lines, (New-Object System.Text.UTF8Encoding($false)))
    $script:TokenCreated = $true
    Write-Host "RHCSD_TOKEN was missing or shorter than 20 characters: wrote a new random one to .env." -ForegroundColor Yellow
    return $new
}

# rhcsd reads .env only at start. Make sure the running one already has a token,
# otherwise a tunnel would just return 403.
function Test-RhcsdHasToken {
    try {
        $h = Invoke-RestMethod -Uri "http://127.0.0.1:$($script:Port)/api/health" -TimeoutSec 4
        return [bool]$h.auth_required
    } catch {
        return $null
    }
}

function Assert-Rhcsd {
    $r = Test-RhcsdHasToken
    if ($null -eq $r) {
        Write-Host "rhcsd does not answer on port $($script:Port). Start it first:  cd D:\setup\rhcs ; .\target\debug\rhcsd.exe" -ForegroundColor Red
        exit 1
    }
    if (-not $r) {
        Write-Host "rhcsd is running but started before the token existed. Stop it (Get-Process rhcsd | Stop-Process), start it again, then re-run this script." -ForegroundColor Red
        exit 1
    }
}

function Get-PairingLink([string]$HubUrl, [string]$Token) {
    "$($script:PulseUrl)/#hub=$([uri]::EscapeDataString($HubUrl))&token=$([uri]::EscapeDataString($Token))"
}

function Show-Pairing([string]$HubUrl, [string]$Token) {
    $link = Get-PairingLink $HubUrl $Token
    Write-Host ""
    Write-Host "Pairing link (contains your access token: send it only to yourself, delete it afterwards):" -ForegroundColor Cyan
    Write-Host $link
    try { Set-Clipboard -Value $link; Write-Host "(copied to the clipboard)" -ForegroundColor DarkGray } catch { }
    Write-Host ""
}

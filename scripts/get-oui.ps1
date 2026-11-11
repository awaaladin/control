# Downloads the official IEEE MAC-prefix registry (public data, ~4 MB)
# so `rhcs` can show device manufacturers.
$dest = Join-Path $PSScriptRoot "..\data\oui.csv"
New-Item -ItemType Directory -Force (Split-Path $dest) | Out-Null
curl.exe -L --fail -o $dest "https://standards-oui.ieee.org/oui/oui.csv"
if ($LASTEXITCODE -eq 0) { Write-Host "saved $((Get-Item $dest).Length / 1MB) MB to $dest" }

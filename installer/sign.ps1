# Optional code signing. Skipped (successfully) when the secrets are absent, so the
# workflow works without a certificate. An unsigned installer triggers Windows SmartScreen.
#
# To enable: add repository secrets
#   WINDOWS_CERT_PFX_BASE64  the .pfx code-signing certificate, base64-encoded
#   WINDOWS_CERT_PASSWORD    its password
# The certificate must be bought from a certificate authority; nothing is generated here.
param([Parameter(Mandatory = $true)][string[]]$Files)

if (-not $env:WINDOWS_CERT_PFX_BASE64 -or -not $env:WINDOWS_CERT_PASSWORD) {
    Write-Host "No code-signing secrets set: leaving the files unsigned."
    exit 0
}

$signtool = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin" -Recurse -Filter signtool.exe -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -match '\\x64\\' } | Sort-Object FullName -Descending | Select-Object -First 1
if (-not $signtool) { throw "signtool.exe not found" }

$pfx = Join-Path $env:RUNNER_TEMP "codesign.pfx"
[IO.File]::WriteAllBytes($pfx, [Convert]::FromBase64String($env:WINDOWS_CERT_PFX_BASE64))
try {
    foreach ($f in $Files) {
        & $signtool.FullName sign /f $pfx /p $env:WINDOWS_CERT_PASSWORD /fd sha256 /tr http://timestamp.digicert.com /td sha256 $f
        if ($LASTEXITCODE -ne 0) { throw "signing failed for $f" }
    }
} finally {
    Remove-Item $pfx -Force -ErrorAction SilentlyContinue
}

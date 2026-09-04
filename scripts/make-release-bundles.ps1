# Build bản phát hành ra một ổ khác (ổ chứa dự án thường chật), rồi gom bộ cài NSIS, exe
# portable và checksum vào release\ để đính lên GitHub Release.
# Dùng: powershell -File scripts\make-release-bundles.ps1 [-Out C:\tmp\slclean-release-out]
param([string]$Out = 'C:\tmp\slclean-release-out')
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$env:CARGO_TARGET_DIR = $Out
Push-Location $root
try {
  npx tauri build
  if ($LASTEXITCODE -ne 0) { throw "tauri build failed with exit code $LASTEXITCODE" }
} finally {
  Pop-Location
}
$ver = (Get-Content -LiteralPath (Join-Path $root 'src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json).version
$rel = Join-Path $root 'release'
New-Item -ItemType Directory -Force -Path $rel | Out-Null
Get-ChildItem -LiteralPath $rel -File | Remove-Item
Copy-Item -LiteralPath (Join-Path $Out "release\bundle\nsis\SLClean_${ver}_x64-setup.exe") -Destination $rel
Copy-Item -LiteralPath (Join-Path $Out 'release\slclean.exe') -Destination (Join-Path $rel 'SLClean-portable.exe')
$lines = Get-ChildItem -LiteralPath $rel -File | ForEach-Object {
  '{0}  {1}' -f (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLower(), $_.Name
}
Set-Content -LiteralPath (Join-Path $rel 'SHA256SUMS.txt') -Value $lines -Encoding ascii
Write-Output "version=$ver"
Get-ChildItem -LiteralPath $rel -File | ForEach-Object { '{0,10}  {1}' -f $_.Length, $_.Name }

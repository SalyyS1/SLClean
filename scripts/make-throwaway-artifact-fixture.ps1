# Tạo một dự án giả trong D:\tmp với thư mục cài gói 3 MB để thử xoá thật qua app.
$root = 'D:\tmp\slclean-e2e-fake-project'
$nm = Join-Path $root 'node_modules\pkg'
New-Item -ItemType Directory -Force -Path $nm | Out-Null
Set-Content -LiteralPath (Join-Path $root 'package.json') -Value '{"name":"slclean-e2e"}'
$bytes = New-Object byte[] 3000000
(New-Object Random).NextBytes($bytes)
[IO.File]::WriteAllBytes((Join-Path $nm 'blob.bin'), $bytes)
Set-Content -LiteralPath (Join-Path $nm 'index.js') -Value 'module.exports = 1'
Write-Output "fixture: $root"
Get-ChildItem -LiteralPath $root -Recurse -File | ForEach-Object { '{0,10}  {1}' -f $_.Length, $_.FullName }

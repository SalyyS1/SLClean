# Chạy SLClean với cổng DevTools của WebView2 mở để script bên ngoài điều khiển UI thật.
# Mặc định mở bản debug; truyền -Exe để thử bản release hoặc bản đã cài.
param([int]$Port = 9223, [string]$Exe = '')
if (-not $Exe) { $Exe = Join-Path $PSScriptRoot '..\src-tauri\target\debug\slclean.exe' }
if (-not (Test-Path -LiteralPath $Exe)) { Write-Output "exe not found: $Exe"; exit 1 }
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=$Port"
$p = Start-Process -FilePath $Exe -PassThru
Write-Output "pid=$($p.Id) port=$Port exe=$Exe"

# Xoá thư mục build của chính SLClean (src-tauri\target) để lấy lại chỗ trên D trước khi build lại.
$dir = Join-Path $PSScriptRoot '..\src-tauri\target'
if (Test-Path -LiteralPath $dir) {
    Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction Continue
    Write-Output "removed $dir"
}
Get-PSDrive D | Select-Object Name, @{n = 'FreeGB'; e = { [math]::Round($_.Free / 1GB, 2) } } | Format-Table -AutoSize

# Tạo ba fixture để thử tab Ứng dụng và Thư mục thừa trên app thật, không đụng app nào của bạn:
#  - ZzFixtureDead: mục Uninstall (HKCU) có trình gỡ đã mất + thư mục 1 MB còn sót → "mục chết".
#  - ZzFixtureLive: mục Uninstall (HKCU) mà trình gỡ là reg.exe tự xoá khoá của nó → gỡ thật được,
#    thư mục cài còn lại để app đề nghị dọn.
#  - ZzOrphanFixtureApp: thư mục trong %LOCALAPPDATA% không app nào nhận → "thư mục thừa".
# Tên bắt đầu bằng "Zz" để không trùng từ khoá của app nào đang cài. Dùng -Remove để dọn fixture.
param([switch]$Remove)
$un = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall'
$dead = Join-Path $env:LOCALAPPDATA 'ZzFixtureDead'
$live = Join-Path $env:LOCALAPPDATA 'ZzFixtureLive'
$orphan = Join-Path $env:LOCALAPPDATA 'ZzOrphanFixtureApp'
if ($Remove) {
  foreach ($k in 'ZzFixtureDead', 'ZzFixtureLive') { Remove-Item -Path (Join-Path $un $k) -Recurse -Force -ErrorAction SilentlyContinue }
  foreach ($d in $dead, $live, $orphan) { Remove-Item -LiteralPath $d -Recurse -Force -ErrorAction SilentlyContinue }
  Write-Output 'fixtures removed'
  exit 0
}
foreach ($d in $dead, $live, $orphan) {
  New-Item -ItemType Directory -Force -Path $d | Out-Null
  $bytes = New-Object byte[] 1000000
  (New-Object Random).NextBytes($bytes)
  [IO.File]::WriteAllBytes((Join-Path $d 'blob.bin'), $bytes)
}
$k = New-Item -Path (Join-Path $un 'ZzFixtureDead') -Force
Set-ItemProperty -Path $k.PSPath -Name DisplayName -Value 'Zz Fixture Dead App'
Set-ItemProperty -Path $k.PSPath -Name Publisher -Value 'SLClean test'
Set-ItemProperty -Path $k.PSPath -Name DisplayVersion -Value '0.0.1'
Set-ItemProperty -Path $k.PSPath -Name InstallLocation -Value $dead
Set-ItemProperty -Path $k.PSPath -Name UninstallString -Value ('"' + (Join-Path $dead 'uninstall.exe') + '"')
$k = New-Item -Path (Join-Path $un 'ZzFixtureLive') -Force
Set-ItemProperty -Path $k.PSPath -Name DisplayName -Value 'Zz Fixture Live App'
Set-ItemProperty -Path $k.PSPath -Name Publisher -Value 'SLClean test'
Set-ItemProperty -Path $k.PSPath -Name DisplayVersion -Value '0.0.2'
Set-ItemProperty -Path $k.PSPath -Name InstallLocation -Value $live
Set-ItemProperty -Path $k.PSPath -Name InstallDate -Value '20260101'
$reg = Join-Path $env:SystemRoot 'System32\reg.exe'
Set-ItemProperty -Path $k.PSPath -Name UninstallString -Value ('"' + $reg + '" delete "HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\ZzFixtureLive" /f')
Write-Output "dead=$dead"
Write-Output "live=$live"
Write-Output "orphan=$orphan"

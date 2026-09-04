# Tạo hai mục khởi động giả dưới HKCU để thử tab Khởi động trên app thật:
#   ZzStartupFixture       trỏ tới notepad có thật  → tắt/bật được
#   ZzStartupFixtureBroken trỏ tới exe không tồn tại → phải hiện nhãn "hỏng" và xoá được
# Không đụng HKLM (cần admin) và không đụng mục thật nào. Dọn bằng -Remove.
param([switch]$Remove)

$run  = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$appr = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run'
$live = 'ZzStartupFixture'
$dead = 'ZzStartupFixtureBroken'

if ($Remove) {
  foreach ($n in @($live, $dead)) {
    Remove-ItemProperty -Path $run  -Name $n -ErrorAction SilentlyContinue
    Remove-ItemProperty -Path $appr -Name $n -ErrorAction SilentlyContinue
  }
  Write-Output "removed live=$(-not (Get-ItemProperty $run -Name $live -ErrorAction SilentlyContinue)) dead=$(-not (Get-ItemProperty $run -Name $dead -ErrorAction SilentlyContinue))"
  exit
}

if (-not (Test-Path $appr)) { New-Item -Path $appr -Force | Out-Null }
Set-ItemProperty -Path $run -Name $live -Value "$env:SystemRoot\System32\notepad.exe"
Set-ItemProperty -Path $run -Name $dead -Value '"C:\Zz\NoSuchFolder\gone.exe" --autostart'
# Không ghi cờ approved: không có cờ = Windows coi là đang bật, đúng trạng thái đầu bài.
Remove-ItemProperty -Path $appr -Name $live -ErrorAction SilentlyContinue
Remove-ItemProperty -Path $appr -Name $dead -ErrorAction SilentlyContinue
Write-Output "created live=$live dead=$dead"

# Chạy bản debug của SLClean, đợi cửa sổ lên, chụp ảnh cửa sổ rồi để app chạy tiếp (in PID).
# Dùng: powershell -File launch-app-and-capture-window.ps1 [-Wait giây] [-Kill]
param([int]$Wait = 25, [switch]$Kill)

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class Win {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@

$exe = Join-Path $PSScriptRoot '..\src-tauri\target\debug\slclean.exe'
$shots = Join-Path $PSScriptRoot 'shots'
New-Item -ItemType Directory -Force -Path $shots | Out-Null

$size = (Get-Item -LiteralPath $exe).Length / 1MB
Write-Output ('exe: {0} ({1:N1} MB)' -f $exe, $size)
$dir = (Get-ChildItem -LiteralPath (Split-Path (Split-Path $exe)) -Recurse -File -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum / 1GB
Write-Output ('build dir: {0:N2} GB' -f $dir)

$p = Start-Process -FilePath $exe -PassThru
Write-Output "pid: $($p.Id)"
Start-Sleep -Seconds $Wait
$p.Refresh()
if ($p.HasExited) { Write-Output "app exited with code $($p.ExitCode)"; exit 1 }

$h = $p.MainWindowHandle
if ($h -eq 0) { Write-Output "no main window yet"; exit 1 }
[Win]::ShowWindow($h, 9) | Out-Null
[Win]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 800
$r = New-Object Win+RECT
[Win]::GetWindowRect($h, [ref]$r) | Out-Null
$w = $r.R - $r.L; $hgt = $r.B - $r.T
Write-Output "window: ${w}x${hgt} at $($r.L),$($r.T) title='$($p.MainWindowTitle)'"
$bmp = New-Object System.Drawing.Bitmap $w, $hgt
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($r.L, $r.T, 0, 0, $bmp.Size)
$out = Join-Path $shots 'slclean-real-window.png'
$bmp.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
Write-Output "saved: $out"
if ($Kill) { Stop-Process -Id $p.Id -Force; Write-Output "killed" }

# Chụp cửa sổ SLClean đang chạy bằng PrintWindow (không phụ thuộc cửa sổ nào đang ở trên).
param([int]$ProcId, [string]$Name = 'slclean-real-window.png')

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class WinCap {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@

[WinCap]::SetProcessDPIAware() | Out-Null   # màn hình 1920x1080 @150%: lấy kích thước pixel thật
$p = if ($ProcId) { Get-Process -Id $ProcId } else { Get-Process slclean | Select-Object -First 1 }
if (-not $p) { Write-Output "slclean not running"; exit 1 }
$h = $p.MainWindowHandle
$r = New-Object WinCap+RECT
[WinCap]::GetWindowRect($h, [ref]$r) | Out-Null
$w = $r.R - $r.L; $hgt = $r.B - $r.T
$bmp = New-Object System.Drawing.Bitmap $w, $hgt
$g = [System.Drawing.Graphics]::FromImage($bmp)
$hdc = $g.GetHdc()
$ok = [WinCap]::PrintWindow($h, $hdc, 2)   # 2 = PW_RENDERFULLCONTENT (cần cho WebView2)
$g.ReleaseHdc($hdc)
$out = Join-Path $PSScriptRoot "shots\$Name"
$bmp.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
Write-Output "pid=$($p.Id) ${w}x${hgt} printwindow=$ok saved=$out"

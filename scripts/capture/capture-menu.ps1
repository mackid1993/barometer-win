param([string]$Out)
$ErrorActionPreference = "Continue"
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
Add-Type @"
using System; using System.Runtime.InteropServices;
public class EM { [DllImport("user32.dll")] public static extern bool SetProcessDPIAware(); [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y); [DllImport("user32.dll")] public static extern void mouse_event(int f, int dx, int dy, int d, IntPtr e); [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowW(string c, string n); [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r); [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; } }
"@
[EM]::SetProcessDPIAware() | Out-Null
$dir = Split-Path $Out
$lines = @()
function Snap($name, $x, $y, $w, $h) { $b = New-Object System.Drawing.Bitmap($w, $h); $g = [System.Drawing.Graphics]::FromImage($b); $g.CopyFromScreen($x, $y, 0, 0, (New-Object System.Drawing.Size($w, $h))); $b.Save("$dir\$name.png", [System.Drawing.Imaging.ImageFormat]::Png); $g.Dispose(); $b.Dispose() }
$s = [EM]::FindWindowW("BarometerStrip", $null); $sr = New-Object EM+RECT; [EM]::GetWindowRect($s, [ref]$sr) | Out-Null
$lines += "strip: $($sr.L),$($sr.T)-$($sr.R),$($sr.B)"
$x = $sr.L + [int](($sr.R - $sr.L) * 0.5); $y = $sr.T + 24
[EM]::SetCursorPos($x, $y) | Out-Null; Start-Sleep -Milliseconds 150
[EM]::mouse_event(8,0,0,0,[IntPtr]::Zero); Start-Sleep -Milliseconds 50; [EM]::mouse_event(16,0,0,0,[IntPtr]::Zero)
Start-Sleep -Milliseconds 900
$m = [EM]::FindWindowW("#32768", $null); $mr = New-Object EM+RECT; [EM]::GetWindowRect($m, [ref]$mr) | Out-Null
$lines += "menu: $($mr.L),$($mr.T)-$($mr.R),$($mr.B)"
if ($mr.R -gt $mr.L) {
  # hover the second row so the plate shows
  [EM]::SetCursorPos([int](($mr.L + $mr.R) / 2), $mr.T + [int](($mr.B - $mr.T) * 0.4)) | Out-Null
  Start-Sleep -Milliseconds 500
  Snap "menu" ($mr.L - 12) ($mr.T - 12) ($mr.R - $mr.L + 24) ($mr.B - $mr.T + 24)
}
[System.Windows.Forms.SendKeys]::SendWait("{ESC}"); Start-Sleep -Milliseconds 300
[EM]::SetCursorPos(1200, 800) | Out-Null
$lines | Out-File -FilePath $Out -Encoding utf8

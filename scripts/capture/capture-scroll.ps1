param([string]$Out, [double]$Fraction, [string]$Name, [int]$WaitMs = 4000, [int]$Wheel = 0)
$ErrorActionPreference = "Continue"
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
Add-Type @"
using System; using System.Runtime.InteropServices;
public class ES2 { [DllImport("user32.dll")] public static extern bool SetProcessDPIAware(); [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y); [DllImport("user32.dll")] public static extern void mouse_event(int f, int dx, int dy, int d, IntPtr e); [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowW(string c, string n); [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r); [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h); [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; } }
"@
[ES2]::SetProcessDPIAware() | Out-Null
$dir = Split-Path $Out
$lines = @()
function Snap($name, $x, $y, $w, $h) { $b = New-Object System.Drawing.Bitmap($w, $h); $g = [System.Drawing.Graphics]::FromImage($b); $g.CopyFromScreen($x, $y, 0, 0, (New-Object System.Drawing.Size($w, $h))); $b.Save("$dir\$name.png", [System.Drawing.Imaging.ImageFormat]::Png); $g.Dispose(); $b.Dispose() }
function ClickAt($x, $y) { [ES2]::SetCursorPos($x, $y) | Out-Null; Start-Sleep -Milliseconds 150; [ES2]::mouse_event(2,0,0,0,[IntPtr]::Zero); Start-Sleep -Milliseconds 50; [ES2]::mouse_event(4,0,0,0,[IntPtr]::Zero) }
$s = [ES2]::FindWindowW("BarometerStrip", $null); $sr = New-Object ES2+RECT; [ES2]::GetWindowRect($s, [ref]$sr) | Out-Null
$lines += "strip: $($sr.L),$($sr.T)-$($sr.R),$($sr.B)"
ClickAt ($sr.L + [int](($sr.R - $sr.L) * $Fraction)) ($sr.T + 24); Start-Sleep -Milliseconds $WaitMs
$f = [ES2]::FindWindowW("BarometerFlyout", $null); $r = New-Object ES2+RECT; [ES2]::GetWindowRect($f, [ref]$r) | Out-Null
$lines += "$Name panel: $($r.L),$($r.T)-$($r.R),$($r.B) visible=$([ES2]::IsWindowVisible($f))"
if ($Wheel -gt 0 -and $r.R -gt $r.L + 20) {
  [ES2]::SetCursorPos([int](($r.L + $r.R) / 2), [int](($r.T + $r.B) / 2)) | Out-Null; Start-Sleep -Milliseconds 200
  for ($i = 0; $i -lt $Wheel; $i++) { [ES2]::mouse_event(0x0800, 0, 0, -120, [IntPtr]::Zero); Start-Sleep -Milliseconds 60 }
  Start-Sleep -Milliseconds 700
}
if ($r.R -gt $r.L + 20) { Snap $Name ($r.L - 10) ($r.T - 10) ($r.R - $r.L + 20) ($r.B - $r.T + 20) }
[System.Windows.Forms.SendKeys]::SendWait("{ESC}"); Start-Sleep -Milliseconds 300
[ES2]::SetCursorPos(1200, 800) | Out-Null
$lines | Out-File -FilePath $Out -Encoding utf8

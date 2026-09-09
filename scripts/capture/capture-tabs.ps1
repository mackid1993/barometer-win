param([string]$Out, [double]$Fraction, [int]$WaitMs = 6000, [int]$ChipX = 129, [int]$ChipY = 33)
$ErrorActionPreference = "Continue"
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
Add-Type @"
using System; using System.Runtime.InteropServices;
public class ET { [DllImport("user32.dll")] public static extern bool SetProcessDPIAware(); [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y); [DllImport("user32.dll")] public static extern void mouse_event(int f, int dx, int dy, int d, IntPtr e); [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowW(string c, string n); [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r); [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; } }
"@
[ET]::SetProcessDPIAware() | Out-Null
$dir = Split-Path $Out
$lines = @()
function Snap($name, $x, $y, $w, $h) { $b = New-Object System.Drawing.Bitmap($w, $h); $g = [System.Drawing.Graphics]::FromImage($b); $g.CopyFromScreen($x, $y, 0, 0, (New-Object System.Drawing.Size($w, $h))); $b.Save("$dir\$name.png", [System.Drawing.Imaging.ImageFormat]::Png); $g.Dispose(); $b.Dispose() }
function ClickAt($x, $y) { [ET]::SetCursorPos($x, $y) | Out-Null; Start-Sleep -Milliseconds 200; [ET]::mouse_event(2,0,0,0,[IntPtr]::Zero); Start-Sleep -Milliseconds 60; [ET]::mouse_event(4,0,0,0,[IntPtr]::Zero) }
function Panel { $f = [ET]::FindWindowW("BarometerFlyout", $null); $r = New-Object ET+RECT; [ET]::GetWindowRect($f, [ref]$r) | Out-Null; return $r }
$s = [ET]::FindWindowW("BarometerStrip", $null); $sr = New-Object ET+RECT; [ET]::GetWindowRect($s, [ref]$sr) | Out-Null
$lines += "strip: $($sr.L),$($sr.T)-$($sr.R),$($sr.B)"
ClickAt ($sr.L + [int](($sr.R - $sr.L) * $Fraction)) ($sr.T + 24)
Start-Sleep -Milliseconds $WaitMs
$r = Panel
$lines += "tab1 panel: $($r.L),$($r.T)-$($r.R),$($r.B)"
if ($r.R -gt $r.L + 20) {
  Snap "tab1" ($r.L - 8) ($r.T - 8) ($r.R - $r.L + 16) ($r.B - $r.T + 16)
  # The switcher chips sit on the first line of the panel's content.
  ClickAt ($r.L + $ChipX) ($r.T + $ChipY)
  Start-Sleep -Milliseconds 2500
  $r2 = Panel
  $lines += "tab2 panel: $($r2.L),$($r2.T)-$($r2.R),$($r2.B)"
  Snap "tab2" ($r2.L - 8) ($r2.T - 8) ($r2.R - $r2.L + 16) ($r2.B - $r2.T + 16)
}
[System.Windows.Forms.SendKeys]::SendWait("{ESC}"); Start-Sleep -Milliseconds 300
[ET]::SetCursorPos(1200, 800) | Out-Null
$log = "$env:LOCALAPPDATA\Barometer\trace.log"
if (Test-Path $log) { $lines += (Select-String -Path $log -Pattern "click column" | Select-Object -Last 1).Line }
$lines | Out-File -FilePath $Out -Encoding utf8

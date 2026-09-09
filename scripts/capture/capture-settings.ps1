param([string]$Out, [string]$Name = "settings", [int]$ClickX = 0, [int]$ClickY = 0, [int]$Click2X = 0, [int]$Click2Y = 0, [int]$Click3X = 0, [int]$Click3Y = 0)
$ErrorActionPreference = "Continue"
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
Add-Type @"
using System; using System.Text; using System.Runtime.InteropServices;
public class EW {
  public delegate bool Proc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(int f, int dx, int dy, int d, IntPtr e);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowW(string c, string n);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool EnumWindows(Proc f, IntPtr l);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  // FindWindowW does not match this class across processes; enumeration does.
  public static IntPtr ByClass(string wanted) {
    IntPtr found = IntPtr.Zero;
    EnumWindows((h, l) => { var sb = new StringBuilder(256); GetClassNameW(h, sb, 256); if (sb.ToString() == wanted && IsWindowVisible(h)) { found = h; return false; } return true; }, IntPtr.Zero);
    return found;
  }
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@
[EW]::SetProcessDPIAware() | Out-Null
$dir = Split-Path $Out
$lines = @()
function Snap($name, $x, $y, $w, $h) { $b = New-Object System.Drawing.Bitmap($w, $h); $g = [System.Drawing.Graphics]::FromImage($b); $g.CopyFromScreen($x, $y, 0, 0, (New-Object System.Drawing.Size($w, $h))); $b.Save("$dir\$name.png", [System.Drawing.Imaging.ImageFormat]::Png); $g.Dispose(); $b.Dispose() }
function ClickAt($x, $y) { [EW]::SetCursorPos($x, $y) | Out-Null; Start-Sleep -Milliseconds 220; [EW]::mouse_event(2,0,0,0,[IntPtr]::Zero); Start-Sleep -Milliseconds 60; [EW]::mouse_event(4,0,0,0,[IntPtr]::Zero) }
$w = [EW]::ByClass("BarometerSettings")
if ($w -eq [IntPtr]::Zero) {
  # Not up yet: open it from the strip's right-click menu, first row, driven
  # from the keyboard because a menu ignores an injected click on an item the
  # real pointer never entered.
  $s = [EW]::FindWindowW("BarometerStrip", $null); $sr = New-Object EW+RECT; [EW]::GetWindowRect($s, [ref]$sr) | Out-Null
  [EW]::SetCursorPos(($sr.L + [int](($sr.R - $sr.L) * 0.5)), ($sr.T + 24)) | Out-Null
  Start-Sleep -Milliseconds 200
  [EW]::mouse_event(8,0,0,0,[IntPtr]::Zero); Start-Sleep -Milliseconds 60; [EW]::mouse_event(16,0,0,0,[IntPtr]::Zero)
  Start-Sleep -Milliseconds 900
  [System.Windows.Forms.SendKeys]::SendWait("{DOWN}")
  Start-Sleep -Milliseconds 300
  [System.Windows.Forms.SendKeys]::SendWait("{ENTER}")
  for ($i = 0; $i -lt 40; $i++) { Start-Sleep -Milliseconds 300; $w = [EW]::ByClass("BarometerSettings"); if ($w -ne [IntPtr]::Zero) { break } }
}
if ($w -eq [IntPtr]::Zero) { $lines += "no settings window"; $lines | Out-File -FilePath $Out -Encoding utf8; exit }
[EW]::SetForegroundWindow($w) | Out-Null
Start-Sleep -Milliseconds 500
$r = New-Object EW+RECT; [EW]::GetWindowRect($w, [ref]$r) | Out-Null
$lines += "settings: $($r.L),$($r.T)-$($r.R),$($r.B)"
foreach ($click in @(@($ClickX, $ClickY), @($Click2X, $Click2Y), @($Click3X, $Click3Y))) {
  if ($click[0] -gt 0) { ClickAt ($r.L + $click[0]) ($r.T + $click[1]); Start-Sleep -Milliseconds 800 }
}
[EW]::GetWindowRect($w, [ref]$r) | Out-Null
Snap $Name $r.L $r.T ($r.R - $r.L) ($r.B - $r.T)
$lines | Out-File -FilePath $Out -Encoding utf8

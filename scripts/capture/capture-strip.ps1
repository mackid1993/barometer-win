param([string]$Out, [int]$WaitMs = 9000)
$ErrorActionPreference = "Continue"
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System; using System.Runtime.InteropServices;
public class ES { [DllImport("user32.dll")] public static extern bool SetProcessDPIAware(); [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowW(string c, string n); [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r); [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; } }
"@
[ES]::SetProcessDPIAware() | Out-Null
Start-Sleep -Milliseconds $WaitMs
$dir = Split-Path $Out
$s = [ES]::FindWindowW("BarometerStrip", $null); $sr = New-Object ES+RECT; [ES]::GetWindowRect($s, [ref]$sr) | Out-Null
$w = $sr.R - $sr.L + 20; $h = $sr.B - $sr.T
$b = New-Object System.Drawing.Bitmap($w, $h); $g = [System.Drawing.Graphics]::FromImage($b)
$g.CopyFromScreen($sr.L - 10, $sr.T, 0, 0, (New-Object System.Drawing.Size($w, $h)))
$b.Save("$dir\strip.png", [System.Drawing.Imaging.ImageFormat]::Png); $g.Dispose(); $b.Dispose()
"strip: $($sr.L),$($sr.T)-$($sr.R),$($sr.B)" | Out-File -FilePath $Out -Encoding utf8

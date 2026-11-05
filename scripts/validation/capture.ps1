param([int]$AppProcessId, [string]$OutputPath, [string]$ExpectedTitle)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public class ViewerWindow {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out RECT rect);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
}
'@
$app = Get-Process -Id $AppProcessId
if ($app.MainWindowHandle -eq 0) { throw 'Native renderer did not create a desktop window' }
if ($app.MainWindowTitle -notlike "*$ExpectedTitle*") { throw "Unexpected native window title: $($app.MainWindowTitle)" }
[ViewerWindow]::SetForegroundWindow($app.MainWindowHandle) | Out-Null
Start-Sleep -Milliseconds 300
$rect = New-Object ViewerWindow+RECT
if (![ViewerWindow]::GetWindowRect($app.MainWindowHandle, [ref]$rect)) { throw 'Cannot inspect native window bounds' }
$bitmap = New-Object System.Drawing.Bitmap (($rect.Right-$rect.Left), ($rect.Bottom-$rect.Top))
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
try {
    $graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size)
    $bitmap.Save($OutputPath, [System.Drawing.Imaging.ImageFormat]::Png)
} finally {
    $graphics.Dispose()
    $bitmap.Dispose()
}

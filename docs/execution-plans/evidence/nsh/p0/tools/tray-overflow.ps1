param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [string]$Tag = "s4-tray-overflow"
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class O {
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint d, int e);
}
"@
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$shots = Join-Path $here "..\shots"

$p = Start-Process -FilePath $Exe -PassThru
Start-Sleep -Milliseconds 3000
$b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
[O]::SetCursorPos($b.Right - 187, $b.Bottom - 33) | Out-Null
Start-Sleep -Milliseconds 300
[O]::mouse_event(0x0002, 0, 0, 0, 0)
[O]::mouse_event(0x0004, 0, 0, 0, 0)
Start-Sleep -Milliseconds 1200

$w = 340; $h = 180
$bmp = New-Object System.Drawing.Bitmap $w, $h
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($b.Right - $w, $b.Bottom - 60 - $h, 0, 0, (New-Object System.Drawing.Size $w, $h))
$bmp.Save((Join-Path $shots "$Tag.png"), [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
Write-Host ("{0}: overflow captured; process alive={1}" -f $Tag, (-not $p.HasExited))
Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
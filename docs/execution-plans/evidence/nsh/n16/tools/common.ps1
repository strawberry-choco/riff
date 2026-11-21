# Shared window-drive helpers for the N7 evidence runs (the N6 drive's
# machinery, factored so the walk and the shots stay in one place).
param()

$ErrorActionPreference = 'Stop'
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class N7 {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr a, int x, int y, int w, int ht, uint f);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint d, int e);
    [DllImport("user32.dll")] public static extern uint SendInput(uint n, INPUT[] inputs, int size);
    [StructLayout(LayoutKind.Sequential)] public struct INPUT { public uint type; public InputUnion U; }
    [StructLayout(LayoutKind.Explicit)] public struct InputUnion { [FieldOffset(0)] public MOUSEINPUT mi; [FieldOffset(0)] public KEYBDINPUT ki; }
    [StructLayout(LayoutKind.Sequential)] public struct MOUSEINPUT { public int dx, dy; public uint mouseData, dwFlags, time; public IntPtr dwExtraInfo; }
    [StructLayout(LayoutKind.Sequential)] public struct KEYBDINPUT { public ushort wVk, wScan; public uint dwFlags, time; public IntPtr dwExtraInfo; public ushort unused1, unused2; }
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms

function Shot($h, $name) {
    $r = New-Object N7+RECT
    [N7]::GetWindowRect($h, [ref]$r) | Out-Null
    $w = $r.R - $r.L; $ht = $r.B - $r.T
    $bmp = New-Object System.Drawing.Bitmap $w, $ht
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $hdc = $g.GetHdc()
    $ok = [N7]::PrintWindow($h, $hdc, 2)
    $g.ReleaseHdc($hdc); $g.Dispose()
    if (-not $ok) { throw "PrintWindow failed" }
    $bmp.Save((Join-Path $script:ShotsDir "$name.png"), [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    Write-Host "shot saved: $name.png (${w}x${ht})"
}

function Click($h, $x, $y) {
    $r = New-Object N7+RECT
    [N7]::GetWindowRect($h, [ref]$r) | Out-Null
    [N7]::SetCursorPos($r.L + $x, $r.T + $y) | Out-Null
    Start-Sleep -Milliseconds 200
    [N7]::mouse_event(0x0002, 0, 0, 0, 0)
    Start-Sleep -Milliseconds 60
    [N7]::mouse_event(0x0004, 0, 0, 0, 0)
}

function Wheel($h, $notches) {
    $r = New-Object N7+RECT
    [N7]::GetWindowRect($h, [ref]$r) | Out-Null
    [N7]::SetCursorPos($r.L + 500, $r.T + 400) | Out-Null
    Start-Sleep -Milliseconds 120
    $delta = if ($notches -ge 0) { 120 } else { -120 }
    $n = [Math]::Abs($notches)
    for ($i = 0; $i -lt $n; $i++) {
        $inp = New-Object N7+INPUT
        $inp.type = 0
        $inp.U.mi.mouseData = $delta
        $inp.U.mi.dwFlags = 0x0800
        $arr = New-Object 'N7+INPUT[]' 1
        $arr[0] = $inp
        $sent = [N7]::SendInput(1, $arr, [System.Runtime.InteropServices.Marshal]::SizeOf([type][N7+INPUT]))
        if ($sent -ne 1) { Write-Host "SendInput sent $sent" }
        Start-Sleep -Milliseconds 30
    }
}

function Keys([string]$keys, [double]$settle) {
    [System.Windows.Forms.SendKeys]::SendWait($keys)
    Start-Sleep -Seconds $settle
}

function Start-Riff($Exe, [string]$Store) {
    $psi = New-Object Diagnostics.ProcessStartInfo
    $psi.FileName = $Exe
    $psi.UseShellExecute = $false
    $psi.EnvironmentVariables["RUST_LOG"] = "info"
    $psi.EnvironmentVariables["RIFF_N3_STORE"] = $Store
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $p = [Diagnostics.Process]::Start($psi)
    $h = [IntPtr]::Zero
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.Elapsed.TotalSeconds -lt 30) {
        $p.Refresh()
        if ($p.HasExited) { break }
        if ($p.MainWindowHandle -ne [IntPtr]::Zero) { $h = $p.MainWindowHandle; break }
        Start-Sleep -Milliseconds 200
    }
    if ($h -eq [IntPtr]::Zero) { throw "no window after 30 s" }
    Write-Host ("window after {0} ms pid {1}" -f $sw.ElapsedMilliseconds, $p.Id)
    return @{ Process = $p; Handle = $h }
}

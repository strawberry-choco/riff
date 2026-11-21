# N4 keyboard walkthrough, take two: the shell's map is Space = pause/resume
# (a stopped session stays stopped), Ctrl+K = focus search, Escape = blur,
# arrows = the focus ring. So the sequence plays by click first, then proves
# each key with the state it can actually move; the Ctrl+K proof is typing.
$ErrorActionPreference = "Stop"
$repo = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..\..\..\..")).Path
$exe  = Join-Path $repo "target/debug/riff-nsh.exe"
$env:RIFF_N3_STORE = Join-Path $repo ".scratch/nsh-n3/store/riff.sqlite3"
$env:RUST_LOG = "info"

Add-Type -AssemblyName System.Windows.Forms
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class WinWait {
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc cb, IntPtr lp);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
    public delegate bool EnumWindowsProc(IntPtr h, IntPtr lp);
    public struct RECT { public int Left, Top, Right, Bottom; }
    public static IntPtr MainOf(uint target) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, lp) => {
            uint pid; GetWindowThreadProcessId(h, out pid);
            if (pid == target && IsWindowVisible(h)) { found = h; return false; }
            return true;
        }, IntPtr.Zero);
        return found;
    }
    public static void Click(IntPtr hwnd, int x, int y) {
        RECT r; GetWindowRect(hwnd, out r);
        SetCursorPos(r.Left + x, r.Top + y);
        System.Threading.Thread.Sleep(120);
        mouse_event(2, 0, 0, 0, UIntPtr.Zero);
        mouse_event(4, 0, 0, 0, UIntPtr.Zero);
    }
}
"@

$p = Start-Process -FilePath $exe -WorkingDirectory $repo -PassThru -RedirectStandardOutput "../logs/kb-walk.stdout.log" -RedirectStandardError "../logs/kb-walk.stderr.log"
$hwnd = [IntPtr]::Zero
for ($i = 0; $i -lt 60; $i++) {
    Start-Sleep -Milliseconds 500
    $hwnd = [WinWait]::MainOf($p.Id)
    if ($hwnd -ne [IntPtr]::Zero) { break }
}
if ($hwnd -eq [IntPtr]::Zero) { Write-Error "no window after 30 s"; exit 1 }

$sh = New-Object -ComObject WScript.Shell
function Shot([string]$name) {
    & (Join-Path $PSScriptRoot "shot.ps1") -ProcId $p.Id -Out "../shots/$name.png" | Out-Null
    Write-Output "shot $name"
}
function Keys([string]$keys, [double]$settle) {
    [void]$sh.AppActivate($p.Id)
    Start-Sleep -Milliseconds 300
    $sh.SendKeys($keys)
    Start-Sleep -Seconds $settle
}

Start-Sleep -Seconds 2
[void]$sh.AppActivate($p.Id)
Start-Sleep -Milliseconds 500
# Debug builds open on the bridge readout; mod+alt+r mounts the shell.
$sh.SendKeys("^%r")
Start-Sleep -Seconds 2

[WinWait]::Click($hwnd, 497, 667)
Start-Sleep -Seconds 2
Shot "n4-kb-1-click-playing"
Keys -keys " " -settle 1
Shot "n4-kb-2-space-paused"
Keys -keys " " -settle 1
Shot "n4-kb-3-space-resumed"
Keys -keys "^k" -settle 0.5
Keys -keys "echo" -settle 1
Shot "n4-kb-4-ctrlk-typed"
Keys -keys "{ESC}" -settle 0.5
Keys -keys "zzz" -settle 1
Shot "n4-kb-5-escape-typed"
Keys -keys "{DOWN}" -settle 0.7
Shot "n4-kb-6-arrow-ring"

Stop-Process -Id $p.Id -Force
Write-Output "done"

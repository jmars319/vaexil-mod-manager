# Visual smoke test for the window repainting after it is minimized and
# restored, and after maximize/restore. Runs the app against a throwaway fake
# HITMAN folder (VAEXIL_MOD_MANAGER_GAME), so no real game or SMF is touched.
# Exits 1 if a restored window differs from the first screenshot.
#
#   ./scripts/smoke-minimize-restore.ps1 [-Exe target\release\vaexil-mod-manager.exe]
param(
    [string] $Exe = (Join-Path $PSScriptRoot '..\target\release\vaexil-mod-manager.exe'),
    [int] $Cycles = 3,
    [int] $MinimizedMs = 3000,
    [double] $MaxDifferentPercent = 1.0
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing, UIAutomationClient, UIAutomationTypes
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class SmokeWin {
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr h, int x, int y, int w, int hh, bool r);
  [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr v);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int w, int hh, uint flags);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@
[SmokeWin]::SetProcessDpiAwarenessContext([IntPtr]-4) | Out-Null

# A fake game folder: Retail\HITMAN3.exe, and an SMF folder with an empty
# Deploy.exe that is never run, a load order and a few mods, so the window
# has rows to repaint.
$work = Join-Path ([IO.Path]::GetTempPath()) "vaexil-mm-smoke-$PID"
$smf = Join-Path $work 'Simple Mod Framework'
New-Item -ItemType Directory -Force (Join-Path $work 'Retail') | Out-Null
Set-Content -Path (Join-Path $work 'Retail\HITMAN3.exe') -Value '' -NoNewline
New-Item -ItemType Directory -Force $smf | Out-Null
Set-Content -Path (Join-Path $smf 'Deploy.exe') -Value '' -NoNewline
Set-Content -Path (Join-Path $smf 'config.json') -Value '{ "loadOrder": ["Smoke.ModTwo", "Smoke RPKG"] }'
foreach ($n in 'One', 'Two', 'Three', 'Four') {
    $dir = Join-Path $smf "Mods\Mod $n"
    New-Item -ItemType Directory -Force $dir | Out-Null
    Set-Content -Path (Join-Path $dir 'manifest.json') -Value "{ `"id`": `"Smoke.Mod$n`", `"name`": `"Smoke Mod $n`", `"version`": `"1.0.0`", `"authors`": [`"Smoke`"] }"
}
New-Item -ItemType Directory -Force (Join-Path $smf 'Mods\Smoke RPKG') | Out-Null

$psi = [System.Diagnostics.ProcessStartInfo]::new((Resolve-Path $Exe))
$psi.Environment['VAEXIL_MOD_MANAGER_GAME'] = $work
$p = [System.Diagnostics.Process]::Start($psi)

# The process also owns a small helper window, which .NET's MainWindowHandle
# can pick, so find the real window by its title through UI Automation.
function Find-AppWindow {
    $A = [System.Windows.Automation.AutomationElement]
    $byPid = New-Object System.Windows.Automation.PropertyCondition ($A::ProcessIdProperty), $p.Id
    foreach ($t in $A::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children, $byPid)) {
        if ($t.Current.Name -eq 'Vaexil Mod Manager') { return [IntPtr]$t.Current.NativeWindowHandle }
    }
    [IntPtr]::Zero
}

# Background scripts cannot take the foreground, so keep the window on top
# while capturing; otherwise other windows end up in the screenshots.
function KeepOnTop($h) {
    [SmokeWin]::SetWindowPos($h, [IntPtr]-1, 0, 0, 0, 0, 0x0001 -bor 0x0002 -bor 0x0010) | Out-Null  # TOPMOST; NOSIZE|NOMOVE|NOACTIVATE
}

# Captures only the client area: the window rectangle also includes the
# invisible resize border, which shows whatever is behind the window.
function Capture($h) {
    $r = New-Object SmokeWin+RECT
    [SmokeWin]::GetClientRect($h, [ref]$r) | Out-Null
    $origin = New-Object SmokeWin+POINT
    [SmokeWin]::ClientToScreen($h, [ref]$origin) | Out-Null
    $bmp = New-Object System.Drawing.Bitmap ($r.R - $r.L), ($r.B - $r.T)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($origin.X, $origin.Y, 0, 0, $bmp.Size)
    $g.Dispose()
    $bmp
}

function DifferentPercent($a, $b) {
    $w = [Math]::Min($a.Width, $b.Width); $h = [Math]::Min($a.Height, $b.Height)
    $diff = 0; $n = 0
    for ($y = 0; $y -lt $h; $y += 6) {
        for ($x = 0; $x -lt $w; $x += 6) {
            $c1 = $a.GetPixel($x, $y); $c2 = $b.GetPixel($x, $y); $n++
            if ([Math]::Abs($c1.R - $c2.R) + [Math]::Abs($c1.G - $c2.G) + [Math]::Abs($c1.B - $c2.B) -gt 24) { $diff++ }
        }
    }
    100.0 * $diff / [Math]::Max(1, $n)
}

$failed = $false
try {
    $h = [IntPtr]::Zero
    for ($i = 0; $i -lt 50 -and $h -eq [IntPtr]::Zero; $i++) { Start-Sleep -Milliseconds 200; $h = Find-AppWindow }
    if ($h -eq [IntPtr]::Zero) { throw 'the Vaexil Mod Manager window did not appear' }
    [SmokeWin]::MoveWindow($h, 80, 80, 1000, 700, $true) | Out-Null
    [SmokeWin]::SetForegroundWindow($h) | Out-Null
    KeepOnTop $h
    Start-Sleep -Milliseconds 1500
    $baseline = Capture $h
    $baseline.Save((Join-Path $work 'baseline.png'))

    $steps = @()
    for ($c = 1; $c -le $Cycles; $c++) { $steps += @{ Name = "minimize-restore-$c"; Show = 6; Wait = $MinimizedMs } }
    $steps += @{ Name = 'maximize-restore'; Show = 3; Wait = 1500 }
    foreach ($step in $steps) {
        [SmokeWin]::ShowWindow($h, $step.Show) | Out-Null
        Start-Sleep -Milliseconds $step.Wait
        [SmokeWin]::ShowWindow($h, 9) | Out-Null   # SW_RESTORE
        [SmokeWin]::SetForegroundWindow($h) | Out-Null
        KeepOnTop $h
        Start-Sleep -Milliseconds 1200
        $shot = Capture $h
        $shot.Save((Join-Path $work "$($step.Name).png"))
        $pct = DifferentPercent $baseline $shot
        $shot.Dispose()
        $ok = $pct -le $MaxDifferentPercent
        if (-not $ok) { $failed = $true }
        '{0,-20} {1,6:N2}% different  {2}' -f $step.Name, $pct, $(if ($ok) { 'ok' } else { 'FAIL' })
    }
} finally {
    if (-not $p.HasExited) { $p.Kill(); $p.WaitForExit() }
}
"Screenshots: $work"
if ($failed) { exit 1 }
exit 0

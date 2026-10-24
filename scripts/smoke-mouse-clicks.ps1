# Smoke test for real mouse clicks on the themed controls. UI Automation's
# Invoke skips the mouse path, so a control can pass every other test while
# ignoring clicks (found in Modlist Launcher, fixed there in #20). Posts mouse
# messages straight to the window (the real cursor never moves) and exits 1
# if a click does not act.
#
# The only themed control so far is the OK button on the message window that
# `--version` opens. That button already has keyboard focus when the window
# opens, and the Modlist Launcher bug only swallowed the first click on a
# control without focus, so this check passed even before the fix. It becomes
# a real guard once the main window has themed controls: add them here as
# they arrive, including a check that a click moves keyboard focus.
#
#   ./scripts/smoke-mouse-clicks.ps1 [-Exe target\release\vaexil-mod-manager.exe]
param([string] $Exe = (Join-Path $PSScriptRoot '..\target\release\vaexil-mod-manager.exe'))
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class SmokeClicks {
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool ScreenToClient(IntPtr h, ref POINT p);
  [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr v);
}
"@
[SmokeClicks]::SetProcessDpiAwarenessContext([IntPtr]-4) | Out-Null
$A = [System.Windows.Automation.AutomationElement]

# The process also owns a small helper window, which .NET's MainWindowHandle
# can pick, so find the real window by its title through UI Automation.
function Find-Window($proc, $title) {
    $byPid = New-Object System.Windows.Automation.PropertyCondition ($A::ProcessIdProperty), $proc.Id
    foreach ($t in $A::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children, $byPid)) {
        if ($t.Current.Name -eq $title) { return $t }
    }
    $null
}

# The automation tree can come back empty on the first query.
function Find-Control($window, $name) {
    for ($k = 0; $k -lt 10; $k++) {
        $byName = New-Object System.Windows.Automation.PropertyCondition ($A::NameProperty), $name
        $e = $window.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $byName)
        if ($e) { return $e }
        Start-Sleep -Milliseconds 500
    }
    $null
}

function Click($window, $control) {
    $h = [IntPtr]$window.Current.NativeWindowHandle
    $b = $control.Current.BoundingRectangle
    $pt = New-Object SmokeClicks+POINT
    $pt.X = [int]($b.X + $b.Width / 2); $pt.Y = [int]($b.Y + $b.Height / 2)
    [SmokeClicks]::ScreenToClient($h, [ref]$pt) | Out-Null
    $l = [IntPtr](($pt.Y -shl 16) -bor ($pt.X -band 0xffff))
    [SmokeClicks]::PostMessage($h, 0x0200, [IntPtr]0, $l) | Out-Null   # WM_MOUSEMOVE
    Start-Sleep -Milliseconds 120
    [SmokeClicks]::PostMessage($h, 0x0201, [IntPtr]1, $l) | Out-Null   # WM_LBUTTONDOWN
    Start-Sleep -Milliseconds 60
    [SmokeClicks]::PostMessage($h, 0x0202, [IntPtr]0, $l) | Out-Null   # WM_LBUTTONUP
}

$failed = $false
function Check($label, [bool] $ok) {
    if (-not $ok) { $script:failed = $true }
    '{0,-46} {1}' -f $label, $(if ($ok) { 'ok' } else { 'FAIL' })
}

$psi = [System.Diagnostics.ProcessStartInfo]::new((Resolve-Path $Exe), '--version')
$p = [System.Diagnostics.Process]::Start($psi)
try {
    $window = $null
    for ($i = 0; $i -lt 50 -and -not $window; $i++) { Start-Sleep -Milliseconds 200; $window = Find-Window $p 'Vaexil Mod Manager' }
    if (-not $window) { throw 'the message window did not appear' }
    Start-Sleep -Milliseconds 800
    $ok = Find-Control $window 'OK'
    if (-not $ok) { throw "no control named 'OK'" }
    Click $window $ok
    Check 'button: OK closes the message window' ($p.WaitForExit(3000))
} finally {
    if (-not $p.HasExited) { $p.Kill(); $p.WaitForExit() }
}
if ($failed) { exit 1 }
exit 0

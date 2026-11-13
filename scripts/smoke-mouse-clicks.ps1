# Smoke test for real mouse clicks on the themed controls. UI Automation's
# Invoke skips the mouse path, so a control can pass every other test while
# ignoring clicks (found in Modlist Launcher, fixed there in #20). Posts mouse
# messages straight to the window (the real cursor never moves) and exits 1
# if a click does not act.
#
# Checks the OK button on the message window that `--version` opens, then
# the main window against a throwaway fake game folder: a click on a mod's
# switch turns it on in config.json, keeps a copy of the old file, and moves
# keyboard focus to the switch; a second click turns it off again. Focusing
# the last mod's switch in a long list must scroll it into view.
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
  [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr h, int x, int y, int w, int hh, bool r);
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

# The main window, against a fake game folder with enough mods to scroll.
$work = Join-Path ([IO.Path]::GetTempPath()) "vaexil-mm-clicks-$PID"
$smf = Join-Path $work 'Simple Mod Framework'
$data = Join-Path $work 'app-data'
New-Item -ItemType Directory -Force (Join-Path $work 'Retail') | Out-Null
Set-Content -Path (Join-Path $work 'Retail\HITMAN3.exe') -Value '' -NoNewline
New-Item -ItemType Directory -Force $smf | Out-Null
Set-Content -Path (Join-Path $smf 'Deploy.exe') -Value '' -NoNewline
$config = Join-Path $smf 'config.json'
Set-Content -Path $config -Value "{runtimePath:'..\\Runtime',loadOrder:['Smoke.Mod01'],skipIntro:false}" -NoNewline
foreach ($n in 1..24) {
    $id = 'Smoke.Mod{0:D2}' -f $n
    $dir = Join-Path $smf ('Mods\Mod {0:D2}' -f $n)
    New-Item -ItemType Directory -Force $dir | Out-Null
    Set-Content -Path (Join-Path $dir 'manifest.json') -Value "{ `"id`": `"$id`", `"name`": `"Smoke Mod $('{0:D2}' -f $n)`", `"authors`": [`"Smoke`"] }"
}

function LoadOrder { (Get-Content $config -Raw) -match "Smoke.Mod02" }

$psi = [System.Diagnostics.ProcessStartInfo]::new((Resolve-Path $Exe))
$psi.Environment['VAEXIL_MOD_MANAGER_GAME'] = $work
$psi.Environment['VAEXIL_MOD_MANAGER_DATA'] = $data
$p = [System.Diagnostics.Process]::Start($psi)
try {
    $window = $null
    for ($i = 0; $i -lt 50 -and -not $window; $i++) { Start-Sleep -Milliseconds 200; $window = Find-Window $p 'Vaexil Mod Manager' }
    if (-not $window) { throw 'the main window did not appear' }
    $h = [IntPtr]$window.Current.NativeWindowHandle
    [SmokeClicks]::MoveWindow($h, 60, 60, 900, 640, $true) | Out-Null
    Start-Sleep -Milliseconds 1500

    $switch = Find-Control $window 'Smoke Mod 02'
    if (-not $switch) { throw "no switch named 'Smoke Mod 02'" }
    Click $window $switch
    Start-Sleep -Milliseconds 800
    Check 'switch: a click turns the mod on' (LoadOrder)
    Check 'switch: the old config.json was kept' ((Get-ChildItem (Join-Path $data 'config-backups') -Filter 'config-*.json' -ErrorAction SilentlyContinue).Count -eq 1)
    Check 'switch: a click moves keyboard focus to it' ($switch.Current.HasKeyboardFocus)
    Click $window $switch
    Start-Sleep -Milliseconds 800
    Check 'switch: a second click turns it off' (-not (LoadOrder))
    Check 'switch: SMF keys are kept' ((Get-Content $config -Raw) -match 'runtimePath')

    # After clicks, the switch must still follow a change made outside the app
    # (SMF's own Mod Manager writing config.json). The window rereads it every
    # five seconds.
    Set-Content -Path $config -Value "{loadOrder:['Smoke.Mod01','Smoke.Mod02']}" -NoNewline
    Start-Sleep -Seconds 6
    $toggle = $switch.GetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern)
    Check 'switch: follows config.json changed elsewhere' ($toggle.Current.ToggleState -eq 'On')

    # Tab from Smoke Mod 02 down to the last switch, which starts below the
    # visible part of the list. UI Automation only lists on-screen rows, so
    # finding it at all means the list scrolled.
    foreach ($k in 3..24) {
        [SmokeClicks]::PostMessage($h, 0x0100, [IntPtr]0x09, [IntPtr]0x000F0001) | Out-Null   # WM_KEYDOWN Tab
        [SmokeClicks]::PostMessage($h, 0x0101, [IntPtr]0x09, [IntPtr]0xC00F0001) | Out-Null   # WM_KEYUP Tab
        Start-Sleep -Milliseconds 120
    }
    Start-Sleep -Milliseconds 800
    $last = Find-Control $window 'Smoke Mod 24'
    $win = $window.Current.BoundingRectangle
    $inView = $last -and $last.Current.HasKeyboardFocus -and $last.Current.BoundingRectangle.Bottom -le $win.Bottom
    Check 'focus: Tab to the last row scrolls it into view' $inView
} finally {
    if (-not $p.HasExited) { $p.Kill(); $p.WaitForExit() }
}
Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
if ($failed) { exit 1 }
exit 0

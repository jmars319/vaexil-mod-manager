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
# the last mod's switch in a long list must scroll it into view. A mod's Edit
# button opens its sheet, where a clicked checkbox and Save store the option.
# The Load order view's moves and drag are drafts until Save. Deploy runs a
# stand-in Deploy.exe (the fake-deploy example, built first) and must end up
# to date with a log; a later change must show as changes since the deploy.
# A setup can be saved, and a compatibility pass can start with every mod
# off, try a batch and turn it off again. Settings' "Add the command line to
# my PATH" must first show what it would change; the test leaves without
# confirming and checks the user PATH did not change.
#
#   ./scripts/smoke-mouse-clicks.ps1 [-Exe target\release\vaexil-mod-manager.exe]
param([string] $Exe = (Join-Path $PSScriptRoot '..\target\release\vaexil-mod-manager.exe'))
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing, UIAutomationClient, UIAutomationTypes
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class SmokeClicks {
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool ScreenToClient(IntPtr h, ref POINT p);
  [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr v);
  [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr h, int x, int y, int w, int hh, bool r);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
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
    # A control that wasn't found must fail the test, not click nothing.
    if (-not $control) { throw 'Click: the control was not found (off screen, or renamed)' }
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
# A failed check saves a picture of the window (only this window, through
# PrintWindow, never the screen) for CI to upload.
$shots = if ($env:RUNNER_TEMP) { Join-Path $env:RUNNER_TEMP 'smoke' } else { Join-Path ([IO.Path]::GetTempPath()) "vaexil-mm-clicks-$PID" }
$script:shotWindow = $null
function Save-Shot($label) {
    if (-not $script:shotWindow) { return }
    try {
        $h = [IntPtr]$script:shotWindow.Current.NativeWindowHandle
        $r = New-Object SmokeClicks+RECT
        [SmokeClicks]::GetWindowRect($h, [ref]$r) | Out-Null
        $bmp = New-Object System.Drawing.Bitmap ($r.R - $r.L), ($r.B - $r.T)
        $g = [System.Drawing.Graphics]::FromImage($bmp); $hdc = $g.GetHdc()
        [SmokeClicks]::PrintWindow($h, $hdc, 2) | Out-Null
        $g.ReleaseHdc($hdc); $g.Dispose()
        New-Item -ItemType Directory -Force $shots | Out-Null
        $name = ($label -replace '[^A-Za-z0-9]+', '-').Trim('-')
        $bmp.Save((Join-Path $shots "fail-$name.png")); $bmp.Dispose()
    } catch { "  (no screenshot: $_)" }
}

# A control below the fold of a scrolling sheet isn't listed by UI
# Automation. This scrolls the window with the mouse wheel until the control
# shows, which works without keyboard focus.
function Scroll-To($window, $name) {
    $h = [IntPtr]$window.Current.NativeWindowHandle
    $b = $window.Current.BoundingRectangle
    $x = [int]($b.X + $b.Width / 2); $y = [int]($b.Y + $b.Height / 2)
    $l = [IntPtr](($y -shl 16) -bor ($x -band 0xffff))
    # Slint scrolls what is under the pointer it last saw, so move the
    # pointer to the middle of the window (client coordinates) first.
    $pt = New-Object SmokeClicks+POINT; $pt.X = $x; $pt.Y = $y
    [SmokeClicks]::ScreenToClient($h, [ref]$pt) | Out-Null
    [SmokeClicks]::PostMessage($h, 0x0200, [IntPtr]0, [IntPtr](($pt.Y -shl 16) -bor ($pt.X -band 0xffff))) | Out-Null   # WM_MOUSEMOVE
    Start-Sleep -Milliseconds 120
    $byName = New-Object System.Windows.Automation.PropertyCondition ($A::NameProperty), $name
    for ($k = 0; $k -lt 20; $k++) {
        $c = $window.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $byName)
        if ($c) { return $c }
        [SmokeClicks]::PostMessage($h, 0x020A, [IntPtr](-7864320), $l) | Out-Null   # WM_MOUSEWHEEL, one notch down
        Start-Sleep -Milliseconds 250
    }
    $null
}

# Scrolls on by $notches mouse-wheel notches, as Scroll-To does.
function Scroll-More($window, $notches) {
    $h = [IntPtr]$window.Current.NativeWindowHandle
    $b = $window.Current.BoundingRectangle
    $x = [int]($b.X + $b.Width / 2); $y = [int]($b.Y + $b.Height / 2)
    $l = [IntPtr](($y -shl 16) -bor ($x -band 0xffff))
    for ($k = 0; $k -lt $notches; $k++) {
        [SmokeClicks]::PostMessage($h, 0x020A, [IntPtr](-7864320), $l) | Out-Null   # WM_MOUSEWHEEL, one notch down
        Start-Sleep -Milliseconds 150
    }
    Start-Sleep -Milliseconds 300
}

function Check($label, [bool] $ok) {
    if (-not $ok) { $script:failed = $true; Save-Shot $label }
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
$fake = Join-Path $PSScriptRoot '..\target\release\examples\fake-deploy.exe'
if (-not (Test-Path $fake)) { throw "build the stand-in first: cargo build --release -p vaexil-mod-manager --example fake-deploy" }
Copy-Item $fake (Join-Path $smf 'Deploy.exe')
$config = Join-Path $smf 'config.json'
Set-Content -Path $config -Value "{runtimePath:'..\\Runtime',loadOrder:['Smoke.Mod01'],skipIntro:false}" -NoNewline
foreach ($n in 1..24) {
    $id = 'Smoke.Mod{0:D2}' -f $n
    $dir = Join-Path $smf ('Mods\Mod {0:D2}' -f $n)
    New-Item -ItemType Directory -Force $dir | Out-Null
    $options = if ($n -eq 3) { ", `"options`": [{ `"name`": `"Darker`", `"type`": `"checkbox`" }]" } else { '' }
    Set-Content -Path (Join-Path $dir 'manifest.json') -Value "{ `"id`": `"$id`", `"name`": `"Smoke Mod $('{0:D2}' -f $n)`", `"authors`": [`"Smoke`"]$options }"
}

function LoadOrder { (Get-Content $config -Raw) -match "Smoke.Mod02" }

$psi = [System.Diagnostics.ProcessStartInfo]::new((Resolve-Path $Exe))
$psi.Environment['VAEXIL_MOD_MANAGER_GAME'] = $work
$psi.Environment['VAEXIL_MOD_MANAGER_DATA'] = $data
# An empty stand-in for Steam, so Settings' Steam's Play button can never
# reach the real Steam's settings from a test.
$fakeSteam = Join-Path $work 'steam'
New-Item -ItemType Directory -Force $fakeSteam | Out-Null
$psi.Environment['VAEXIL_MOD_MANAGER_STEAM'] = $fakeSteam
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

    # The edit sheet: open it, tick the option, save.
    $edit = Find-Control $window 'Edit Smoke Mod 03'
    if (-not $edit) { throw "no button named 'Edit Smoke Mod 03'" }
    Click $window $edit
    Start-Sleep -Milliseconds 800
    $darker = Find-Control $window 'Darker'
    if (-not $darker) { throw "the edit sheet has no option named 'Darker'" }
    Click $window $darker
    Start-Sleep -Milliseconds 300
    $save = Find-Control $window 'Save'
    Click $window $save
    Start-Sleep -Milliseconds 800
    Check 'sheet: Save stores the clicked option' ((Get-Content $config -Raw) -match '"Smoke\.Mod03":\s*\[\s*"Darker"')
    $stillOpen = $window.FindFirst([System.Windows.Automation.TreeScope]::Descendants, (New-Object System.Windows.Automation.PropertyCondition ($A::NameProperty), 'Save'))
    Check 'sheet: Save closes the sheet' (-not $stillOpen)

    # Tab down to the last switch, which starts below the visible part of the
    # list. UI Automation only lists on-screen rows, so finding it at all
    # means the list scrolled. Each row has two stops: its switch and Edit.
    Click $window (Find-Control $window 'Edit Smoke Mod 01')
    Start-Sleep -Milliseconds 600
    [SmokeClicks]::PostMessage($h, 0x0100, [IntPtr]0x1B, [IntPtr]0x00010001) | Out-Null   # WM_KEYDOWN Escape
    [SmokeClicks]::PostMessage($h, 0x0101, [IntPtr]0x1B, [IntPtr]0xC0010001) | Out-Null
    Start-Sleep -Milliseconds 600
    for ($k = 0; $k -lt 80; $k++) {
        $focused = [System.Windows.Automation.AutomationElement]::FocusedElement
        if ($focused -and $focused.Current.Name -eq 'Smoke Mod 24') { break }
        [SmokeClicks]::PostMessage($h, 0x0100, [IntPtr]0x09, [IntPtr]0x000F0001) | Out-Null   # WM_KEYDOWN Tab
        [SmokeClicks]::PostMessage($h, 0x0101, [IntPtr]0x09, [IntPtr]0xC00F0001) | Out-Null   # WM_KEYUP Tab
        Start-Sleep -Milliseconds 150
    }
    Start-Sleep -Milliseconds 800
    $cancel = $window.FindFirst([System.Windows.Automation.TreeScope]::Descendants, (New-Object System.Windows.Automation.PropertyCondition ($A::NameProperty), 'Cancel'))
    Check 'sheet: Escape closes it' (-not $cancel)
    $last = Find-Control $window 'Smoke Mod 24'
    $win = $window.Current.BoundingRectangle
    $inView = $last -and $last.Current.HasKeyboardFocus -and $last.Current.BoundingRectangle.Bottom -le $win.Bottom
    Check 'focus: Tab to the last row scrolls it into view' $inView

    # The Load order view: a move is a draft until Save. The load order is
    # Mod01, Mod02 since the change made elsewhere above.
    Click $window (Find-Control $window 'Load order')
    Start-Sleep -Milliseconds 800
    $down = Find-Control $window 'Move Smoke Mod 01 down'
    if (-not $down) { throw "the Load order view has no 'Move Smoke Mod 01 down' button" }
    $before = Get-Content $config -Raw
    Click $window $down
    Start-Sleep -Milliseconds 800
    Check 'order: a move is not saved until Save' ((Get-Content $config -Raw) -eq $before)
    Click $window (Find-Control $window 'Save load order')
    Start-Sleep -Milliseconds 800
    Check 'order: Save writes the new order' ((Get-Content $config -Raw) -match "Smoke\.Mod02`"\s*,\s*`"Smoke\.Mod01")

    # Drag the first row down one place by its handle, then save.
    $row = Find-Control $window '1. Smoke Mod 02, Other'
    if (-not $row) { throw "no Load order row named '1. Smoke Mod 02, Other'" }
    $b = $row.Current.BoundingRectangle
    $pt = New-Object SmokeClicks+POINT
    $pt.X = [int]($b.X + 17); $pt.Y = [int]($b.Y + $b.Height / 2)
    [SmokeClicks]::ScreenToClient($h, [ref]$pt) | Out-Null
    $lp = { param($x, $y) [IntPtr](($y -shl 16) -bor ($x -band 0xffff)) }
    [SmokeClicks]::PostMessage($h, 0x0200, [IntPtr]0, (& $lp $pt.X $pt.Y)) | Out-Null
    Start-Sleep -Milliseconds 100
    [SmokeClicks]::PostMessage($h, 0x0201, [IntPtr]1, (& $lp $pt.X $pt.Y)) | Out-Null
    foreach ($dy in 20, 40, 60, 72) {
        Start-Sleep -Milliseconds 60
        [SmokeClicks]::PostMessage($h, 0x0200, [IntPtr]1, (& $lp $pt.X ($pt.Y + $dy))) | Out-Null
    }
    Start-Sleep -Milliseconds 60
    [SmokeClicks]::PostMessage($h, 0x0202, [IntPtr]0, (& $lp $pt.X ($pt.Y + 72))) | Out-Null
    Start-Sleep -Milliseconds 800
    Click $window (Find-Control $window 'Save load order')
    Start-Sleep -Milliseconds 800
    Check 'order: dragging a row by its handle moves it' ((Get-Content $config -Raw) -match "Smoke\.Mod01`"\s*,\s*`"Smoke\.Mod02")
    Click $window (Find-Control $window 'Back to mods')
    Start-Sleep -Milliseconds 600
    Check 'order: Back returns to the Mods view' ([bool](Find-Control $window 'Edit Smoke Mod 01'))

    # Deploy, with the stand-in Deploy.exe.
    function Wait-Name($name, $seconds) {
        for ($k = 0; $k -lt $seconds * 4; $k++) {
            $e = $window.FindFirst([System.Windows.Automation.TreeScope]::Descendants, (New-Object System.Windows.Automation.PropertyCondition ($A::NameProperty), $name))
            if ($e) { return $e }
            Start-Sleep -Milliseconds 250
        }
        $null
    }
    Check 'deploy: not deployed yet at first' ([bool](Wait-Name 'Deploy status: Not deployed from Vaexil Mod Manager yet.' 3))
    Click $window (Find-Control $window 'Deploy')
    Check 'deploy: ends up to date' ([bool](Wait-Name 'Deploy status: Deployed and up to date.' 10))
    $log = Get-ChildItem (Join-Path $data 'logs') -Filter 'deploy-*.log' -ErrorAction SilentlyContinue | Select-Object -First 1
    $logText = if ($log) { Get-Content $log.FullName -Raw } else { '' }
    Check 'deploy: the log has the output and SMF''s flags' ($logText -match 'Done in 1s' -and $logText -match '--doNotPause --colors' -and $logText -match 'A warning on stderr')
    # The report reads that log: one raw-entity warning counted once, and
    # the stderr line as a warning it doesn't recognise.
    Click $window (Find-Control $window 'Show the last deploy report')
    Start-Sleep -Milliseconds 600
    $outcome = $window.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition) |
        Where-Object { $_.Current.Name -like 'The last deploy succeeded*' } | Select-Object -First 1
    Check 'report: the deploy succeeded' ([bool]$outcome)
    Check 'report: a repeated warning counts once' ([bool](Find-Control $window 'Smoke Mod 01: once'))
    Click $window (Find-Control $window 'Close the deploy report')
    Start-Sleep -Milliseconds 400
    Click $window (Find-Control $window 'Smoke Mod 03')
    Check 'deploy: a change shows as changes since the deploy' ([bool](Wait-Name 'Deploy status: Changes since the last deploy. Deploy, or Play, to put them in the game.' 8))
    # Play with changes since the deploy asks first; Cancel starts nothing.
    Click $window (Find-Control $window 'Play HITMAN')
    Start-Sleep -Milliseconds 600
    Check 'play: changes since the deploy ask first' ([bool](Find-Control $window 'Play the last deployed build'))
    Click $window (Find-Control $window 'Cancel playing')
    Start-Sleep -Milliseconds 400
    Check 'play: Cancel closes the choice' (-not (Find-Control $window 'Play the last deployed build'))

    # Setups: save the mods that are on now.
    Click $window (Find-Control $window 'Setup: not saved. Open setups')
    Start-Sleep -Milliseconds 800
    $nameBox = Find-Control $window 'Setup name'
    $nameBox.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).SetValue('Smoke setup')
    Start-Sleep -Milliseconds 300
    Click $window (Find-Control $window 'Save as setup')
    Start-Sleep -Milliseconds 800
    $setupsFile = Join-Path $data 'setups.json'
    Check 'setups: Save keeps the mods that are on' ((Test-Path $setupsFile) -and (Get-Content $setupsFile -Raw) -match 'Smoke setup' -and (Get-Content $setupsFile -Raw) -match 'Smoke\.Mod03')
    Click $window (Find-Control $window 'Close')
    Start-Sleep -Milliseconds 600
    Check 'setups: the header names the setup in use' ([bool](Wait-Name 'Setup: Smoke setup. Open setups' 8))

    # A compatibility pass: every mod off, one batch, marked bad.
    Click $window (Find-Control $window 'Compatibility pass')
    Start-Sleep -Milliseconds 800
    Click $window (Find-Control $window 'Start with every mod off')
    Start-Sleep -Milliseconds 800
    $order = { ((Get-Content $config -Raw | ConvertFrom-Json).loadOrder) -join ',' }
    Check 'pass: starting turns every mod off' ((& $order) -eq '')
    Click $window (Find-Control $window 'Smoke Mod 05')
    Start-Sleep -Milliseconds 300
    Click $window (Find-Control $window 'Try this batch')
    Start-Sleep -Milliseconds 800
    Check 'pass: trying a batch turns on just its mods' ((& $order) -eq 'Smoke.Mod05')
    Click $window (Find-Control $window 'Mark batch bad')
    Start-Sleep -Milliseconds 800
    Check 'pass: a bad batch goes back to the baseline' ((& $order) -eq '')
    Click $window (Find-Control $window 'End compatibility pass')
    Start-Sleep -Milliseconds 600
    Check 'pass: ending keeps the mods as they are' ((& $order) -eq '' -and -not ((Get-Content $setupsFile -Raw) -match '"pass"'))
    Click $window (Find-Control $window 'Back to mods')
    Start-Sleep -Milliseconds 400

    # Settings: "Add the command line to my PATH" only shows what it would
    # change. This test never presses the confirming "Add to PATH", which
    # would change the real user PATH.
    # The test leaves the confirmation by Close, far from "Add to PATH", not
    # by the Cancel button beside it, and checks the user PATH is unchanged.
    $pathBefore = [Environment]::GetEnvironmentVariable('Path', 'User')
    $script:shotWindow = $window
    Click $window (Find-Control $window 'Settings')
    Start-Sleep -Milliseconds 800
    # The cards, top to bottom: Steam's Play button, SMF, AI tools, Command
    # line, Diagnostics. Scroll-To only scrolls down, so they are checked in
    # that order.
    # Steam's Play button reads the stand-in Steam, which has no account, so
    # it says so and offers no button.
    Check 'settings: Steam''s Play button finds no account in the stand-in' ([bool](Scroll-To $window 'No Steam account has signed in on this PC, so there are no settings to change.'))
    # The stand-in SMF isn't a release this app can name, so it offers no update.
    Check 'settings: SMF updates need a named release' ([bool](Scroll-To $window 'The installed SMF isn''t a release this app can name, so it can''t be updated from here.'))
    # "Let AI tools make changes" is off until switched on, and is kept in
    # the app's settings.json (here the test's throwaway data folder).
    $settingsFile = Join-Path $data 'settings.json'
    $aiOn = { (Test-Path $settingsFile) -and ((Get-Content $settingsFile -Raw) -match '"aiChanges":\s*true') }
    Check 'settings: AI changes are off at first' (-not (& $aiOn))
    Click $window (Scroll-To $window 'Let AI tools make changes')
    Start-Sleep -Milliseconds 500
    Check 'settings: the switch allows AI changes' (& $aiOn)
    Click $window (Scroll-To $window 'Let AI tools make changes')
    Start-Sleep -Milliseconds 500
    Check 'settings: a second click turns them off' (-not (& $aiOn))
    $add = Scroll-To $window 'Add the command line to my PATH'
    if ($add) {
        Click $window $add
        Start-Sleep -Milliseconds 600
        # The confirm panel opens in the card, below the fold at this size.
        Check 'settings: Add first shows what it will change' ([bool](Scroll-To $window 'Add to PATH'))
        Click $window (Find-Control $window 'Close settings')
        Start-Sleep -Milliseconds 500
        Click $window (Find-Control $window 'Settings')
        Start-Sleep -Milliseconds 500
        Check 'settings: leaving without confirming changes nothing' ([bool](Scroll-To $window 'Add the command line to my PATH') -and [Environment]::GetEnvironmentVariable('Path', 'User') -eq $pathBefore)
    } else {
        Check 'settings: vmm.exe is beside the app and offered for PATH' ([bool](Scroll-To $window 'Take the command line out of my PATH'))
    }
    # "Copy diagnostics" replaces the clipboard, so only on CI, never on a
    # developer's machine.
    if ($env:CI) {
        # The last card: scroll on to the end of the sheet, so the button is
        # wholly in view and not clipped at the bottom edge.
        $copy = Scroll-To $window 'Copy diagnostics'
        Scroll-More $window 6
        Click $window (Find-Control $window 'Copy diagnostics')
        Start-Sleep -Milliseconds 800
        $copied = try { Get-Clipboard -Raw | ConvertFrom-Json } catch { $null }
        Check 'settings: Copy diagnostics copies the JSON' ($copied -and $copied.schemaVersion -eq 1 -and $copied.status.installed -gt 0)
    }
    Click $window (Find-Control $window 'Close settings')
    Start-Sleep -Milliseconds 400
} finally {
    if (-not $p.HasExited) { $p.Kill(); $p.WaitForExit() }
}
Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
if ($failed) { exit 1 }
exit 0

# furnish updates itself on Windows: the installed .msi (this build) takes the next version from a
# feed on this computer, first in the desktop app (automatic, hidden in the tray, restarted by the
# installer), then by a person in the open app (Settings → About: Restart to update, pressed
# through UI Automation), then with `furnish self-update` while the furnish service runs,
# which the installer stops and starts again. Run as administrator on a throwaway machine (CI).
#
#   tests/self-update.ps1 -Msi furnish_0.1.0_x64.msi -Feed <folder with latest.json, windows/…>
#
# The feed's folder comes from the update-feed job: the next version's .msi, signed, and
# latest.json pointing at http://127.0.0.1:8765.
param([Parameter(Mandatory)][string]$Msi, [Parameter(Mandatory)][string]$Feed)
$ErrorActionPreference = 'Stop'
$app = "$env:ProgramFiles\furnish"
$next = (Get-Content "$Feed\latest.json" -Raw | ConvertFrom-Json).version -replace '-alpha\.', '+alpha.'

function Fail([string]$Message) {
    Write-Host "FAIL: $Message"
    Get-ChildItem $app -ErrorAction SilentlyContinue | Format-Table Name, Length, LastWriteTime
    Get-CimInstance Win32_Process -Filter "Name = 'msiexec.exe' or Name like 'furnish%'" | ForEach-Object { "$($_.ProcessId) $($_.CommandLine)" }
    Get-WinEvent -LogName Application -MaxEvents 30 -ErrorAction SilentlyContinue | Where-Object ProviderName -eq 'MsiInstaller' | Format-List TimeCreated, Message
    exit 1
}

function Install-Msi([string]$Path) {
    $p = Start-Process msiexec -ArgumentList "/i `"$Path`" /qn /norestart" -Wait -PassThru
    if ($p.ExitCode) { Fail "msiexec /i $Path exited with $($p.ExitCode)" }
}

# The app's buttons as Windows UI Automation sees them (WebView2 exposes the page's), the way a
# screen reader finds and presses them.
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$UIA = [System.Windows.Automation.AutomationElement]
function Find-Button($Window, [string]$Name, [int]$Seconds = 60) {
    $isButton = New-Object System.Windows.Automation.PropertyCondition($UIA::ControlTypeProperty, [System.Windows.Automation.ControlType]::Button)
    $deadline = (Get-Date).AddSeconds($Seconds)
    while ((Get-Date) -lt $deadline) {
        $found = $Window.FindAll([System.Windows.Automation.TreeScope]::Descendants, $isButton) | Where-Object { $_.Current.Name -like $Name } | Select-Object -First 1
        if ($found) { return $found }
        Start-Sleep 1
    }
    $names = ($Window.FindAll([System.Windows.Automation.TreeScope]::Descendants, $isButton) | ForEach-Object { $_.Current.Name }) -join ' | '
    Fail "no button '$Name' within $Seconds s; the window has: $names"
}
function Press($Button) { $Button.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke() }

function Version { try { ((& "$app\furnish.exe" --version) -replace '^furnish ', '').Trim() } catch { '(none: an install is running?)' } }

# An update is done when no installer runs any more: neither a client (msiexec /i) nor the
# install itself (Windows Installer holds Global\_MSIExecute while it runs).
function Wait-Installed([string]$What, [int]$Seconds = 300) {
    $deadline = (Get-Date).AddSeconds($Seconds)
    $started = $false
    while ((Get-Date) -lt $deadline) {
        $client = Get-CimInstance Win32_Process -Filter "Name = 'msiexec.exe'" | Where-Object CommandLine -match '/i\s'
        $m = $null
        $busy = [System.Threading.Mutex]::TryOpenExisting('Global\_MSIExecute', [ref]$m)
        if ($m) { $m.Dispose() }
        if ($client -or $busy) { $started = $true }
        elseif ($started) { Start-Sleep 3; return }
        Start-Sleep 2
    }
    Fail "$What`: no installer ran and finished within $Seconds s (started: $started)"
}

$old = Get-Item $Msi
Write-Host "== $($old.Name) installed, the feed offers $next"
Install-Msi $old.FullName
$current = Version
if ($current -eq $next) { Fail "the installed version is already $next" }
$gui = Get-ChildItem "$app\*.exe" | Where-Object Name -ne 'furnish.exe' | Select-Object -First 1
if (-not $gui) { Fail 'no desktop app in the install folder' }
$server = Start-Process python -ArgumentList '-m', 'http.server', '8765', '--bind', '127.0.0.1', '--directory', (Resolve-Path $Feed) -PassThru -WindowStyle Hidden
Start-Sleep 2
$env:FURNISH_UPDATE_FEED = 'http://127.0.0.1:8765/latest.json'
try {
    $channel = if ($next -like '*+alpha.*') { 'alpha' } else { 'stable' }
    & "$app\furnish.exe" settings --update-channel $channel
    if ($LASTEXITCODE) { Fail "could not select $channel" }
    Write-Host "== the desktop app ($($gui.Name), hidden) updates itself"
    Start-Process $gui.FullName -ArgumentList '--hidden'
    Wait-Installed 'the desktop app'
    $guiVersion = (& $gui.FullName --build-info | ConvertFrom-Json).version
    if ($guiVersion -ne (Get-Content "$Feed\latest.json" -Raw | ConvertFrom-Json).version) { Fail "updated desktop reports $guiVersion" }
    $v = Version
    if ($v -ne $next) { Fail "after the desktop app's update: $v, expected $next" }
    for ($i = 0; $i -lt 30 -and -not (Get-Process ($gui.BaseName) -ErrorAction SilentlyContinue); $i++) { Start-Sleep 1 }
    if (-not (Get-Process ($gui.BaseName) -ErrorAction SilentlyContinue)) { Fail "the installer didn't start the desktop app again" }
    Write-Host "✓ $current -> $v, and the app runs again"
    Get-Process ($gui.BaseName) | Stop-Process -Force

    # The button a person presses: with `auto` the update downloads on its own, but waits while
    # the window is open (until it is closed to the tray) unless someone chooses Restart to update.
    # (Update now, offered with `notify`, downloads and restarts in one go.)
    Write-Host "== Restart to update in Settings → About (the window open)"
    Install-Msi $old.FullName
    if ((Version) -ne $current) { Fail "reinstalling $($old.Name) left $(Version)" }
    $running = Start-Process $gui.FullName -PassThru
    for ($i = 0; $i -lt 60 -and (Get-Process -Id $running.Id).MainWindowHandle -eq [IntPtr]::Zero; $i++) { Start-Sleep 1 }
    $handle = (Get-Process -Id $running.Id).MainWindowHandle
    if ($handle -eq [IntPtr]::Zero) { Fail 'the desktop app shows no window' }
    $window = $UIA::FromHandle($handle)
    Press (Find-Button $window 'Settings*')
    $restart = Find-Button $window 'Restart to update' 120
    # Downloaded, and still not installed while the window is open: only the button installs it.
    Start-Sleep 10
    if (-not $restart.Current.IsEnabled) { Fail 'Restart to update is disabled' }
    if ((Version) -ne $current) { Fail "updated to $(Version) before Restart to update was pressed" }
    try { Press $restart } catch { Write-Host "(the app ended while pressing: $($_.Exception.Message))" }
    Wait-Installed 'Restart to update'
    $v = Version
    if ($v -ne $next) { Fail "after Restart to update: $v, expected $next" }
    for ($i = 0; $i -lt 30 -and -not (Get-Process ($gui.BaseName) -ErrorAction SilentlyContinue | Where-Object Id -ne $running.Id); $i++) { Start-Sleep 1 }
    if (-not (Get-Process ($gui.BaseName) -ErrorAction SilentlyContinue | Where-Object Id -ne $running.Id)) { Fail "the installer didn't start the desktop app again" }
    Write-Host "✓ $current -> $v after Restart to update, and the app runs again"
    Get-Process ($gui.BaseName) | Stop-Process -Force

    Write-Host "== furnish self-update, with the furnish service running"
    Install-Msi $old.FullName
    if ((Version) -ne $current) { Fail "reinstalling $($old.Name) left $(Version)" }
    & "$app\furnish.exe" service install
    if ($LASTEXITCODE) { Fail 'service install failed' }
    $before = Get-Date
    & "$app\furnish.exe" self-update
    if ($LASTEXITCODE) { Fail "furnish self-update exited with $LASTEXITCODE" }
    Wait-Installed 'furnish self-update'
    $v = Version
    if ($v -ne $next) { Fail "after furnish self-update: $v, expected $next" }
    $service = Get-CimInstance Win32_Service -Filter "Name = 'furnish'"
    if ($service.State -ne 'Running') { Fail "the furnish service is $($service.State) after the update" }
    $process = Get-Process -Id $service.ProcessId
    if ($process.StartTime -lt $before) { Fail "the furnish service wasn't restarted (it runs since $($process.StartTime))" }
    if ($process.Path -ne "$app\furnish.exe") { Fail "the service runs $($process.Path)" }
    Write-Host "✓ $current -> $v; the service runs the new furnish.exe since $($process.StartTime)"
    & "$app\furnish.exe" self-update --check
    if ($LASTEXITCODE) { Fail 'self-update --check failed' }
} finally {
    Stop-Process $server -ErrorAction SilentlyContinue
    & "$app\furnish.exe" service uninstall 2>$null
}

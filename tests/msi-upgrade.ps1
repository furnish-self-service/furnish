# Upgrading an earlier release's MSI to this build, with the furnish service installed and running: the way an admin deploys
# a new version (msiexec /i /qn /norestart). The service must run the new furnish.exe afterwards,
# and the upgrade must not need a reboot. Run as administrator on a throwaway machine (CI); it
# leaves furnish uninstalled.
#
#   tests/msi-upgrade.ps1 -From furnish_0.1.0-alpha.177_x64.msi -To furnish_0.1.0_x64.msi
param([Parameter(Mandatory)][string]$From, [Parameter(Mandatory)][string]$To)
$ErrorActionPreference = 'Stop'
$app = "$env:ProgramFiles\furnish"
$logs = "$env:RUNNER_TEMP\msi-upgrade"
New-Item -ItemType Directory $logs -Force | Out-Null

# Files Windows replaces or deletes at the next start (the upgrade's, if new after it).
function Pending {
    @((Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager' -Name PendingFileRenameOperations -ErrorAction SilentlyContinue).PendingFileRenameOperations |
        Where-Object { $_ })
}

function Fail([string]$Message) {
    Write-Host "FAIL: $Message"
    Get-ChildItem $app -ErrorAction SilentlyContinue | Format-Table Name, Length, LastWriteTime
    Get-CimInstance Win32_Service -Filter "Name = 'furnish'" | Format-List State, ProcessId, PathName
    Write-Host '--- files waiting for a reboot'; Pending | Where-Object { $_ -notin $pendingBefore }
    $log = "$logs\upgrade.log"
    if (Test-Path $log) {
        Write-Host "--- ${log}: in use, Restart Manager, reboots, services"
        Select-String -Path $log -Pattern 'FilesInUse|RestartManager|RmShutdown|REBOOT|ScheduleReboot|in use|StopServices|StartFurnishService|StopFurnishService|RemoveExistingProducts|Return value 3|error' |
            Select-Object -First 80 | ForEach-Object Line
    }
    Get-WinEvent -LogName System -MaxEvents 20 -ErrorAction SilentlyContinue | Where-Object ProviderName -eq 'Service Control Manager' | Format-List TimeCreated, Message
    exit 1
}

function Msiexec([string]$Arguments) {
    $p = Start-Process msiexec -ArgumentList $Arguments -Wait -PassThru
    Write-Host "+ msiexec $Arguments -> $($p.ExitCode)"
    $p.ExitCode
}

function Version { ((& "$app\furnish.exe" --version) -replace '^furnish ', '').Trim() }

Write-Host "== $(Split-Path -Leaf $From) with the furnish service running"
if (($code = Msiexec "/i `"$From`" /qn /norestart /l*v `"$logs\old.log`"") -ne 0) { Fail "installing $From exited with $code" }
$old = Version
& "$app\furnish.exe" service install
if ($LASTEXITCODE) { Fail 'service install failed' }
$service = Get-CimInstance Win32_Service -Filter "Name = 'furnish'"
if ($service.State -ne 'Running') { Fail "the furnish service is $($service.State)" }
$oldPid = $service.ProcessId
Write-Host "furnish $old, the service runs as process $oldPid"

try {
    Write-Host "== upgrade to $(Split-Path -Leaf $To)"
    $pendingBefore = Pending
    $before = Get-Date
    $code = Msiexec "/i `"$To`" /qn /norestart /l*v `"$logs\upgrade.log`""
    # Everything first, then what's wrong: one run shows all of it.
    $new = Version
    $reboot = @(Pending | Where-Object { $_ -notin $pendingBefore })
    $service = Get-CimInstance Win32_Service -Filter "Name = 'furnish'"
    $process = if ($service.ProcessId) { Get-Process -Id $service.ProcessId -ErrorAction SilentlyContinue }
    Write-Host "exit code $code; furnish $new; service $($service.State), process $($service.ProcessId) ($($process.Path), since $($process.StartTime)); $($reboot.Count) file operation(s) wait for a reboot"
    $problems = @()
    if ($code -eq 3010) { $problems += 'the upgrade asks for a reboot (3010)' }
    elseif ($code -ne 0) { $problems += "the upgrade exited with $code" }
    if ($reboot) { $problems += "file operations wait for a reboot: $($reboot -join ', ')" }
    if ($new -eq $old) { $problems += "furnish is still $old" }
    # The service runs furnish.exe from the install folder, started after the upgrade began: the
    # new one. (An old process kept running would show the path it was moved to, or the old start.)
    if ($service.State -ne 'Running') { $problems += "the furnish service is $($service.State) after the upgrade" }
    elseif ($service.ProcessId -eq $oldPid -or $process.StartTime -lt $before) { $problems += "the furnish service wasn't restarted (it runs since $($process.StartTime))" }
    elseif ($process.Path -ne "$app\furnish.exe") { $problems += "the service runs $($process.Path)" }
    if ($problems) { Fail ($problems -join '; ') }
    Write-Host "✓ $old -> $new without a reboot; the service runs the new furnish.exe since $($process.StartTime)"
} finally {
    & "$app\furnish.exe" service uninstall 2>$null
    Msiexec "/x `"$To`" /qn /norestart" | Out-Null
}

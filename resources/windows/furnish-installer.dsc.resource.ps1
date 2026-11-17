# Furnish/Installer on Windows: install an .msi or .exe shipped in the collection.
# furnish passes `file` as an absolute path inside the collection. DSC passes the resource
# properties as env vars: file, productCode (.msi, optional), displayName and installArgs
# (.exe), uninstallArgs (.exe, optional), _exist, and from furnish's required versions:
#   reportVersions     latest or all: every state also has versionScheme (generic), via,
#                      latestVersion (.msi: the file's ProductVersion) and, with all,
#                      availableVersions
#   targetVersion      the version to have; an installer can only install the one in its file
#
#   get                print the current state
#   test               current state with _inDesiredState (with targetVersion: installed is that one)
#   set [what-if]      converge; with what-if only describe the steps (_metadata.whatIf)
#
# Exit codes: 10 invalid input, 11 missing tool or file, 12 cancelled (UAC prompt declined).
param([string]$Operation, [string]$Mode)

function Fail([int]$Code, [string]$Message) {
    [Console]::Error.WriteLine((@{ error = $Message } | ConvertTo-Json -Compress)); exit $Code
}

$file = $env:file
if (-not $file) { Fail 10 'property file is required' }
$report = $env:reportVersions
$target = $env:targetVersion
if ($report -and $report -cnotin 'latest', 'all') { Fail 10 "reportVersions must be latest or all: $report" }
if ($target -and $target.StartsWith('-')) { Fail 10 "targetVersion starts with '-': $target" }
if ($target -and $target -cnotmatch '^[A-Za-z0-9.+~:_^-]+$') { Fail 10 "targetVersion has characters a version can't have: $target" }
$manager = 'its installer'
$fileVersion = $null
function Write-State([string]$Version, [string[]]$WhatIf, $InDesiredState) {
    # Echo back every identifying property: DSC's test compares each desired property with the state.
    $state = [ordered]@{ file = $file }
    foreach ($p in 'productCode', 'displayName', 'installArgs', 'uninstallArgs') {
        $v = [Environment]::GetEnvironmentVariable($p)
        if ($null -ne $v) { $state[$p] = $v }
    }
    $state._exist = [bool]$Version
    if ($Version -and $Version -ne '?') { $state.version = $Version }
    if ($report) {
        $state.versionScheme = 'generic'
        $state.via = $manager
        if ($fileVersion) { $state.latestVersion = $fileVersion }
        if ($report -eq 'all' -and $fileVersion) { $state.availableVersions = @($fileVersion) }
    }
    if ($WhatIf) { $state._metadata = @{ whatIf = $WhatIf } }
    if ($null -ne $InDesiredState) { $state._inDesiredState = [bool]$InDesiredState }
    [Console]::Out.WriteLine(($state | ConvertTo-Json -Compress -Depth 4))
}

$subject = Split-Path -Leaf $file
# A link (symbolic link, junction) could point anywhere outside the collection. GetAttributes reads
# the link itself, so a dangling one counts too; it throws only when nothing is there.
$attributes = try { [IO.File]::GetAttributes($file) } catch { $null }
if ($null -ne $attributes -and ($attributes -band [IO.FileAttributes]::ReparsePoint)) {
    Fail 10 "installer is a symbolic link: $file"
}
if (-not (Test-Path -LiteralPath $file -PathType Leaf)) {
    # Left out of the checkout (blobsOnDemand): furnish downloads it when the app is installed.
    # Until then the app counts as not installed, and only an actual install needs the file.
    if ($Operation -eq 'get') { Write-State ''; exit 0 }
    if ($Operation -eq 'test') { Write-State '' $null ($env:_exist -eq 'false'); exit 0 }
    if ($Operation -eq 'set' -and $Mode -eq 'what-if') {
        # The state after the step: installed, like the Unix resource's what-if reports.
        if ($env:_exist -ne 'false' -and $target) { Fail 10 "its installer can't install a specific version ($target)" }
        if ($env:_exist -ne 'false') { Write-State '?' @("Download $subject from the collection, then install it as administrator") }
        else { Write-State '' @("$subject is not installed") }
        exit 0
    }
    Fail 11 "installer not found: $file"
}
if ((Get-Content -LiteralPath $file -TotalCount 1 -ErrorAction SilentlyContinue) -like 'version https://git-lfs*') {
    [Console]::Error.WriteLine("furnish: needs git-lfs to download $subject (install it, then update the collection)"); exit 11
}

$uninstallKeys = @(
    'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*',
    'HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*',
    'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*'
)
function Find-Installed([scriptblock]$Match) {
    Get-ItemProperty -Path $uninstallKeys -ErrorAction SilentlyContinue | Where-Object $Match | Select-Object -First 1
}

switch -Wildcard ($file) {
    '*.msi' {
        $code = $env:productCode
        if (-not $code -or $report -or $target) {
            # Read the ProductCode (and ProductVersion) from the package itself.
            $wi = New-Object -ComObject WindowsInstaller.Installer
            $db = $wi.GetType().InvokeMember('OpenDatabase', 'InvokeMethod', $null, $wi, @($file, 0))
            function Get-MsiProperty([string]$Property) {
                $view = $db.GetType().InvokeMember('OpenView', 'InvokeMethod', $null, $db, @("SELECT Value FROM Property WHERE Property='$Property'"))
                $view.GetType().InvokeMember('Execute', 'InvokeMethod', $null, $view, $null) | Out-Null
                $record = $view.GetType().InvokeMember('Fetch', 'InvokeMethod', $null, $view, $null)
                if ($null -eq $record) { return $null }
                $record.GetType().InvokeMember('StringData', 'GetProperty', $null, $record, 1)
            }
            if (-not $code) { $code = Get-MsiProperty 'ProductCode' }
            if ($report -or $target) { $fileVersion = Get-MsiProperty 'ProductVersion' }
        }
        $installed = Find-Installed { $_.PSChildName -eq $code }
        $installCmd = @('msiexec.exe', "/i `"$file`" /qn /norestart")
        $removeCmd = @('msiexec.exe', "/x $code /qn /norestart")
        $manager = 'Windows Installer'
    }
    '*.exe' {
        if (-not $env:displayName) { Fail 10 '.exe installers need the property displayName (as shown in Apps & features)' }
        if ($null -eq $env:installArgs) { Fail 10 '.exe installers need the property installArgs (silent install switches, e.g. /S)' }
        $installed = Find-Installed { $_.DisplayName -like $env:displayName }
        $installCmd = @($file, $env:installArgs)
        $removeCmd = $null
        if ($installed) {
            $uninstall = if ($installed.QuietUninstallString) { $installed.QuietUninstallString } else { "$($installed.UninstallString) $($env:uninstallArgs)" }
            $removeCmd = @('cmd.exe', "/c `"$uninstall`"")
        }
    }
    default { Fail 10 "unsupported installer: $subject (Windows: .msi, .exe)" }
}

$version = if ($installed) { if ($installed.DisplayVersion) { [string]$installed.DisplayVersion } else { '?' } } else { '' }

# Runs elevated (UAC prompt unless already elevated, e.g. as the furnish service) and waits.
# A declined prompt is a cancel.
function Invoke-Elevated([string[]]$Command) {
    $elevated = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    try {
        if ($elevated) {
            $p = Start-Process -FilePath $Command[0] -ArgumentList $Command[1] -Wait -PassThru -WindowStyle Hidden
        } else {
            $p = Start-Process -FilePath $Command[0] -ArgumentList $Command[1] -Verb RunAs -Wait -PassThru -WindowStyle Hidden
        }
    } catch {
        [Console]::Error.WriteLine('{"warn":"administrator rights were not granted"}'); exit 12
    }
    switch ($p.ExitCode) {
        0 { }
        3010 { [Console]::Error.WriteLine('{"warn":"a restart is needed to finish"}') }
        1602 { [Console]::Error.WriteLine('{"warn":"cancelled in the installer"}'); exit 12 }
        default { [Console]::Error.WriteLine((@{ error = "$($Command[0]) failed with exit code $($p.ExitCode)" } | ConvertTo-Json -Compress)); exit $p.ExitCode }
    }
}

switch ($Operation) {
    'get' { Write-State $version }
    'test' {
        $ok = if ($env:_exist -eq 'false') { -not $version } elseif ($target) { $version -eq $target } else { [bool]$version }
        Write-State $version $null $ok
    }
    'set' {
        $want = $env:_exist -ne 'false'
        $shown = if ($version -and $version -ne '?') { "$subject $version" } else { $subject }
        $cmd = $null
        if ($want -and $target) {
            # Only the version in the file can be the target: installing the file brings it.
            $after = $target
            if ($version -eq $target) { $step = "$subject $target is installed, the required version" }
            elseif ($fileVersion -ne $target) { Fail 10 "$manager can't install a specific version ($target)" }
            elseif (-not $version) { $step = "Install $subject $target with $manager"; $cmd = $installCmd }
            else {
                $step = if ($version -eq '?') { "Update $subject to $target with $manager" } else { "Update $subject from $version to $target with $manager" }
                $cmd = $installCmd
            }
        }
        elseif ($want -and -not $version) { $step = "Install $subject with $manager"; $cmd = $installCmd; $after = '?' }
        elseif (-not $want -and $version) {
            if (-not $removeCmd) { Fail 10 "no uninstall command registered for $subject" }
            $step = "Remove $shown with $manager"; $cmd = $removeCmd; $after = ''
        }
        elseif ($version) { $step = "$shown is already installed"; $after = $version }
        else { $step = "$subject is not installed"; $after = '' }

        if ($Mode -eq 'what-if') {
            $whatIf = @($step)
            if ($cmd) { $whatIf += "Run as administrator: $($cmd -join ' ')" }
            Write-State $after $whatIf
            exit 0
        }
        if ($cmd) {
            [Console]::Error.WriteLine((@{ info = "$($step): running as administrator: $($cmd -join ' ')" } | ConvertTo-Json -Compress))
            Invoke-Elevated $cmd
        }
        $installed = if ($file -like '*.msi') { Find-Installed { $_.PSChildName -eq $code } } else { Find-Installed { $_.DisplayName -like $env:displayName } }
        Write-State $(if ($installed) { if ($installed.DisplayVersion) { [string]$installed.DisplayVersion } else { '?' } } else { '' })
    }
    default { Fail 10 'usage: get|set [what-if]' }
}

# Furnish/Installer on Windows: install an .msi or .exe shipped in the collection.
# furnish passes `file` as an absolute path inside the collection. DSC passes the resource
# properties as env vars: file, productCode (.msi, optional), displayName and installArgs
# (.exe), uninstallArgs (.exe, optional), _exist.
#
#   get                print the current state
#   set [what-if]      converge; with what-if only describe the steps (_metadata.whatIf)
#
# Exit codes: 10 invalid input, 11 missing tool or file, 12 cancelled (UAC prompt declined).
param([string]$Operation, [string]$Mode)

function Fail([int]$Code, [string]$Message) {
    [Console]::Error.WriteLine((@{ error = $Message } | ConvertTo-Json -Compress)); exit $Code
}

$file = $env:file
if (-not $file) { Fail 10 'property file is required' }
if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { Fail 11 "installer not found: $file" }
$subject = Split-Path -Leaf $file
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
        if (-not $code) {
            # Read the ProductCode from the package itself.
            $wi = New-Object -ComObject WindowsInstaller.Installer
            $db = $wi.GetType().InvokeMember('OpenDatabase', 'InvokeMethod', $null, $wi, @($file, 0))
            $view = $db.GetType().InvokeMember('OpenView', 'InvokeMethod', $null, $db, @("SELECT Value FROM Property WHERE Property='ProductCode'"))
            $view.GetType().InvokeMember('Execute', 'InvokeMethod', $null, $view, $null) | Out-Null
            $record = $view.GetType().InvokeMember('Fetch', 'InvokeMethod', $null, $view, $null)
            $code = $record.GetType().InvokeMember('StringData', 'GetProperty', $null, $record, 1)
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
        $manager = 'its installer'
    }
    default { Fail 10 "unsupported installer: $subject (Windows: .msi, .exe)" }
}

$version = if ($installed) { if ($installed.DisplayVersion) { [string]$installed.DisplayVersion } else { '?' } } else { '' }

function Write-State([string]$Version, [string[]]$WhatIf) {
    # Echo back every identifying property: DSC's test compares each desired property with the state.
    $state = [ordered]@{ file = $file }
    foreach ($p in 'productCode', 'displayName', 'installArgs', 'uninstallArgs') {
        $v = [Environment]::GetEnvironmentVariable($p)
        if ($null -ne $v) { $state[$p] = $v }
    }
    $state._exist = [bool]$Version
    if ($Version -and $Version -ne '?') { $state.version = $Version }
    if ($WhatIf) { $state._metadata = @{ whatIf = $WhatIf } }
    [Console]::Out.WriteLine(($state | ConvertTo-Json -Compress -Depth 4))
}

# Runs elevated (UAC prompt unless already elevated) and waits. A declined prompt is a cancel.
function Invoke-Elevated([string[]]$Command) {
    try {
        $p = Start-Process -FilePath $Command[0] -ArgumentList $Command[1] -Verb RunAs -Wait -PassThru -WindowStyle Hidden
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
    'set' {
        $want = $env:_exist -ne 'false'
        $shown = if ($version -and $version -ne '?') { "$subject $version" } else { $subject }
        $cmd = $null
        if ($want -and -not $version) { $step = "Install $subject with $manager"; $cmd = $installCmd; $after = '?' }
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

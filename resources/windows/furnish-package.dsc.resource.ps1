# Furnish/Package on Windows: manage a package with winget.
# DSC passes the resource properties as env vars: $env:name (a winget package ID), $env:command
# (optional: a program the package provides) and $env:_exist.
#
#   get                  print the current state
#   test                 current state with _inDesiredState (an available update: not in desired state)
#   set [what-if]        converge (install, update or remove); with what-if only describe the
#                        steps (in _metadata.whatIf) and print the state it would lead to
param([string]$Operation, [string]$Mode)

$name = $env:name
# Own exit codes start at 10; winget failures exit with winget's (unmapped) code so DSC shows stderr.
if (-not $name) { [Console]::Error.WriteLine('{"error":"property name is required"}'); exit 10 }
# Not an option to winget (the schema says so too; this holds without it).
if ($name.StartsWith('-')) { [Console]::Error.WriteLine((@{ error = "package name starts with '-': $name" } | ConvertTo-Json -Compress)); exit 10 }
$command = $env:command
if ($command -and ($command -match '[\\/:]' -or $command.StartsWith('-'))) {
    [Console]::Error.WriteLine((@{ error = "command must be a program's name, without a folder: $command" } | ConvertTo-Json -Compress)); exit 10
}
# SYSTEM (the furnish service) doesn't have winget on its PATH: use App Installer's copy.
if (-not (Get-Command winget -ErrorAction SilentlyContinue)) {
    $found = Get-ChildItem "$env:ProgramFiles\WindowsApps\Microsoft.DesktopAppInstaller_*_x64__8wekyb3d8bbwe\winget.exe" -ErrorAction SilentlyContinue |
        Sort-Object { [version]($_.Directory.Name -split '_')[1] } | Select-Object -Last 1
    if ($found) { Set-Alias winget $found.FullName }
}
$haveWinget = [bool](Get-Command winget -ErrorAction SilentlyContinue)

$common = @('--exact', '--id', $name, '--accept-source-agreements', '--disable-interactivity')
$installArgs = @('install') + $common + @('--silent', '--accept-package-agreements')
$removeArgs = @('uninstall') + $common + @('--silent')
$upgradeArgs = @('upgrade') + $common + @('--silent', '--accept-package-agreements')

# winget output goes to stderr: DSC reads stdout as JSON.
function Invoke-Winget([string[]]$WingetArgs) {
    & winget @WingetArgs 2>&1 | ForEach-Object { [Console]::Error.WriteLine($_) }
    return $LASTEXITCODE
}
function Test-Installed { (Invoke-Winget (@('list') + $common)) -eq 0 }
# winget's exit code for `list --upgrade-available` isn't a reliable "newer version" signal (e.g.
# portable packages with an unknown installed version), so read the row: Name, Id, Version,
# Available, Source. Only a known installed version that differs from Available counts.
function Test-Outdated {
    $out = & winget list --upgrade-available @common 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { return $false }
    $row = ($out -split "`r?`n") | Where-Object { $_ -match "(^|\s)$([regex]::Escape($name))(\s|$)" } | Select-Object -Last 1
    [Console]::Error.WriteLine((@{ trace = "winget upgrade row: $row" } | ConvertTo-Json -Compress))
    if (-not $row) { return $false }
    $cols = $row.Trim() -split '\s+'
    $i = [array]::IndexOf($cols, $name)
    if ($i -lt 0 -or $cols.Count -lt $i + 3) { return $false }
    $installed, $available = $cols[$i + 1], $cols[$i + 2]
    return ($installed -notmatch '^(Unknown|<)' -and $available -and $available -ne $installed -and $available -ne 'winget')
}
# The program $command, where people's installers put programs: PATH, then the folders that
# aren't always on it (~\.local\bin for vendors' installers, npm's global folder). Only looks.
function Find-Program {
    $found = Get-Command $command -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($found) { return $found.Source }
    $candidates = @()
    if ($env:USERPROFILE) { $candidates += "$env:USERPROFILE\.local\bin\$command.exe" }
    if ($env:APPDATA) { $candidates += "$env:APPDATA\npm\$command.cmd" }
    return $candidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
}
function Write-State([bool]$Exist, [string[]]$WhatIf, $InDesiredState) {
    $state = [ordered]@{ name = $name; _exist = $Exist }
    if ($outside) { $state.managedBy = 'other'; $state.path = $outside }
    if ($WhatIf) { $state._metadata = @{ whatIf = $WhatIf } }
    if ($null -ne $InDesiredState) { $state._inDesiredState = [bool]$InDesiredState }
    [Console]::Out.WriteLine(($state | ConvertTo-Json -Compress -Depth 4))
}

# Installed some other way (npm, the vendor's installer): the program is there although winget
# doesn't have the package. It counts as installed, not by furnish, which neither installs it
# again nor updates or removes it.
$outside = $null
if ($command -and -not ($haveWinget -and (Test-Installed))) { $outside = Find-Program }
if (-not $haveWinget -and -not $outside) {
    # Plain text and an unmapped exit code: furnish shows "Needs winget" instead of a failure.
    [Console]::Error.WriteLine('furnish: needs winget (App Installer from the Microsoft Store)'); exit 11
}
if ($outside) {
    $want = $env:_exist -ne 'false'
    switch ($Operation) {
        'get' { Write-State $true }
        'test' { Write-State $true $null $want }
        'set' {
            $step = "$name is installed and up to date"
            if (-not $want) {
                $step = "$name was installed outside furnish ($outside): uninstall it the way it was installed"
                # Plain text and an unmapped exit code, so DSC keeps the line in its error.
                if ($Mode -ne 'what-if') { [Console]::Error.WriteLine("furnish: $step"); exit 14 }
            }
            Write-State $true $(if ($Mode -eq 'what-if') { @($step) })
        }
        default { [Console]::Error.WriteLine('{"error":"usage: get|test|set [what-if]"}'); exit 10 }
    }
    exit 0
}

switch ($Operation) {
    'get' { Write-State (Test-Installed) }
    'test' {
        $installed = Test-Installed
        $ok = if ($env:_exist -ne 'false') { $installed -and -not (Test-Outdated) } else { -not $installed }
        Write-State $installed $null $ok
    }
    'set' {
        $installed = Test-Installed
        $want = $env:_exist -ne 'false'
        $wingetArgs = $null
        if ($want -and -not $installed) { $step = "Install $name with winget"; $wingetArgs = $installArgs }
        elseif ($want -and (Test-Outdated)) { $step = "Update $name with winget"; $wingetArgs = $upgradeArgs }
        elseif (-not $want -and $installed) { $step = "Remove $name with winget"; $wingetArgs = $removeArgs }
        elseif ($installed) { $step = "$name is installed and up to date" }
        else { $step = "$name is not installed" }

        if ($Mode -eq 'what-if') {
            $whatIf = @($step)
            if ($wingetArgs) { $whatIf += "Run: winget $($wingetArgs -join ' ')" }
            Write-State $want $whatIf
            exit 0
        }
        if ($wingetArgs) {
            [Console]::Error.WriteLine((@{ info = "$($step): running winget $($wingetArgs -join ' ')" } | ConvertTo-Json -Compress))
            $code = Invoke-Winget $wingetArgs
            if ($code -ne 0) { [Console]::Error.WriteLine("{`"error`":`"winget failed with exit code $code`"}"); exit $code }
        }
        Write-State (Test-Installed)
    }
    default { [Console]::Error.WriteLine('{"error":"usage: get|test|set [what-if]"}'); exit 10 }
}

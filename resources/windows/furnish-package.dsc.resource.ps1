# Furnish/Package on Windows: manage a package with winget.
# The native resource passes its resolved $env:name (a winget package ID), $env:command
# (optional: a program the package provides), $env:_exist,
# and from furnish's required versions (furnish compares, this script reports and obeys):
#   reportVersions   latest or all: every state also has version (if known), versionScheme, via,
#                    latestVersion (if known) and, with all, availableVersions
#   targetVersion    install or update to exactly this version; test is in desired state iff the
#                    installed version is this one
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
$report = $env:reportVersions
$target = $env:targetVersion
if ($report -and $report -cnotin 'latest', 'all') { [Console]::Error.WriteLine((@{ error = "reportVersions must be latest or all: $report" } | ConvertTo-Json -Compress)); exit 10 }
if ($target -and $target.StartsWith('-')) { [Console]::Error.WriteLine((@{ error = "targetVersion starts with '-': $target" } | ConvertTo-Json -Compress)); exit 10 }
if ($target -and $target -cnotmatch '^[A-Za-z0-9.+~:_^-]+$') { [Console]::Error.WriteLine((@{ error = "targetVersion has characters a version can't have: $target" } | ConvertTo-Json -Compress)); exit 10 }
# Furnish resolves the same executable for collection conditions and package operations.
# Under SYSTEM this can be the native App Installer package outside PATH.
if ($env:FURNISH_WINGET) { Set-Alias winget $env:FURNISH_WINGET }
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
# The column after the ID in the row of a winget table (list: Version, upgrade: Version then
# Available). Nothing if there is no such row.
function Get-Column([string]$Out, [int]$Offset) {
    $row = ($Out -split "`r?`n") | Where-Object { $_ -match "(^|\s)$([regex]::Escape($name))(\s|$)" } | Select-Object -Last 1
    if (-not $row) { return $null }
    $cols = $row.Trim() -split '\s+'
    $i = [array]::IndexOf($cols, $name)
    if ($i -lt 0 -or $cols.Count -le $i + $Offset) { return $null }
    return $cols[$i + $Offset]
}
# The installed version: '' if not installed, '?' if winget doesn't know it ("Unknown", "< 1.2").
function Get-Installed {
    $out = & winget list @common 2>&1 | Out-String
    [Console]::Error.WriteLine($out)
    if ($LASTEXITCODE -ne 0) { return '' }
    $v = Get-Column $out 1
    if (-not $v -or $v -match '^(Unknown|<)' -or $v -match '\u2026') { return '?' }
    return $v
}
# The newest version winget offers ("Version: 1.2" in winget show).
function Get-Latest {
    $out = & winget show @common 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { return $null }
    foreach ($l in $out -split "`r?`n") { if ($l -match '^\s*Version:\s*(\S+)\s*$') { return $Matches[1] } }
    return $null
}
# Every version winget can install: the lines under the dashes in winget show --versions.
function Get-Available {
    $out = & winget show @common --versions 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { return @() }
    $seen = $false
    $versions = @()
    foreach ($l in $out -split "`r?`n") {
        if ($seen) { if ($l.Trim()) { $versions += $l.Trim() } }
        elseif ($l -match '^-{3,}\s*$') { $seen = $true }
    }
    return $versions
}
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
function Write-State([string]$Version, [string[]]$WhatIf, $InDesiredState) {
    $state = [ordered]@{ name = $name; packageManager = 'winget'; _exist = [bool]$Version }
    if ($outside) { $state.managedBy = 'other'; $state.path = $outside }
    if ($report -and -not $outside) {
        if ($Version -and $Version -ne '?') { $state.version = $Version }
        $state.supportsVersionPin = $true
        $state.versionScheme = 'winget'
        $state.via = 'winget'
        if ($null -eq $script:latest) { $script:latest = Get-Latest }
        if ($script:latest) { $state.latestVersion = $script:latest }
        if ($report -eq 'all') { $state.availableVersions = @(Get-Available) }
    }
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
        'get' { Write-State '?' }
        'test' { Write-State '?' $null $want }
        'set' {
            $step = "$name is installed and up to date"
            if (-not $want) {
                $step = "$name was installed outside furnish ($outside): uninstall it the way it was installed"
                # Plain text and an unmapped exit code, so DSC keeps the line in its error.
                if ($Mode -ne 'what-if') { [Console]::Error.WriteLine("furnish: $step"); exit 14 }
            }
            Write-State '?' $(if ($Mode -eq 'what-if') { @($step) })
        }
        default { [Console]::Error.WriteLine('{"error":"usage: get|test|set [what-if]"}'); exit 10 }
    }
    exit 0
}
# Without reportVersions and targetVersion, as before: installed or not.
function Get-State { if ($report -or $target) { Get-Installed } elseif (Test-Installed) { '?' } else { '' } }
$latest = $null

switch ($Operation) {
    'get' { Write-State (Get-State) }
    'test' {
        $version = Get-State
        $ok = if ($env:_exist -eq 'false') { -not $version }
              elseif ($target) { $version -eq $target }
              else { $version -and -not (Test-Outdated) }
        Write-State $version $null $ok
    }
    'set' {
        $version = Get-State
        $installed = [bool]$version
        $want = $env:_exist -ne 'false'
        $wingetArgs = $null
        $after = if ($want) { '?' } else { '' }
        if ($want -and $target) {
            $after = $target
            if ($version -eq $target) { $step = "$name $target is installed, the required version" }
            elseif (-not $installed) { $step = "Install $name $target with winget"; $wingetArgs = $installArgs + @('--version', $target) }
            else {
                $step = if ($version -eq '?') { "Update $name to $target with winget" } else { "Update $name from $version to $target with winget" }
                $wingetArgs = $upgradeArgs + @('--version', $target)
            }
        }
        elseif ($want -and -not $installed) { $step = "Install $name with winget"; $wingetArgs = $installArgs }
        elseif ($want -and (Test-Outdated)) { $step = "Update $name with winget"; $wingetArgs = $upgradeArgs }
        elseif (-not $want -and $installed) { $step = "Remove $name with winget"; $wingetArgs = $removeArgs }
        elseif ($installed) { $step = "$name is installed and up to date" }
        else { $step = "$name is not installed" }

        if ($Mode -eq 'what-if') {
            $whatIf = @($step)
            if ($wingetArgs) { $whatIf += "Run: winget $($wingetArgs -join ' ')" }
            if ($want -and $version -and -not $target) { $after = $version }
            Write-State $after $whatIf
            exit 0
        }
        if ($wingetArgs) {
            [Console]::Error.WriteLine((@{ info = "$($step): running winget $($wingetArgs -join ' ')" } | ConvertTo-Json -Compress))
            $code = Invoke-Winget $wingetArgs
            if ($code -ne 0) { [Console]::Error.WriteLine("{`"error`":`"winget failed with exit code $code`"}"); exit $code }
        }
        Write-State (Get-State)
    }
    default { [Console]::Error.WriteLine('{"error":"usage: get|test|set [what-if]"}'); exit 10 }
}

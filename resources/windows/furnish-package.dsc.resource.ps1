# Furnish/Package on Windows: manage a package with winget.
# DSC passes the resource properties as env vars: $env:name (a winget package ID) and $env:_exist.
#
#   get                  print the current state
#   test                 current state with _inDesiredState (an available update: not in desired state)
#   set [what-if]        converge (install, update or remove); with what-if only describe the
#                        steps (in _metadata.whatIf) and print the state it would lead to
param([string]$Operation, [string]$Mode)

$name = $env:name
# Own exit codes start at 10; winget failures exit with winget's (unmapped) code so DSC shows stderr.
if (-not $name) { [Console]::Error.WriteLine('{"error":"property name is required"}'); exit 10 }
# SYSTEM (the furnish service) doesn't have winget on its PATH: use App Installer's copy.
if (-not (Get-Command winget -ErrorAction SilentlyContinue)) {
    $found = Get-ChildItem "$env:ProgramFiles\WindowsApps\Microsoft.DesktopAppInstaller_*_x64__8wekyb3d8bbwe\winget.exe" -ErrorAction SilentlyContinue |
        Sort-Object { [version]($_.Directory.Name -split '_')[1] } | Select-Object -Last 1
    if ($found) { Set-Alias winget $found.FullName }
}
if (-not (Get-Command winget -ErrorAction SilentlyContinue)) {
    # Plain text and an unmapped exit code: furnish shows "Needs winget" instead of a failure.
    [Console]::Error.WriteLine('furnish: needs winget (App Installer from the Microsoft Store)'); exit 11
}

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
function Write-State([bool]$Exist, [string[]]$WhatIf, $InDesiredState) {
    $state = [ordered]@{ name = $name; _exist = $Exist }
    if ($WhatIf) { $state._metadata = @{ whatIf = $WhatIf } }
    if ($null -ne $InDesiredState) { $state._inDesiredState = [bool]$InDesiredState }
    [Console]::Out.WriteLine(($state | ConvertTo-Json -Compress -Depth 4))
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

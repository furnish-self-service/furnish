# Furnish/Package on Windows: manage a package with winget.
# DSC passes the resource properties as env vars: $env:name (a winget package ID) and $env:_exist.
#
#   get                  print the current state
#   set [what-if]        converge to the desired state; with what-if only describe the
#                        steps (in _metadata.whatIf) and print the state it would lead to
param([string]$Operation, [string]$Mode)

$name = $env:name
# Own exit codes start at 10; winget failures exit with winget's (unmapped) code so DSC shows stderr.
if (-not $name) { [Console]::Error.WriteLine('{"error":"property name is required"}'); exit 10 }
if (-not (Get-Command winget -ErrorAction SilentlyContinue)) {
    [Console]::Error.WriteLine('{"error":"winget not found"}'); exit 11
}

$common = @('--exact', '--id', $name, '--accept-source-agreements', '--disable-interactivity')
$installArgs = @('install') + $common + @('--silent', '--accept-package-agreements')
$removeArgs = @('uninstall') + $common + @('--silent')

# winget output goes to stderr: DSC reads stdout as JSON.
function Invoke-Winget([string[]]$WingetArgs) {
    & winget @WingetArgs 2>&1 | ForEach-Object { [Console]::Error.WriteLine($_) }
    return $LASTEXITCODE
}
function Test-Installed { (Invoke-Winget (@('list') + $common)) -eq 0 }
function Write-State([bool]$Exist, [string[]]$WhatIf) {
    $state = [ordered]@{ name = $name; _exist = $Exist }
    if ($WhatIf) { $state._metadata = @{ whatIf = $WhatIf } }
    [Console]::Out.WriteLine(($state | ConvertTo-Json -Compress -Depth 4))
}

switch ($Operation) {
    'get' { Write-State (Test-Installed) }
    'set' {
        $installed = Test-Installed
        $want = $env:_exist -ne 'false'
        $wingetArgs = $null
        if ($want -and -not $installed) { $step = "Install $name with winget"; $wingetArgs = $installArgs }
        elseif (-not $want -and $installed) { $step = "Remove $name with winget"; $wingetArgs = $removeArgs }
        elseif ($installed) { $step = "$name is already installed" }
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
    default { [Console]::Error.WriteLine('{"error":"usage: get|set [what-if]"}'); exit 10 }
}

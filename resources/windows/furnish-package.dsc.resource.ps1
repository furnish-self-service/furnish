# Furnish/Package on Windows: manage a package with winget.
# DSC passes the resource properties as env vars: $env:name (a winget package ID) and $env:_exist.
param([string]$Operation)

$name = $env:name
# Own exit codes start at 10; winget failures exit with winget's (unmapped) code so DSC shows stderr.
if (-not $name) { [Console]::Error.WriteLine('{"error":"property name is required"}'); exit 10 }
if (-not (Get-Command winget -ErrorAction SilentlyContinue)) {
    [Console]::Error.WriteLine('{"error":"winget not found"}'); exit 11
}

# winget output goes to stderr: DSC reads stdout as JSON.
function Invoke-Winget {
    & winget @args --exact --id $name --accept-source-agreements --disable-interactivity 2>&1 |
        ForEach-Object { [Console]::Error.WriteLine($_) }
    return $LASTEXITCODE
}

switch ($Operation) {
    'get' {
        $exist = (Invoke-Winget list) -eq 0
        [Console]::Out.WriteLine((@{ name = $name; _exist = $exist } | ConvertTo-Json -Compress))
    }
    'set' {
        if ($env:_exist -eq 'false') {
            $code = Invoke-Winget uninstall --silent
        } else {
            $code = Invoke-Winget install --silent --accept-package-agreements
        }
        if ($code -ne 0) { [Console]::Error.WriteLine("{`"error`":`"winget failed with exit code $code`"}"); exit $code }
    }
    default { [Console]::Error.WriteLine('{"error":"usage: get|set"}'); exit 10 }
}

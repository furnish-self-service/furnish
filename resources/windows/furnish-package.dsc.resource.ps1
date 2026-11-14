# Furnish/Package on Windows: manage a package with winget.
# DSC passes the resource properties as env vars: $env:name (a winget package ID) and $env:_exist.
param([string]$Operation)

$name = $env:name
if (-not $name) { [Console]::Error.WriteLine("property 'name' is required"); exit 1 }
if (-not (Get-Command winget -ErrorAction SilentlyContinue)) {
    [Console]::Error.WriteLine('Furnish/Package: winget not found'); exit 2
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
        if ($code -ne 0) { [Console]::Error.WriteLine("winget failed with exit code $code"); exit 3 }
    }
    default { [Console]::Error.WriteLine('usage: get|set'); exit 1 }
}

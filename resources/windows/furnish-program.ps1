$command = $env:command
if ($command -and $command -cnotmatch '^[A-Za-z0-9._+][A-Za-z0-9._+-]*$') {
    [Console]::Error.WriteLine((@{ error = "command must be a program's name, without a folder: $command" } | ConvertTo-Json -Compress)); exit 10
}

# The program $command, where people's installers put programs: PATH, then the folders that
# aren't always on it (~\.local\bin for vendors' installers, npm's global folder). Only looks.
function Find-Program([string]$command) {
    $found = Get-Command $command -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($found) { return $found.Source }
    $candidates = @()
    if ($env:USERPROFILE) { $candidates += "$env:USERPROFILE\.local\bin\$command.exe" }
    if ($env:APPDATA) { $candidates += "$env:APPDATA\npm\$command.cmd" }
    return $candidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
}

function Complete-External([string]$subject) {
    $want = $env:_exist -ne 'false'
    switch ($Operation) {
        'get' { Write-State '?' }
        'test' { Write-State '?' $null $want }
        'set' {
            $step = "$subject was installed outside furnish ($outside)"
            if (-not $want) {
                $step = "$subject was installed outside furnish ($outside): uninstall it the way it was installed"
                # Plain text and an unmapped exit code, so DSC keeps the line in its error.
                if ($Mode -ne 'what-if') { [Console]::Error.WriteLine("furnish: $step"); exit 14 }
            }
            Write-State '?' $(if ($Mode -eq 'what-if') { @($step) })
        }
        default { [Console]::Error.WriteLine('{"error":"usage: get|test|set [what-if]"}'); exit 10 }
    }
    exit 0
}

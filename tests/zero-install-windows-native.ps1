# Verify the real client's My Apps registry independently from the Furnish resource.
param([Parameter(Mandatory)][string]$Furnish)
$ErrorActionPreference = 'Stop'
$Furnish = (Resolve-Path $Furnish).Path
$dir = New-Item -ItemType Directory (Join-Path $env:RUNNER_TEMP ([guid]::NewGuid()))
$archive = Join-Path $dir 'client.tar.gz'
$feed = 'https://apps.0install.net/utils/jq.xml'
$oldPath = $env:PATH
function RegistrationCount {
    [xml]$apps = (& 0install list-apps --batch --xml $feed) -join "`n"
    if ($LASTEXITCODE -ne 0) { throw 'My Apps query failed' }
    $apps.SelectNodes("/*[local-name()='app-list']/*[local-name()='app']").Count
}
function Apply([bool]$want) {
    $state = @{ packageManager = '0install'; name = 'jq'; feed = $feed; _exist = $want } |
        ConvertTo-Json -Compress | & $Furnish dsc-resource package set | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0 -or $state._exist -ne $want) { throw "Unexpected Furnish result for _exist=$want" }
}
try {
    Invoke-WebRequest 'https://github.com/0install/0install-win/releases/download/2.30.0/0install-win-2.30.0.tar.gz' -OutFile $archive
    if ((Get-FileHash $archive).Hash -ne '41e137d53a604748306e24547be6a4c8df02149e6f4d00371c6522dcc90acc46') { throw 'Unexpected client archive digest' }
    tar -xf $archive -C $dir.FullName
    if ($LASTEXITCODE -ne 0) { throw 'Extracting Zero Install failed' }
    $env:PATH = "$($dir.FullName);$oldPath"
    if ((RegistrationCount) -ne 0) { throw 'Fixture feed was already registered' }
    Apply $true
    if ((RegistrationCount) -ne 1) { throw 'Feed missing from My Apps' }
    Apply $true
    if ((RegistrationCount) -ne 1) { throw 'Repeat install changed registration count' }
    Apply $false
    if ((RegistrationCount) -ne 0) { throw 'Feed remains in My Apps' }
} finally {
    $env:PATH = $oldPath
    Remove-Item $dir.FullName -Recurse -Force
}

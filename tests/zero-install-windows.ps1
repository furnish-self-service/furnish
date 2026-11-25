# Exercise the resource entry point and the Windows Zero Install CLI contract offline.
param([Parameter(Mandatory)][string]$Furnish)
$ErrorActionPreference = 'Stop'
$Furnish = (Resolve-Path $Furnish).Path
$temp = Join-Path $env:RUNNER_TEMP ([guid]::NewGuid())
$oldPath = $env:PATH
try {
    New-Item -ItemType Directory $temp | Out-Null
    $env:PATH = "$temp;$oldPath"
    $env:ZERO_TEST_DIR = $temp
    @'
@echo off
setlocal
if "%*"=="list-apps --batch --xml https://example.test/hello.xml" (
    if exist "%ZERO_TEST_DIR%\registered" (
        echo ^<app-list^>^<app interface="https://example.test/hello.xml"/^>^</app-list^>
    ) else (
        echo ^<app-list/^>
    )
    exit /b 0
)
if "%*"=="download --batch https://example.test/hello.xml" (
    echo done>"%ZERO_TEST_DIR%\cached"
    exit /b 0
)
if "%*"=="integrate --batch --no-download --add-standard https://example.test/hello.xml" (
    if not exist "%ZERO_TEST_DIR%\cached" exit /b 42
    echo done>"%ZERO_TEST_DIR%\registered"
    exit /b 0
)
if "%*"=="remove --batch https://example.test/hello.xml" (
    del "%ZERO_TEST_DIR%\registered"
    exit /b 0
)
exit /b 99
'@ | Set-Content -Encoding ascii "$temp/0install.cmd"
    function Resource($Operation, $Want, $Preview = $false) {
        $arguments = @('dsc-resource', 'package', $Operation)
        if ($Preview) { $arguments += '--what-if' }
        $output = @{ packageManager = '0install'; name = 'hello'; feed = 'https://example.test/hello.xml'; _exist = $Want } |
            ConvertTo-Json -Compress | & $Furnish @arguments
        if ($LASTEXITCODE -ne 0) { throw "$Operation failed with $LASTEXITCODE" }
        $output | ConvertFrom-Json
    }
    if ((Resource 'test' $true)._inDesiredState) { throw 'Already installed' }
    $null = Resource 'set' $true $true
    if (Test-Path "$temp/cached") { throw 'Preview downloaded an app' }
    if (!(Resource 'set' $true)._exist) { throw 'Install failed' }
    if (!(Resource 'test' $true)._inDesiredState) { throw 'Registration missing' }
    if ((Resource 'set' $false)._exist) { throw 'Removal failed' }
    if (!(Test-Path "$temp/cached")) { throw 'Removal cleared the shared cache' }
} finally {
    $env:PATH = $oldPath
    Remove-Item Env:ZERO_TEST_DIR -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force $temp
}

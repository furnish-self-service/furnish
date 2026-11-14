# Register the furnish service (run as administrator, e.g. from Intune or a GPO startup script).
# It runs as SYSTEM and installs your organization's apps for users without administrator rights;
# configure it with the `service` section of the policy (HKLM\SOFTWARE\Policies\furnish\Policy).
param([string]$Furnish = "$env:ProgramFiles\furnish\furnish.exe")
$ErrorActionPreference = 'Stop'
if (-not (Test-Path $Furnish)) { throw "furnish.exe not found at $Furnish" }
if (Get-Service furnish -ErrorAction SilentlyContinue) {
    Stop-Service furnish
} else {
    New-Service -Name furnish -DisplayName 'furnish' -Description 'Installs apps your organization provides' `
        -BinaryPathName "`"$Furnish`" service" -StartupType Automatic | Out-Null
}
Start-Service furnish

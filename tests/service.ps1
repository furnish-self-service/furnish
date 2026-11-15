# The furnish Windows service: a standard user in the allowed group installs an .msi the
# organization provides; other users can't; users can't change the service's collections.
# Run as administrator on a throwaway machine (CI): creates users and a service.
#
#   tests/service.ps1 -Furnish target\release\furnish.exe -Msi target\furnish-demo.msi
param([Parameter(Mandatory)][string]$Furnish, [Parameter(Mandatory)][string]$Msi)
$ErrorActionPreference = 'Stop'
$pw = ConvertTo-SecureString 'Furnish-Test-1!' -AsPlainText -Force

function Fail([string]$Message) {
    Write-Host "FAIL: $Message"
    Get-ChildItem C:\ProgramData\furnish -Recurse -Depth 3 -ErrorAction SilentlyContinue | Select-Object -First 30 | ForEach-Object FullName
    # The service's copy of the collection, as git sees it.
    $org = 'C:\ProgramData\furnish\collections\org'
    if (Test-Path "$org\.git") {
        Write-Host '--- git config, sparse patterns, index (S: left out of the checkout)'
        git -c safe.directory=* -C $org config --local --list
        Get-Content "$org\.git\info\sparse-checkout" -ErrorAction SilentlyContinue
        git -c safe.directory=* -C $org ls-files -t | Select-Object -First 10
        git -c safe.directory=* -C $org status --short | Select-Object -First 10
    }
    Write-Host '--- git processes still running (the service waiting on one?)'
    Get-CimInstance Win32_Process -Filter "Name like 'git%' or Name = 'furnish.exe'" | ForEach-Object { "$($_.ProcessId) $($_.CommandLine)" }
    Get-WinEvent -LogName System -MaxEvents 20 -ErrorAction SilentlyContinue | Where-Object ProviderName -eq 'Service Control Manager' | Format-List TimeCreated, Message
    exit 1
}

# Run furnish as another user; output in $script:Out, returns the exit code.
function As([string]$User, [string]$Arguments) {
    $out = "C:\Users\Public\$User-out.txt"
    $batch = "C:\Users\Public\$User-run.cmd"
    Remove-Item $out -ErrorAction SilentlyContinue
    @(
        '@echo off',
        "set FURNISH_DATA_DIR=C:\Users\Public\$User-data",
        "`"$bin`" $Arguments > `"$out`" 2>&1",
        'exit /b %ERRORLEVEL%'
    ) | Set-Content $batch -Encoding ascii
    $p = Start-Process $batch -Credential (New-Object PSCredential($User, $pw)) -WorkingDirectory C:\Users\Public -Wait -PassThru
    $script:Out = Get-Content $out -Raw -ErrorAction SilentlyContinue
    Write-Host "+ ($User) furnish $Arguments -> $($p.ExitCode)`n$script:Out"
    if ($null -eq $script:Out) { Fail "furnish didn't run as $User" }
    return $p.ExitCode
}

# The demo .msi is 32-bit: it registers under WOW6432Node.
function Installed {
    [bool](Get-ChildItem HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall, HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall |
        Get-ItemProperty | Where-Object DisplayName -eq 'Furnish Demo')
}

New-LocalGroup furnish-users | Out-Null
foreach ($u in 'alice', 'bob') { New-LocalUser $u -Password $pw -PasswordNeverExpires | Out-Null; Add-LocalGroupMember Users $u }
Add-LocalGroupMember furnish-users alice

$dir = "$env:ProgramFiles\furnish"
New-Item -ItemType Directory $dir -Force | Out-Null
Copy-Item $Furnish "$dir\furnish.exe"
$bin = "$dir\furnish.exe"

# The organization's collection: one app, an .msi.
$repo = 'C:\srv\org'
New-Item -ItemType Directory "$repo\apps", "$repo\installers" -Force | Out-Null
Copy-Item $Msi "$repo\installers\furnish-demo.msi"
@"
metadata:
  furnish:
    description: Installer test
resources:
  - name: demo installer
    type: Furnish/Installer
    properties:
      file: installers/furnish-demo.msi
"@ | Set-Content "$repo\apps\demo.dsc.yaml"
git -C $repo init -q; git -C $repo add -A; git -C $repo -c user.name=t -c user.email=t@t commit -qm init
git config --system --add safe.directory C:/srv/org  # SYSTEM clones a repository the runner owns

$policy = @{ collections = @{ preset = @(@{ name = 'org'; url = 'C:/srv/org' }); usersCanAdd = $false }; service = @{ allowedGroup = 'furnish-users' } } | ConvertTo-Json -Depth 5
New-Item HKLM:\SOFTWARE\Policies\furnish -Force | Out-Null
Set-ItemProperty HKLM:\SOFTWARE\Policies\furnish Policy $policy

& $bin service install
if ($LASTEXITCODE -ne 0) { Fail 'service install failed' }
& $bin service install  # again: replaces the running service
if ($LASTEXITCODE -ne 0) { Fail 'reinstalling the service failed' }
for ($i = 0; $i -lt 100 -and -not (Test-Path \\.\pipe\furnish); $i++) { Start-Sleep -Milliseconds 300 }
if (-not (Test-Path \\.\pipe\furnish)) { Fail "the service didn't start" }
for ($i = 0; $i -lt 100 -and -not (Test-Path C:\ProgramData\furnish\collections\org\apps); $i++) { Start-Sleep -Milliseconds 300 }

Write-Host '== a member of the allowed group installs system-wide'
if ((As alice 'install demo') -ne 0) { Fail 'alice could not install' }
if (-not (Installed)) { Fail 'not installed' }

Write-Host '== others are refused'
if ((As bob 'uninstall demo') -eq 0) { Fail 'bob uninstalled' }
if ($script:Out -notmatch "aren't allowed") { Fail 'unexpected refusal' }
if (-not (Installed)) { Fail 'removed anyway' }

Write-Host "== users can't change what the service installs"
$evil = 'C:\ProgramData\furnish\collections\org\apps\evil.dsc.yaml'
Set-Content C:\Users\Public\evil.cmd "echo x > $evil" -Encoding ascii
Start-Process C:\Users\Public\evil.cmd -Credential (New-Object PSCredential('alice', $pw)) -WorkingDirectory C:\Users\Public -Wait | Out-Null
if (Test-Path $evil) { Fail 'alice wrote into the collection' }

Write-Host '== uninstall through the service'
if ((As alice 'uninstall demo') -ne 0) { Fail 'alice could not uninstall' }
if (Installed) { Fail 'still installed' }

& $bin service uninstall
if ($LASTEXITCODE -ne 0) { Fail 'service uninstall failed' }
if (Get-Service furnish -ErrorAction SilentlyContinue) { Fail 'service still registered' }
Write-Host 'service OK'

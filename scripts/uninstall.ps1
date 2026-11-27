<#
CSR uninstaller for Windows (beta). Run it in PowerShell started as Administrator.

For each network (testnet, mainnet) it removes the read-only entry the installer added on the CKB node's P2P key
file for NT SERVICE\CSR-<network> (nothing else in the node's folders is touched), stops and deletes the Windows
service CSR-<network>, unregisters a scheduled task CSR-<network> left by an older install (0.5.x/0.6.x), and
removes the firewall rule "CSR ping <network>". Then it removes the Start menu folder CSR and %ProgramFiles%\csr.
Easiest: double-click uninstall.cmd (it asks for Administrator rights); uninstall.cmd -RemoveData from a
command prompt also deletes the data. The data root %ProgramData%\csr
(configs, runner keys, wallet, logs) is kept unless -RemoveData is given; then you must type DELETE to confirm.
It never touches the CKB node.
#>
[CmdletBinding()]
param(
    [switch]$RemoveData
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Write-Host 'Run PowerShell as Administrator (or double-click uninstall.cmd, which asks for it)' -ForegroundColor Red
    exit 1
}

$ProgData = $env:ProgramData
if (-not $ProgData) { $ProgData = 'C:\ProgramData' }
$ProgFiles = $env:ProgramFiles
if (-not $ProgFiles) { $ProgFiles = 'C:\Program Files' }
$Data = Join-Path $ProgData 'csr'
$InstallDir = Join-Path $ProgFiles 'csr'
$Exe = Join-Path $InstallDir 'csr.exe'

function Remove-KeyEntry([string]$Net) {
    # Takes back the one read entry the installer added on the node's key file. The account's SID only
    # resolves while the service exists, so this runs before the service is deleted.
    $cfg = Join-Path $Data "csr-$Net.toml"
    if (-not (Test-Path -LiteralPath $cfg -PathType Leaf)) { return }
    $key = $null
    $cur = ''
    foreach ($line in (Get-Content -LiteralPath $cfg)) {
        if ($line -match '^\s*\[([^\]]+)\]') { $cur = $Matches[1]; continue }
        if ($cur -eq 'node' -and $line -match '^\s*p2p_key_file\s*=\s*"([^"]*)"') { $key = $Matches[1].Replace('\\', '\'); break }
    }
    if (-not $key -or -not (Test-Path -LiteralPath $key -PathType Leaf)) { return }
    try {
        $sid = (New-Object Security.Principal.NTAccount("NT SERVICE\CSR-$Net")).Translate([Security.Principal.SecurityIdentifier]).Value
    } catch {
        Write-Host "  could not resolve NT SERVICE\CSR-$Net; the key file $key was left as it is"
        return
    }
    $null = & icacls.exe $key /remove "*$sid"
    if ($LASTEXITCODE -eq 0) { Write-Host "  removed read access of NT SERVICE\CSR-$Net from $key" }
    else { Write-Host "  could not remove the entry of NT SERVICE\CSR-$Net from $key" }
}

function Remove-Service([string]$Name) {
    $svc = Get-Service -Name $Name -ErrorAction SilentlyContinue
    if (-not $svc) { Write-Host "  service $Name not found"; return }
    if ($svc.Status -ne 'Stopped') {
        Stop-Service -Name $Name -Force -ErrorAction SilentlyContinue
        try { $svc.WaitForStatus('Stopped', [TimeSpan]::FromSeconds(30)) } catch { }
    }
    $null = & sc.exe delete $Name
    if ($LASTEXITCODE -eq 0) { Write-Host "  removed service $Name" }
    else { Write-Host "  could not delete service $Name (exit code $LASTEXITCODE)" }
}

foreach ($net in @('testnet', 'mainnet')) {
    $name = "CSR-$net"
    Remove-KeyEntry $net
    Remove-Service $name
    if (Get-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue) {
        Stop-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue
        Unregister-ScheduledTask -TaskName $name -Confirm:$false
        Write-Host "  removed old scheduled task $name"
    }
    $rule = Get-NetFirewallRule -DisplayName "CSR ping $net" -ErrorAction SilentlyContinue
    if ($rule) {
        $rule | Remove-NetFirewallRule
        Write-Host "  removed firewall rule 'CSR ping $net'"
    }
}

Get-Process -Name csr -ErrorAction SilentlyContinue |
    Where-Object { $_.Path -and ($_.Path -eq $Exe) } |
    ForEach-Object { $_.Kill(); [void]$_.WaitForExit(10000) }

$menu = Join-Path $ProgData 'Microsoft\Windows\Start Menu\Programs\CSR'
if (Test-Path -LiteralPath $menu) {
    Remove-Item -LiteralPath $menu -Recurse -Force
    Write-Host '  removed the Start menu folder CSR'
}

if (Test-Path -LiteralPath $InstallDir) {
    # Windows cannot delete the folder the shell is in (when run as & '...\csr\uninstall.ps1' from there)
    if ((Get-Location).Path -like "$InstallDir*") { Set-Location -LiteralPath $ProgData }
    Remove-Item -LiteralPath $InstallDir -Recurse -Force
    Write-Host "  removed $InstallDir"
} else {
    Write-Host "  $InstallDir not found"
}

if ($RemoveData) {
    if (Test-Path -LiteralPath $Data) {
        $answer = Read-Host -Prompt 'Type DELETE to remove all CSR data, including runner keys'
        if ($answer -ceq 'DELETE') {
            Remove-Item -LiteralPath $Data -Recurse -Force
            Write-Host "  removed $Data"
        } else {
            Write-Host "  not confirmed: $Data was kept"
        }
    } else {
        Write-Host "  $Data not found"
    }
} else {
    Write-Host "  kept the data (configs, keys, logs) in $Data; use -RemoveData to delete it"
}
Write-Host 'CSR (Windows beta) is uninstalled.'

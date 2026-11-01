<#
CSR uninstaller for Windows (beta). Run it in PowerShell started as Administrator.

It stops and unregisters the scheduled tasks CSR-testnet and CSR-mainnet, removes the firewall rules
"CSR ping testnet" / "CSR ping mainnet" and removes %ProgramFiles%\csr. The data root %ProgramData%\csr
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
    Write-Host 'Run PowerShell as Administrator' -ForegroundColor Red
    exit 1
}

$ProgData = $env:ProgramData
if (-not $ProgData) { $ProgData = 'C:\ProgramData' }
$ProgFiles = $env:ProgramFiles
if (-not $ProgFiles) { $ProgFiles = 'C:\Program Files' }
$Data = Join-Path $ProgData 'csr'
$InstallDir = Join-Path $ProgFiles 'csr'
$Exe = Join-Path $InstallDir 'csr.exe'

foreach ($net in @('testnet', 'mainnet')) {
    $name = "CSR-$net"
    if (Get-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue) {
        Stop-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue
        Unregister-ScheduledTask -TaskName $name -Confirm:$false
        Write-Host "  removed task $name"
    } else {
        Write-Host "  task $name not found"
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

if (Test-Path -LiteralPath $InstallDir) {
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

<#
CSR installer for Windows 10/11 x86_64 (PowerShell 5.1 or later). Windows support is a BETA: x86_64 only,
no code signing, no auto-update.

It finds the CKB full nodes on this PC (ckb.exe, or Neuron's internal MAINNET full node) and installs one CSR
instance per network, like scripts/install.sh does on Linux:
  testnet -> task CSR-testnet, %ProgramData%\csr\csr-testnet.toml, dashboard 8130, ping 8131
  mainnet -> task CSR-mainnet, %ProgramData%\csr\csr-mainnet.toml, dashboard 8140, ping 8141
On mainnet CSR is a node dashboard only: the Service Runtime stays off until Phase 2.
Neuron's testnet mode is a light client and is not supported.

Run it from the unpacked zip, in PowerShell started as Administrator:
  powershell -ExecutionPolicy Bypass -File .\install.ps1

What it does: copies csr.exe to %ProgramFiles%\csr, creates the data root %ProgramData%\csr, writes
csr-<network>.toml per node (only if it does not exist yet), links the node identity (the node's own
secret_key), adds a firewall rule for the ping port only, registers a scheduled task per network (at startup,
as SYSTEM, restarts on failure) and starts it, and asks for a dashboard password. It never changes ckb.toml or
the CKB node's data.

Updating: run the installer of the new zip again. It stops the tasks, replaces csr.exe, keeps configs, keys and
data, and starts the tasks again.

Parameters:
  -Rpc <url[]>      node RPC URL(s) (loopback http only); may list a testnet and a mainnet node
  -CkbDir <dir[]>   folder(s) that contain a ckb.toml
  -NoRanking        write [ranking] join = false
  -Source <dir>     the unpacked zip (default: the folder of this script)
#>
[CmdletBinding()]
param(
    [string[]]$Rpc = @(),
    [string[]]$CkbDir = @(),
    [switch]$NoRanking,
    [string]$Source = ''
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# ---------------------------------------------------------------- admin check
$principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Write-Host 'Run PowerShell as Administrator' -ForegroundColor Red
    exit 1
}

if (-not $Source) { $Source = $PSScriptRoot }
$ProgData = $env:ProgramData
if (-not $ProgData) { $ProgData = 'C:\ProgramData' }
$ProgFiles = $env:ProgramFiles
if (-not $ProgFiles) { $ProgFiles = 'C:\Program Files' }
$Data = Join-Path $ProgData 'csr'
$InstallDir = Join-Path $ProgFiles 'csr'
$Exe = Join-Path $InstallDir 'csr.exe'
$Example = Join-Path $InstallDir 'csr.example.toml'

# ---------------------------------------------------------------- output helpers
function Write-Step([string]$Text) { Write-Host ''; Write-Host "==> $Text" -ForegroundColor Cyan }
function Write-Ok([string]$Text) { Write-Host "  OK $Text" -ForegroundColor Green }
function Write-Warn([string]$Text) { Write-Host "  ! $Text" -ForegroundColor Yellow }
function Stop-Install([string]$Text) { Write-Host ''; Write-Host "Error: $Text" -ForegroundColor Red; exit 1 }

# ---------------------------------------------------------------- helpers
function Get-ChainNetwork([string]$Chain) {
    switch ($Chain) {
        'ckb' { return 'mainnet' }
        'ckb_testnet' { return 'testnet' }
        default { return $null }
    }
}

function Get-InstancePorts([string]$Net) {
    if ($Net -eq 'mainnet') { return @(8140, 8141) }
    return @(8130, 8131)
}

function Test-LoopbackUrl([string]$Url) {
    return ($Url -match '^http://(127\.0\.0\.1|localhost|\[::1\]):\d+')
}

function Invoke-NodeRpc([string]$Url, [string]$Method) {
    # Returns the "result" of a JSON-RPC call, or $null on no answer or a JSON-RPC error
    # (a disabled module still answers HTTP 200, with an "error").
    try {
        $body = '{"id":1,"jsonrpc":"2.0","method":"' + $Method + '","params":[]}'
        $resp = Invoke-RestMethod -Uri $Url -Method Post -ContentType 'application/json' -Body $body -TimeoutSec 5 -UseBasicParsing
        if ($null -eq $resp) { return $null }
        if ($resp.PSObject.Properties.Name -contains 'error' -and $null -ne $resp.error) { return $null }
        if ($resp.PSObject.Properties.Name -contains 'result') {
            if ($null -eq $resp.result) { return $true }
            return $resp.result
        }
        return $null
    } catch {
        return $null
    }
}

function Get-RpcUrlFromCkbToml([string]$Path) {
    # The node's RPC URL from [rpc] listen_address, if it is reachable on loopback; else $null.
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return $null }
    $section = ''
    foreach ($line in (Get-Content -LiteralPath $Path -ErrorAction SilentlyContinue)) {
        if ($line -match '^\s*\[') {
            $section = ($line -replace '^\s*', '' -replace '\s*(#.*)?$', '')
            continue
        }
        if ($section -eq '[rpc]' -and $line -match '^\s*listen_address\s*=\s*"([^"]*)"') {
            $addr = $Matches[1]
            $i = $addr.LastIndexOf(':')
            if ($i -lt 0) { return $null }
            $hostPart = $addr.Substring(0, $i)
            $port = $addr.Substring($i + 1)
            if (@('127.0.0.1', '0.0.0.0', 'localhost') -contains $hostPart) { return "http://127.0.0.1:$port" }
            return $null
        }
    }
    return $null
}

function Find-NodeKeys {
    # Candidate P2P key files: <dir>\data\network\secret_key under -CkbDir, the user profile, Neuron and LocalAppData.
    $roots = @()
    $roots += $CkbDir
    foreach ($v in @($env:USERPROFILE, (Join-Path $env:APPDATA 'Neuron'), $env:LOCALAPPDATA)) {
        if ($v) { $roots += $v }
    }
    $found = @()
    foreach ($r in $roots) {
        if (-not $r -or -not (Test-Path -LiteralPath $r)) { continue }
        $direct = Join-Path $r 'data\network\secret_key'
        if (Test-Path -LiteralPath $direct -PathType Leaf) { $found += $direct }
        $hits = Get-ChildItem -LiteralPath $r -Filter 'secret_key' -File -Recurse -Depth 6 -Force -ErrorAction SilentlyContinue
        foreach ($h in $hits) {
            if ($h.Directory.Name -eq 'network' -and $h.Directory.Parent.Name -eq 'data') { $found += $h.FullName }
        }
    }
    return @($found | Select-Object -Unique)
}

function Get-KeyNodeDir([string]$Key) {
    # <dir>\data\network\secret_key -> <dir>
    return (Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $Key)))
}

function Get-ConfiguredRpcUrls {
    $urls = @()
    if (Test-Path -LiteralPath $Data) {
        foreach ($f in (Get-ChildItem -LiteralPath $Data -Filter 'csr-*.toml' -File -ErrorAction SilentlyContinue)) {
            foreach ($line in (Get-Content -LiteralPath $f.FullName)) {
                if ($line -match '^rpc_url\s*=\s*"([^"]+)"') {
                    if (Test-LoopbackUrl $Matches[1]) { $urls += $Matches[1] }
                    break
                }
            }
        }
    }
    return $urls
}

function Find-Nodes {
    # Returns objects with Network and Url (first node per network wins); empty array when none answers.
    $cands = @()
    if ($Rpc.Count -gt 0) {
        $cands += $Rpc
    } else {
        $cands += 'http://127.0.0.1:8114'
        $cands += Get-ConfiguredRpcUrls
        foreach ($d in $CkbDir) {
            $u = Get-RpcUrlFromCkbToml (Join-Path $d 'ckb.toml')
            if ($u) { $cands += $u }
        }
        foreach ($k in (Find-NodeKeys)) {
            $u = Get-RpcUrlFromCkbToml (Join-Path (Get-KeyNodeDir $k) 'ckb.toml')
            if ($u) { $cands += $u }
        }
    }
    $nodes = @()
    $seen = @{}
    $have = @{}
    foreach ($u in $cands) {
        if ($seen.ContainsKey($u)) { continue }
        $seen[$u] = $true
        $info = Invoke-NodeRpc $u 'get_blockchain_info'
        if ($null -eq $info -or $info -eq $true) { continue }
        if (-not ($info.PSObject.Properties.Name -contains 'chain')) { continue }
        $net = Get-ChainNetwork ([string]$info.chain)
        if (-not $net -or $have.ContainsKey($net)) { continue }
        $have[$net] = $true
        if ($info.PSObject.Properties.Name -contains 'is_initial_block_download' -and $info.is_initial_block_download) {
            Write-Warn "the $net node at $u is still syncing (initial block download); CSR starts anyway and works once it is synced"
        }
        $nodes += [pscustomobject]@{ Network = $net; Url = $u }
    }
    return @($nodes | Sort-Object { if ($_.Network -eq 'testnet') { 0 } else { 1 } })
}

function Get-NodeCkbToml([string]$Url) {
    # The ckb.toml whose [rpc] address serves $Url, from -CkbDir and the key folders; $null if unknown.
    $dirs = @()
    $dirs += $CkbDir
    foreach ($k in (Find-NodeKeys)) { $dirs += (Get-KeyNodeDir $k) }
    foreach ($d in $dirs) {
        $t = Join-Path $d 'ckb.toml'
        if ((Get-RpcUrlFromCkbToml $t) -eq $Url) { return $t }
    }
    return $null
}

function Test-NodeModules([string]$Net, [string]$Url) {
    Write-Step "Checking the $Net node at $Url"
    if ($null -eq (Invoke-NodeRpc $Url 'get_indexer_tip')) {
        Stop-Install "the $Net node's Indexer RPC module is off. In ckb.toml, under [rpc], add `"Indexer`" to modules (and keep `"Net`"), restart the node and run this script again."
    }
    Write-Ok 'Indexer module enabled'
    if ($null -eq (Invoke-NodeRpc $Url 'local_node_info')) {
        Write-Host "  The $Net node's Net RPC module is off. CSR needs it (node identity, NAT help)." -ForegroundColor Red
        $toml = Get-NodeCkbToml $Url
        if ($toml) {
            Write-Host "  Add `"Net`" to modules under [rpc] in: $toml"
            $neuron = Join-Path $env:APPDATA 'Neuron'
            if ($toml.StartsWith($neuron, [StringComparison]::OrdinalIgnoreCase)) {
                Write-Host '  Neuron may overwrite this file when it restarts its node; see docs/windows.md.'
            }
        } else {
            Write-Host '  Add "Net" to modules under [rpc] in the ckb.toml of that node (next to its data folder).'
            Write-Host '  If it is a Neuron node, Neuron may overwrite that file when it restarts its node; see docs/windows.md.'
        }
        Write-Host '  Then restart the node and run this script again. ckb.toml was not changed.'
        exit 1
    }
    Write-Ok 'Net module enabled'
}

# ---------------------------------------------------------------- config editing
function Read-Lines([string]$Path) { return @([System.IO.File]::ReadAllLines($Path)) }
function Save-Lines([string]$Path, [string[]]$Lines) {
    [System.IO.File]::WriteAllLines($Path, $Lines, (New-Object System.Text.UTF8Encoding($false)))
}

function Set-TomlValue([string[]]$Lines, [string]$Section, [string]$Key, [string]$Value, [switch]$MatchCommented) {
    # Replaces `Key = ...` inside [Section] (or adds it at the end of the section); returns the new lines.
    $out = New-Object System.Collections.Generic.List[string]
    $cur = ''
    $done = $false
    $lastInSection = -1
    $pat = '^\s*' + $(if ($MatchCommented) { '#?\s*' } else { '' }) + [regex]::Escape($Key) + '\s*='
    foreach ($line in $Lines) {
        if ($line -match '^\s*\[([^\]]+)\]') { $cur = $Matches[1] }
        if (-not $done -and $cur -eq $Section -and $line -match $pat) {
            $out.Add("$Key = $Value")
            $done = $true
            $lastInSection = $out.Count - 1
            continue
        }
        if ($cur -eq $Section -and $line.Trim() -ne '') { $lastInSection = $out.Count + 0 }
        $out.Add($line)
    }
    if (-not $done) {
        if ($lastInSection -ge 0) { $out.Insert($lastInSection + 1, "$Key = $Value") }
        else { $out.Add("[$Section]"); $out.Add("$Key = $Value") }
    }
    return $out.ToArray()
}

function Get-TomlValue([string]$Path, [string]$Section, [string]$Key) {
    $cur = ''
    foreach ($line in (Read-Lines $Path)) {
        if ($line -match '^\s*\[([^\]]+)\]') { $cur = $Matches[1]; continue }
        if ($cur -eq $Section -and $line -match ('^\s*' + [regex]::Escape($Key) + '\s*=\s*"([^"]*)"')) { return $Matches[1] }
    }
    return $null
}

function Get-ConfigPath([string]$Net) { return (Join-Path $Data "csr-$Net.toml") }

function Write-InstanceConfig([string]$Net, [string]$Url) {
    $f = Get-ConfigPath $Net
    if (Test-Path -LiteralPath $f) {
        Save-Lines $f (Set-TomlValue (Read-Lines $f) 'node' 'rpc_url' "`"$Url`"")
        Write-Ok "keeping your existing $f (rpc_url = $Url)"
        return
    }
    $ports = Get-InstancePorts $Net
    $dash = $ports[0]; $ping = $ports[1]
    $join = 'true'
    if ($NoRanking) { $join = 'false' }
    $lines = Read-Lines $Example
    # `network` is a top-level key (before the first section)
    for ($i = 0; $i -lt $lines.Count; $i++) {
        if ($lines[$i] -match '^\s*\[') { break }
        if ($lines[$i] -match '^network\s*=') { $lines[$i] = "network = `"$Net`"" }
    }
    $lines = Set-TomlValue $lines 'node' 'rpc_url' "`"$Url`""
    $lines = Set-TomlValue $lines 'dashboard' 'bind' "`"127.0.0.1:$dash`""
    $lines = Set-TomlValue $lines 'ranking' 'join' $join
    $lines = Set-TomlValue $lines 'ranking' 'ping_bind' "`"0.0.0.0:$ping`""
    $lines = Set-TomlValue $lines 'ranking' 'ping_mode' '"outbound"'
    Save-Lines $f $lines
    Write-Ok "wrote $f ($Net, dashboard $dash, ping $ping, ping_mode = `"outbound`", ranking join = $join)"
}

function Set-PeerUrl([string]$Net, [string]$Url) {
    $f = Get-ConfigPath $Net
    $lines = Read-Lines $f
    # exactly one peer_url line in [dashboard]: drop duplicates first
    $out = New-Object System.Collections.Generic.List[string]
    $cur = ''; $seenPeer = $false
    foreach ($line in $lines) {
        if ($line -match '^\s*\[([^\]]+)\]') { $cur = $Matches[1] }
        if ($cur -eq 'dashboard' -and $line -match '^\s*peer_url\s*=') {
            if ($seenPeer) { continue }
            $seenPeer = $true
        }
        $out.Add($line)
    }
    Save-Lines $f (Set-TomlValue $out.ToArray() 'dashboard' 'peer_url' "`"$Url`"")
}

function Get-DashboardPort([string]$Net) {
    $b = Get-TomlValue (Get-ConfigPath $Net) 'dashboard' 'bind'
    if ($b -and $b -match ':(\d+)$') { return $Matches[1] }
    return [string](Get-InstancePorts $Net)[0]
}

# ---------------------------------------------------------------- csr.exe
function Invoke-Csr([string]$Net, [string[]]$CsrArgs, [string]$StdIn = $null) {
    # Runs csr.exe for one instance; returns @{ Code; Output }. Never throws on a non-zero exit.
    $old = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $all = @('--data-root', $Data, '--config', (Get-ConfigPath $Net)) + $CsrArgs
        if ($null -ne $StdIn) { $text = $StdIn | & $Exe @all 2>&1 }
        else { $text = & $Exe @all 2>&1 }
        $code = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $old
    }
    $joined = (@($text) | ForEach-Object { "$_" }) -join "`n"
    return @{ Code = $code; Output = $joined }
}

function Get-IdentityState([string]$Net) {
    $r = Invoke-Csr $Net @('check')   # `csr check` exits 2 while the node is not synced: still read its output
    foreach ($l in ($r.Output -split "`n")) {
        if ($l -match '^node identity:\s*(\S+)') { return $Matches[1] }
    }
    return ''
}

function Install-Files {
    Write-Step 'Installing CSR'
    foreach ($n in @('csr.exe', 'csr.example.toml')) {
        if (-not (Test-Path -LiteralPath (Join-Path $Source $n))) {
            Stop-Install "$n not found in $Source. Run install.ps1 from the unpacked zip, or pass -Source <folder>."
        }
    }
    if (-not (Test-Path -LiteralPath (Join-Path $Source 'networks'))) {
        Stop-Install "the folder 'networks' is missing in $Source (unpack the whole zip)."
    }
    foreach ($net in @('testnet', 'mainnet')) {
        $t = Get-ScheduledTask -TaskName "CSR-$net" -ErrorAction SilentlyContinue
        if ($t) {
            Stop-ScheduledTask -TaskName "CSR-$net" -ErrorAction SilentlyContinue
        }
    }
    # a running csr.exe would keep the file locked
    Get-Process -Name csr -ErrorAction SilentlyContinue |
        Where-Object { $_.Path -and ($_.Path -eq $Exe) } |
        ForEach-Object { $_.Kill(); [void]$_.WaitForExit(10000) }
    $old = ''
    if (Test-Path -LiteralPath $Exe) {
        $o = Invoke-Csr 'testnet' @('version')
        $old = $o.Output.Trim()
    }
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    Copy-Item -LiteralPath (Join-Path $Source 'csr.exe') -Destination $Exe -Force
    Copy-Item -LiteralPath (Join-Path $Source 'csr.example.toml') -Destination $Example -Force
    $netDest = Join-Path $InstallDir 'networks'
    New-Item -ItemType Directory -Force -Path $netDest | Out-Null
    Copy-Item -Path (Join-Path (Join-Path $Source 'networks') '*') -Destination $netDest -Recurse -Force
    New-Item -ItemType Directory -Force -Path $Data | Out-Null
    $v = (& $Exe version 2>&1 | Out-String).Trim()
    $was = ''
    if ($old) { $was = " (was: $old)" }
    Write-Ok "installed $v$was in $InstallDir"
    Write-Ok "data root: $Data"
}

function Restrict-DataRoot {
    # Only SYSTEM and Administrators may read the data root (runner keys, wallet, password hash).
    $null = & icacls.exe $Data /inheritance:r /grant:r '*S-1-5-18:(OI)(CI)F' '*S-1-5-32-544:(OI)(CI)F' 2>&1
    if ($LASTEXITCODE -eq 0) { Write-Ok 'data root readable only by SYSTEM and Administrators' }
    else { Write-Warn "could not restrict the permissions of $Data" }
}

# ---------------------------------------------------------------- identity
function Set-NodeKey([string]$Net, [string]$Key) {
    $f = Get-ConfigPath $Net
    $esc = $Key.Replace('\', '\\')
    Save-Lines $f (Set-TomlValue (Read-Lines $f) 'node' 'p2p_key_file' "`"$esc`"" -MatchCommented)
}

function Clear-NodeKey([string]$Net) {
    $f = Get-ConfigPath $Net
    $lines = Read-Lines $f | ForEach-Object { if ($_ -match '^p2p_key_file\s*=') { '# p2p_key_file = ""' } else { $_ } }
    Save-Lines $f @($lines)
}

function Set-Identity([string]$Net, [string]$Url) {
    Write-Step "Node identity of the $Net node (lets the CSR monitor rank it)"
    $configured = Get-TomlValue (Get-ConfigPath $Net) 'node' 'p2p_key_file'
    $cands = @()
    if ($configured) {
        $cands = @($configured.Replace('\\', '\'))
    } else {
        $first = @(); $rest = @()
        foreach ($k in (Find-NodeKeys)) {
            $u = Get-RpcUrlFromCkbToml (Join-Path (Get-KeyNodeDir $k) 'ckb.toml')
            if ($u -and $u -eq $Url) { $first += $k } else { $rest += $k }
        }
        if ($first.Count -gt 0) { $cands = $first } else { $cands = $rest }
    }
    if ($cands.Count -eq 0) {
        Write-Warn "no CKB P2P key found. Set node.p2p_key_file in $(Get-ConfigPath $Net) to <ckb data dir>\network\secret_key"
        return
    }
    foreach ($k in $cands) {
        if (-not (Test-Path -LiteralPath $k -PathType Leaf)) { Write-Warn "$k : not a file"; continue }
        Set-NodeKey $Net $k
        $state = Get-IdentityState $Net
        switch ($state) {
            'proven' { Write-Ok "node identity: proven ($k)"; return }
            'mismatch' { Clear-NodeKey $Net; Write-Warn "$k belongs to another node (mismatch), skipped" }
            'unreadable' { Clear-NodeKey $Net; Write-Warn "$k cannot be read (unreadable), skipped" }
            default {
                Write-Warn "node identity '$state' for $k : kept; CSR proves it once the node answers"
                return
            }
        }
    }
    Clear-NodeKey $Net
    Write-Warn 'node identity not proven; the node is not ranked. Fix later: see docs/windows.md'
}

# ---------------------------------------------------------------- firewall, task
function Set-PingFirewallRule([string]$Net) {
    $ping = (Get-InstancePorts $Net)[1]
    $name = "CSR ping $Net"
    Get-NetFirewallRule -DisplayName $name -ErrorAction SilentlyContinue | Remove-NetFirewallRule
    New-NetFirewallRule -DisplayName $name -Direction Inbound -Action Allow -Protocol TCP -LocalPort $ping -Profile Private,Domain | Out-Null
    Write-Ok "firewall: inbound TCP $ping allowed (Private, Domain) as '$name'; the dashboard port stays closed"
}

function Register-CsrTask([string]$Net) {
    $name = "CSR-$Net"
    $cfg = Get-ConfigPath $Net
    $argLine = '--data-root "' + $Data + '" --config "' + $cfg + '" run'
    $action = New-ScheduledTaskAction -Execute $Exe -Argument $argLine
    $trigger = New-ScheduledTaskTrigger -AtStartup
    $principal = New-ScheduledTaskPrincipal -UserId 'SYSTEM' -LogonType ServiceAccount -RunLevel Highest
    $settings = New-ScheduledTaskSettingsSet -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1) `
        -ExecutionTimeLimit ([TimeSpan]::Zero) -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
    if (Get-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue) {
        Stop-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue
        Unregister-ScheduledTask -TaskName $name -Confirm:$false
    }
    Register-ScheduledTask -TaskName $name -Action $action -Trigger $trigger -Principal $principal -Settings $settings `
        -Description 'CSR (CKB Service Runner), Windows beta' | Out-Null
    Start-ScheduledTask -TaskName $name
    Write-Ok "task $name registered (at startup, as SYSTEM) and started"
}

function Set-DashboardPassword([string]$Net) {
    $f = Get-ConfigPath $Net
    if (Get-TomlValue $f 'dashboard' 'password_hash') { Write-Ok "$Net dashboard password already set"; return }
    Write-Step "Dashboard password ($Net)"
    Write-Host '  Without a password the dashboard is read-only (no updates, wallet, registration or settings).'
    if (-not [Environment]::UserInteractive) { Write-Warn 'not interactive: no password set'; return }
    $sec = Read-Host -Prompt '  Dashboard password (empty = skip)' -AsSecureString
    if ($sec.Length -eq 0) { Write-Warn "no password set. Later: & '$Exe' --data-root '$Data' --config '$f' set-password"; return }
    $bstr = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($sec)
    try { $plain = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($bstr) }
    finally { [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($bstr) }
    $r = Invoke-Csr $Net @('set-password', '--password-stdin') $plain
    $plain = $null
    if ($r.Code -eq 0) {
        Write-Ok 'password set'
        Restart-ScheduledTaskSafe "CSR-$Net"
    } else {
        Write-Warn "password not set: $($r.Output)"
    }
}

function Restart-ScheduledTaskSafe([string]$Name) {
    Stop-ScheduledTask -TaskName $Name -ErrorAction SilentlyContinue
    Start-ScheduledTask -TaskName $Name
}

# ---------------------------------------------------------------- summary
function Show-Summary($Nodes) {
    Write-Host ''
    Write-Host 'CSR is installed (Windows beta).' -ForegroundColor Green
    foreach ($n in $Nodes) {
        $net = $n.Network
        $id = Get-IdentityState $net
        if (-not $id) { $id = 'unknown' }
        Write-Host ''
        Write-Host "  $net" -ForegroundColor Cyan
        Write-Host "  Dashboard   http://127.0.0.1:$(Get-DashboardPort $net)  (on this PC)"
        Write-Host "  Task        CSR-$net (Task Scheduler; starts with Windows)"
        Write-Host "  Identity    $id"
        Write-Host "  Logs        $(Join-Path (Join-Path $Data $net) 'logs')"
        Write-Host "  Config      $(Get-ConfigPath $net)"
        if ($net -eq 'mainnet') { Write-Host '  Runtime     inactive on mainnet until Phase 2 (node dashboard and ranking only)' }
    }
    Write-Host ''
    Write-Host '  Updates     there is no auto-update on Windows (beta): download the new zip and run install.ps1 again.'
    Write-Host '              It keeps your configs, keys and data.'
    Write-Host '  Uninstall   scripts\uninstall.ps1 (add -RemoveData to delete the data as well)'
    Write-Host ''
}

# ---------------------------------------------------------------- main
function Install-Csr {
    Write-Step 'Looking for CKB full nodes on this PC'
    $nodes = @(Find-Nodes)
    if ($nodes.Count -eq 0) {
        Write-Host ''
        Write-Host 'Error: no CKB node answers on this PC.' -ForegroundColor Red
        Write-Host '       CSR must run on the SAME PC as your CKB full node (it only talks to a local node and reads'
        Write-Host "       the node's key file). If your node runs elsewhere, run this script on that PC."
        Write-Host '       If the node is on this PC: start it, or pass -Rpc http://127.0.0.1:<port> (its RPC port),'
        Write-Host '       or -CkbDir <folder with ckb.toml>.'
        Write-Host "       Neuron users: only Neuron's internal mainnet full node works; its testnet mode is a light client."
        exit 1
    }
    foreach ($n in $nodes) { Write-Ok "$($n.Network) node at $($n.Url)" }
    foreach ($n in $nodes) { Test-NodeModules $n.Network $n.Url }

    Install-Files
    Restrict-DataRoot

    foreach ($n in $nodes) {
        $net = $n.Network
        Write-Step "Setting up CSR for $net"
        Write-InstanceConfig $net $n.Url
        Set-Identity $net $n.Url
        if ($net -eq 'mainnet') {
            Write-Warn 'mainnet: dashboard only, the Service Runtime stays off until Phase 2'
            $c = Invoke-Csr $net @('confirm-mainnet')
            if ($c.Code -ne 0) { Stop-Install "could not confirm mainnet: $($c.Output)" }
            Write-Ok 'mainnet confirmed'
        }
        $v = Invoke-Csr $net @('config', 'validate')
        if ($v.Code -ne 0) { Stop-Install "the configuration is invalid: $($v.Output)" }
        Write-Ok 'configuration valid'
        Set-PingFirewallRule $net
    }
    if ($nodes.Count -eq 2) {
        Write-Step 'Linking the two dashboards'
        Set-PeerUrl 'testnet' 'http://127.0.0.1:8140'
        Set-PeerUrl 'mainnet' 'http://127.0.0.1:8130'
        Write-Ok 'each dashboard links to the other'
    }
    foreach ($n in $nodes) {
        Write-Step "Starting CSR-$($n.Network)"
        Register-CsrTask $n.Network
    }
    foreach ($n in $nodes) { Set-DashboardPassword $n.Network }
    Start-Sleep -Seconds 3
    Show-Summary $nodes
}

Install-Csr

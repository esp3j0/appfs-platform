param(
    [string]$AppHost = "http://127.0.0.1:8999",
    [string]$ListenAddress = "",
    [int]$ListenPort = 0,
    [string]$FirewallRulePrefix = "Huoyan Storage PortProxy",
    [int]$PollIntervalSec = 3,
    [switch]$Watch,
    [switch]$Cleanup
)

$ErrorActionPreference = "Stop"

function Write-Info([string]$Message) {
    Write-Host "[info] $Message"
}

function Assert-Admin {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($identity)
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw "Please run this script in an elevated PowerShell session (Run as Administrator)."
    }
}

function Resolve-ListenAddress {
    param([string]$ConfiguredAddress)

    if ($ConfiguredAddress.Trim() -ne "") {
        return $ConfiguredAddress.Trim()
    }

    $candidates = Get-NetIPAddress -AddressFamily IPv4 |
        Where-Object {
            $_.IPAddress -notlike "127.*" -and
            $_.IPAddress -ne "0.0.0.0" -and
            $_.PrefixOrigin -ne "WellKnown"
        } |
        Sort-Object InterfaceMetric, SkipAsSource |
        Select-Object -ExpandProperty IPAddress -Unique

    if ($candidates.Count -eq 1) {
        return $candidates[0]
    }

    if ($candidates.Count -eq 0) {
        throw "Could not auto-detect a non-loopback IPv4 address. Please pass -ListenAddress explicitly."
    }

    throw "Multiple IPv4 addresses detected ($($candidates -join ', ')). Please pass -ListenAddress explicitly."
}

function Get-StorageHost {
    param([string]$BaseHost)

    $optionsUri = "$($BaseHost.TrimEnd('/'))/internal/v1/app/options"
    $response = Invoke-RestMethod -Uri $optionsUri -Method Get
    $storageHost = [string]$response.storagehost
    if ($storageHost.Trim() -eq "") {
        throw "App options did not include storagehost."
    }
    return $storageHost.Trim()
}

function Ensure-IpHelperRunning {
    $service = Get-Service -Name iphlpsvc -ErrorAction Stop
    if ($service.Status -ne "Running") {
        Write-Info "Starting iphlpsvc service"
        Start-Service -Name iphlpsvc
    }
}

function Remove-PortProxy {
    param(
        [string]$Address,
        [int]$Port
    )

    & netsh interface portproxy delete v4tov4 listenaddress=$Address listenport=$Port | Out-Null
}

function Add-PortProxy {
    param(
        [string]$ListenAddress,
        [int]$ListenPort,
        [string]$ConnectAddress,
        [int]$ConnectPort
    )

    & netsh interface portproxy add v4tov4 `
        listenaddress=$ListenAddress `
        listenport=$ListenPort `
        connectaddress=$ConnectAddress `
        connectport=$ConnectPort | Out-Null
}

function Reset-FirewallRule {
    param(
        [string]$RuleName,
        [int]$Port
    )

    Get-NetFirewallRule -DisplayName $RuleName -ErrorAction SilentlyContinue | Remove-NetFirewallRule | Out-Null
    New-NetFirewallRule `
        -DisplayName $RuleName `
        -Direction Inbound `
        -Action Allow `
        -Protocol TCP `
        -LocalPort $Port | Out-Null
}

function Configure-PortProxyFromStorageHost {
    param(
        [string]$ResolvedListenAddress,
        [string]$RawStorageHost,
        [int]$ConfiguredListenPort,
        [string]$RulePrefix,
        [Nullable[int]]$PreviousListenPort
    )

    $storageUri = [Uri]$RawStorageHost
    if ($storageUri.Port -le 0) {
        throw "Storage host '$RawStorageHost' did not include a valid port."
    }

    $resolvedListenPort = if ($ConfiguredListenPort -gt 0) { $ConfiguredListenPort } else { $storageUri.Port }
    $ruleName = "$RulePrefix ${ResolvedListenAddress}:$resolvedListenPort"

    if ($PreviousListenPort.HasValue -and $PreviousListenPort.Value -ne $resolvedListenPort) {
        $previousRuleName = "$RulePrefix ${ResolvedListenAddress}:$($PreviousListenPort.Value)"
        Write-Info "Removing previous portproxy ${ResolvedListenAddress}:$($PreviousListenPort.Value)"
        Remove-PortProxy -Address $ResolvedListenAddress -Port $PreviousListenPort.Value
        Get-NetFirewallRule -DisplayName $previousRuleName -ErrorAction SilentlyContinue | Remove-NetFirewallRule | Out-Null
    }

    Write-Info "storagehost from app/options: $RawStorageHost"
    Write-Info "Configuring portproxy ${ResolvedListenAddress}:$resolvedListenPort -> $($storageUri.Host):$($storageUri.Port)"

    Remove-PortProxy -Address $ResolvedListenAddress -Port $resolvedListenPort
    Add-PortProxy `
        -ListenAddress $ResolvedListenAddress `
        -ListenPort $resolvedListenPort `
        -ConnectAddress $storageUri.Host `
        -ConnectPort $storageUri.Port
    Reset-FirewallRule -RuleName $ruleName -Port $resolvedListenPort

    Write-Info "Portproxy configured successfully"
    Write-Info "Verify from another machine with:"
    Write-Host "Invoke-WebRequest ""http://${ResolvedListenAddress}:$resolvedListenPort/internal/v1/evidence/cid?cid=1"" -UseBasicParsing"
    return $resolvedListenPort
}

Assert-Admin
Ensure-IpHelperRunning

$resolvedListenAddress = Resolve-ListenAddress -ConfiguredAddress $ListenAddress

$currentStorageHost = Get-StorageHost -BaseHost $AppHost
$currentStorageUri = [Uri]$currentStorageHost
if ($currentStorageUri.Port -le 0) {
    throw "Storage host '$currentStorageHost' did not include a valid port."
}
$currentListenPort = if ($ListenPort -gt 0) { $ListenPort } else { $currentStorageUri.Port }
$currentRuleName = "$FirewallRulePrefix ${resolvedListenAddress}:$currentListenPort"

if ($Cleanup) {
    Write-Info "Removing portproxy ${resolvedListenAddress}:$currentListenPort"
    Remove-PortProxy -Address $resolvedListenAddress -Port $currentListenPort
    Get-NetFirewallRule -DisplayName $currentRuleName -ErrorAction SilentlyContinue | Remove-NetFirewallRule | Out-Null
    Write-Info "Cleanup complete"
    return
}

$configuredListenPort = Configure-PortProxyFromStorageHost `
    -ResolvedListenAddress $resolvedListenAddress `
    -RawStorageHost $currentStorageHost `
    -ConfiguredListenPort $ListenPort `
    -RulePrefix $FirewallRulePrefix `
    -PreviousListenPort $null

Write-Info "Current portproxy table:"
& netsh interface portproxy show v4tov4

if (-not $Watch) {
    return
}

Write-Info "Watch mode enabled; polling app/options every $PollIntervalSec second(s). Press Ctrl+C to stop."
$lastStorageHost = $currentStorageHost

while ($true) {
    Start-Sleep -Seconds $PollIntervalSec
    try {
        $nextStorageHost = Get-StorageHost -BaseHost $AppHost
        if ($nextStorageHost -ne $lastStorageHost) {
            Write-Info "Detected storagehost change: $lastStorageHost -> $nextStorageHost"
            $configuredListenPort = Configure-PortProxyFromStorageHost `
                -ResolvedListenAddress $resolvedListenAddress `
                -RawStorageHost $nextStorageHost `
                -ConfiguredListenPort $ListenPort `
                -RulePrefix $FirewallRulePrefix `
                -PreviousListenPort $configuredListenPort
            Write-Info "Updated portproxy table:"
            & netsh interface portproxy show v4tov4
            $lastStorageHost = $nextStorageHost
        }
    } catch {
        Write-Warning "Watch iteration failed: $($_.Exception.Message)"
    }
}

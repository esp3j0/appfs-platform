# AppFS multi-agent Tinode Windows smoke test
# Covers: compose startup + principal private app materialization + Tinode
# credentials + principal-to-principal direct messages + inbox read-through +
# appfs-agent principal-aware skills/status.

param(
    [string]$AgentId = "appfs-tinode-multi-agent-smoke",
    [string]$MountPoint = "C:\mnt\appfs-tinode-multi-agent-smoke",
    [string]$TinodeEndpoint = $(if ($env:APPFS_TINODE_ENDPOINT) { $env:APPFS_TINODE_ENDPOINT } else { "http://101.34.216.193:6060" }),
    [string]$TinodeApiKey = $(if ($env:APPFS_TINODE_API_KEY) { $env:APPFS_TINODE_API_KEY } else { "AQEAAAABAAD_rAp4DJh05a1HAwFT3A6K" }),
    [string]$TinodeAccountPassword = $(if ($env:APPFS_TINODE_ACCOUNT_PASSWORD) { $env:APPFS_TINODE_ACCOUNT_PASSWORD } else { "TinodeSmoke123!" }),
    [string]$TinodeProtocolVersion = $(if ($env:APPFS_TINODE_PROTOCOL_VERSION) { $env:APPFS_TINODE_PROTOCOL_VERSION } else { "0.25" }),
    [int]$TinodeTimeoutMs = $(if ($env:APPFS_TINODE_TIMEOUT_MS) { [int]$env:APPFS_TINODE_TIMEOUT_MS } else { 10000 }),
    [string]$CodePrincipalId = "code-implementer",
    [int]$MountBootstrapTimeoutSec = 180,
    [int]$TinodeActionTimeoutSec = 90,
    [switch]$SkipCleanup,
    [switch]$KeepLogs
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$script:RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..\..")
$script:AppfsCliDir = Join-Path $script:RepoRoot "appfs\cli"
$script:AppfsAgentRustDir = Join-Path $script:RepoRoot "appfs-agent\rust"
$script:BridgeDir = Join-Path $script:RepoRoot "appfs\examples\appfs\bridges\http-python"
$script:Utf8NoBom = New-Object System.Text.UTF8Encoding($false)
$script:LogDir = Join-Path ([System.IO.Path]::GetTempPath()) ("appfs-tinode-multi-agent-{0}-{1}" -f $AgentId, ([guid]::NewGuid().ToString("N")))
$script:RuntimeBinDir = Join-Path $script:LogDir "bin"
$script:CargoCacheRoot = Join-Path ([System.IO.Path]::GetTempPath()) "appfs-platform-cargo-targets"
$script:AppfsCargoTargetDir = Join-Path $script:CargoCacheRoot "appfs-cli"
$script:ClawCargoTargetDir = Join-Path $script:CargoCacheRoot "appfs-agent-rust"
$script:AppfsExe = Join-Path $script:AppfsCargoTargetDir "debug\agentfs.exe"
$script:ClawExe = Join-Path $script:ClawCargoTargetDir "debug\claw.exe"
$script:DbPath = Join-Path $script:LogDir "$AgentId.db"
$script:ComposePath = Join-Path $script:LogDir "appfs-compose.tinode-multi-agent-smoke.yaml"
$script:AppfsHandle = $null
$script:HadFailure = $false
$script:RunId = ("{0}{1}" -f ([DateTimeOffset]::UtcNow.ToUnixTimeSeconds()), ([guid]::NewGuid().ToString("N").Substring(0, 4)))
$script:TinodeLoginPrefix = "af$($script:RunId)"

. (Join-Path $PSScriptRoot "windows-rust-build-env.ps1")

function Write-Success { Write-Host "[ok] $args" -ForegroundColor Green }
function Write-Fail { Write-Host "[fail] $args" -ForegroundColor Red }
function Write-WarningLine { Write-Host "[warn] $args" -ForegroundColor Yellow }
function Write-Section { Write-Host "`n==== $args ====" -ForegroundColor Magenta }

function Remove-TestPath {
    param(
        [string]$Path,
        [switch]$Recurse,
        [int]$RetryCount = 8,
        [int]$RetryDelayMs = 250
    )

    if (!(Test-Path $Path)) {
        return
    }

    for ($attempt = 1; $attempt -le $RetryCount; $attempt++) {
        try {
            Remove-Item -Path $Path -Force -Recurse:$Recurse -ErrorAction Stop
        } catch {
            if (Test-Path $Path -PathType Container) {
                if ($Recurse) {
                    cmd /c "rmdir /s /q `"$Path`" 2>nul || exit /b 0" | Out-Null
                } else {
                    cmd /c "rmdir `"$Path`" 2>nul || exit /b 0" | Out-Null
                }
            } elseif (Test-Path $Path -PathType Leaf) {
                cmd /c "del /f /q `"$Path`" 2>nul || exit /b 0" | Out-Null
            }
        }

        if (!(Test-Path $Path)) {
            return
        }
        if ($attempt -lt $RetryCount) {
            Start-Sleep -Milliseconds $RetryDelayMs
        }
    }

    if (Test-Path $Path) {
        Write-WarningLine "Cleanup could not remove $Path after $RetryCount attempts."
    }
}

function Wait-ProcessExitById {
    param(
        [int]$ProcessId,
        [int]$TimeoutMs = 3000
    )

    $deadline = (Get-Date).AddMilliseconds($TimeoutMs)
    do {
        $process = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
        if ($null -eq $process) {
            return $true
        }
        Start-Sleep -Milliseconds 200
    } while ((Get-Date) -lt $deadline)

    return $null -eq (Get-Process -Id $ProcessId -ErrorAction SilentlyContinue)
}

function Stop-ProcessTreeById {
    param(
        [int]$ProcessId,
        [string]$ProcessName = "process"
    )

    $process = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
    if ($null -eq $process) {
        return
    }

    try {
        Stop-Process -Id $ProcessId -Force -ErrorAction Stop
    } catch {
        Write-WarningLine "Failed to stop $ProcessName (PID $ProcessId) with Stop-Process: $_"
    }

    if (Wait-ProcessExitById -ProcessId $ProcessId) {
        return
    }

    try {
        & taskkill.exe /F /T /PID $ProcessId | Out-Null
    } catch {
        Write-WarningLine "taskkill failed for $ProcessName (PID $ProcessId): $_"
    } finally {
        $global:LASTEXITCODE = 0
    }
}

function Stop-LoggedProcess {
    param($Handle)

    if ($null -eq $Handle -or $null -eq $Handle.Process) {
        return
    }

    $processId = $Handle.Process.Id
    try {
        if (!$Handle.Process.HasExited) {
            Stop-ProcessTreeById -ProcessId $processId -ProcessName $Handle.Name
        }
    } finally {
        try {
            $Handle.Process.Dispose()
        } catch {
        }
    }
}

function Stop-StaleSmokeProcesses {
    Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
        Where-Object {
            ($_.Name -eq "agentfs.exe" -and $_.CommandLine -and $_.CommandLine -like "*$AgentId*") -or
            ($_.Name -eq "python.exe" -and $_.CommandLine -and $_.CommandLine -like "*bridge_server.py*")
        } |
        ForEach-Object {
            Stop-ProcessTreeById -ProcessId $_.ProcessId -ProcessName $_.Name
        }
}

function Cleanup-TestArtifacts {
    Write-Host "[info] Starting Tinode multi-agent smoke cleanup"
    Stop-LoggedProcess $script:AppfsHandle
    Stop-StaleSmokeProcesses

    if (!$SkipCleanup) {
        Remove-TestPath -Path $MountPoint -Recurse -RetryCount 40 -RetryDelayMs 500
    }

    if (!$KeepLogs -and !$script:HadFailure -and (Test-Path $script:LogDir)) {
        Remove-TestPath -Path $script:LogDir -Recurse
    } elseif (Test-Path $script:LogDir) {
        Write-Host "[info] Logs preserved at $script:LogDir"
    }
}

function Fail-WithContext {
    param([string]$Message)

    $script:HadFailure = $true
    Write-Fail $Message
    if (Test-Path $script:LogDir) {
        Write-Host "`nLogs preserved at $script:LogDir" -ForegroundColor Gray
        foreach ($path in @(
            (Join-Path $script:LogDir "appfs-build.log"),
            (Join-Path $script:LogDir "claw-build.log"),
            (Join-Path $script:LogDir "appfs-compose.stdout.log"),
            (Join-Path $script:LogDir "appfs-compose.stderr.log"),
            (Join-Path $script:LogDir "claw-status-default.log"),
            (Join-Path $script:LogDir "claw-status-code.log"),
            (Join-Path $script:LogDir "claw-skills-default.log"),
            (Join-Path $script:LogDir "claw-skills-code.log")
        )) {
            if (Test-Path $path) {
                Write-Host "`n--- tail: $path ---" -ForegroundColor Gray
                Get-Content $path -Tail 80 -ErrorAction SilentlyContinue
            }
        }
    }
    throw $Message
}

function Assert-True {
    param(
        [bool]$Condition,
        [string]$Message
    )

    if (!$Condition) {
        Fail-WithContext $Message
    }
    Write-Success $Message
}

function Ensure-ProcessRunning {
    param($Handle)

    if ($Handle.Process.HasExited) {
        Fail-WithContext "$($Handle.Name) exited unexpectedly with code $($Handle.Process.ExitCode)"
    }
}

function Wait-Until {
    param(
        [scriptblock]$Condition,
        [string]$Description,
        [int]$TimeoutSec = 20
    )

    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    $lastError = $null
    while ((Get-Date) -lt $deadline) {
        try {
            if (& $Condition) {
                return
            }
        } catch {
            $lastError = $_.Exception.Message
        }
        Start-Sleep -Milliseconds 500
    }

    if ($lastError) {
        Fail-WithContext "Timed out waiting for $Description. Last error: $lastError"
    }
    Fail-WithContext "Timed out waiting for $Description"
}

function New-LogHandle {
    param(
        [string]$Name,
        [string]$FilePath,
        [string[]]$ArgumentList,
        [string]$WorkingDirectory
    )

    $stdout = Join-Path $script:LogDir "$Name.stdout.log"
    $stderr = Join-Path $script:LogDir "$Name.stderr.log"
    $proc = Start-Process -FilePath $FilePath `
        -ArgumentList $ArgumentList `
        -WorkingDirectory $WorkingDirectory `
        -PassThru `
        -WindowStyle Hidden `
        -RedirectStandardOutput $stdout `
        -RedirectStandardError $stderr

    return [pscustomobject]@{
        Name = $Name
        Process = $proc
        Stdout = $stdout
        Stderr = $stderr
    }
}

function Invoke-LoggedCommand {
    param(
        [string]$Name,
        [string]$FilePath,
        [string[]]$ArgumentList,
        [string]$WorkingDirectory,
        [hashtable]$Environment = @{}
    )

    $logPath = Join-Path $script:LogDir "$Name.log"
    $stdoutPath = Join-Path $script:LogDir "$Name.stdout.tmp.log"
    $stderrPath = Join-Path $script:LogDir "$Name.stderr.tmp.log"
    $oldEnv = @{}
    foreach ($key in $Environment.Keys) {
        $oldEnv[$key] = [Environment]::GetEnvironmentVariable($key, "Process")
        [Environment]::SetEnvironmentVariable($key, [string]$Environment[$key], "Process")
    }

    try {
        $proc = Start-Process -FilePath $FilePath `
            -ArgumentList $ArgumentList `
            -WorkingDirectory $WorkingDirectory `
            -PassThru `
            -WindowStyle Hidden `
            -Wait `
            -RedirectStandardOutput $stdoutPath `
            -RedirectStandardError $stderrPath
        $exitCode = $proc.ExitCode
    } finally {
        foreach ($key in $Environment.Keys) {
            [Environment]::SetEnvironmentVariable($key, $oldEnv[$key], "Process")
        }
    }

    $stdout = if (Test-Path $stdoutPath) { Get-Content $stdoutPath -Raw } else { "" }
    $stderr = if (Test-Path $stderrPath) { Get-Content $stderrPath -Raw } else { "" }
    $text = ($stdout + $stderr).TrimEnd()
    [System.IO.File]::WriteAllText($logPath, $text + [Environment]::NewLine, $script:Utf8NoBom)

    Remove-TestPath -Path $stdoutPath
    Remove-TestPath -Path $stderrPath

    if ($exitCode -ne 0) {
        Fail-WithContext "$Name failed with exit code $exitCode"
    }
    if ($text) {
        Write-Host $text
    }
    return $text
}

function Require-Command {
    param([string]$Name)

    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "Required command not found: $Name"
    }
}

function Build-TestBinaries {
    Write-Section "Build Test Binaries"
    Initialize-WindowsRustBuildEnv

    Invoke-WithWindowsIntegrationBuildLock {
        Clear-WindowsIntegrationCargoTargetIfLowSpace -CacheRoot $script:CargoCacheRoot
        Clear-WindowsIntegrationExecutableTargets -ExecutablePaths @(
            $script:AppfsExe,
            $script:ClawExe
        )

        $appfsBuildResult = Invoke-WindowsIntegrationStreamingCommand -Name "appfs-build" -FilePath "cargo" -ArgumentList @(
            "build",
            "--target-dir", $script:AppfsCargoTargetDir,
            "--bin", "agentfs"
        ) -WorkingDirectory $script:AppfsCliDir -LogPath (Join-Path $script:LogDir "appfs-build.log") -Encoding $script:Utf8NoBom
        if ($appfsBuildResult.ExitCode -ne 0) {
            Fail-WithContext "appfs-build failed with exit code $($appfsBuildResult.ExitCode)"
        }
        Assert-True (Test-Path $script:AppfsExe) "Built AppFS CLI binary $script:AppfsExe"

        $clawBuildResult = Invoke-WindowsIntegrationStreamingCommand -Name "claw-build" -FilePath "cargo" -ArgumentList @(
            "build",
            "--target-dir", $script:ClawCargoTargetDir,
            "--manifest-path", (Join-Path $script:AppfsAgentRustDir "Cargo.toml"),
            "-p", "rusty-claude-cli"
        ) -WorkingDirectory $script:AppfsAgentRustDir -LogPath (Join-Path $script:LogDir "claw-build.log") -Encoding $script:Utf8NoBom
        if ($clawBuildResult.ExitCode -ne 0) {
            Fail-WithContext "claw-build failed with exit code $($clawBuildResult.ExitCode)"
        }
        Assert-True (Test-Path $script:ClawExe) "Built appfs-agent CLI binary $script:ClawExe"
    }
}

function Stage-TestBinaries {
    $script:AppfsExe = Copy-WindowsIntegrationExecutableForRun -SourcePath $script:AppfsExe -DestinationDirectory $script:RuntimeBinDir
    $script:ClawExe = Copy-WindowsIntegrationExecutableForRun -SourcePath $script:ClawExe -DestinationDirectory $script:RuntimeBinDir
}

function Convert-ToYamlPath {
    param([string]$Path)

    return ([System.IO.Path]::GetFullPath($Path) -replace "\\", "/")
}

function Write-SmokeComposeFile {
    $dbPath = Convert-ToYamlPath $script:DbPath
    $mountPath = Convert-ToYamlPath $MountPoint
    $bridgeDir = Convert-ToYamlPath $script:BridgeDir
    $bridgeUri = [Uri]"http://127.0.0.1:8080"

    $yaml = @"
version: 1
name: appfs-tinode-multi-agent-smoke

runtime:
  db: $dbPath
  mountpoint: $mountPath
  backend: winfsp
  init: if_missing
  reset: true
  poll_ms: 0

connectors:
  aiim-http:
    mode: command
    transport: http
    endpoint: http://127.0.0.1:8080
    healthcheck:
      kind: connector
      interval_ms: 500
      timeout_ms: 2000
      max_attempts: 40
    command:
      cwd: $bridgeDir
      program: python
      args: ["-u", "bridge_server.py"]
      env:
        APPFS_HTTP_BRIDGE_BACKEND: aiim
        APPFS_BRIDGE_HOST: $($bridgeUri.Host)
        APPFS_BRIDGE_PORT: "$($bridgeUri.Port)"
  tinode-in-process:
    mode: in_process
    transport: in_process

apps:
  aiim:
    connector: aiim-http
    visibility: public
    transport:
      http_timeout_ms: 5000
      grpc_timeout_ms: 5000
      bridge_max_retries: 2
      bridge_initial_backoff_ms: 100
      bridge_max_backoff_ms: 1000
      bridge_circuit_breaker_failures: 5
      bridge_circuit_breaker_cooldown_ms: 3000
  tinode:
    connector: tinode-in-process
    visibility: private
    path_template: private/{principal_id}/tinode
    profile_template: tinode:{principal_id}
    credential_policy: auto-create
    transport:
      http_timeout_ms: 5000
      grpc_timeout_ms: 5000
      bridge_max_retries: 2
      bridge_initial_backoff_ms: 100
      bridge_max_backoff_ms: 1000
      bridge_circuit_breaker_failures: 5
      bridge_circuit_breaker_cooldown_ms: 3000
"@
    [System.IO.File]::WriteAllText($script:ComposePath, $yaml, $script:Utf8NoBom)
}

function Append-Utf8JsonLine {
    param(
        [string]$Path,
        [string]$JsonLine
    )

    $parent = Split-Path -Parent $Path
    if ($parent) {
        [void][System.IO.Directory]::CreateDirectory($parent)
    }
    [System.IO.File]::AppendAllText($Path, $JsonLine + "`n", $script:Utf8NoBom)
}

function Read-TextIfExists {
    param([string]$Path)

    if (!(Test-Path $Path)) {
        return ""
    }
    try {
        return Get-Content $Path -Raw -ErrorAction Stop
    } catch {
        return ""
    }
}

function Read-TailTextIfExists {
    param(
        [string]$Path,
        [int]$Tail = 80
    )

    if (!(Test-Path $Path)) {
        return ""
    }
    try {
        return (@(Get-Content $Path -Tail $Tail -ErrorAction Stop) -join "`n")
    } catch {
        return ""
    }
}

function Get-TinodePath {
    param(
        [string]$PrincipalId,
        [string]$RelativePath
    )

    return Join-Path $MountPoint ("private\{0}\tinode\{1}" -f $PrincipalId, $RelativePath)
}

function Wait-EventStreamContains {
    param(
        [string]$PrincipalId,
        [string]$ClientToken,
        [string]$EventType,
        [int]$TimeoutSec = $TinodeActionTimeoutSec
    )

    $eventsPath = Get-TinodePath -PrincipalId $PrincipalId -RelativePath "_stream\events.evt.jsonl"
    Wait-Until -Description "$PrincipalId event $EventType for $ClientToken" -TimeoutSec $TimeoutSec -Condition {
        Ensure-ProcessRunning $script:AppfsHandle
        $tail = Read-TailTextIfExists -Path $eventsPath -Tail 120
        return $tail.Contains($ClientToken) -and $tail.Contains("`"type`":`"$EventType`"")
    }
}

function Wait-InboxContains {
    param(
        [string]$PrincipalId,
        [string]$ExpectedText,
        [int]$TimeoutSec = $TinodeActionTimeoutSec
    )

    $inboxPath = Get-TinodePath -PrincipalId $PrincipalId -RelativePath "inbox\recent.res.jsonl"
    Wait-Until -Description "$PrincipalId inbox contains expected message" -TimeoutSec $TimeoutSec -Condition {
        Ensure-ProcessRunning $script:AppfsHandle
        $content = Read-TextIfExists -Path $inboxPath
        return $content.Contains($ExpectedText)
    }
}

function Assert-ClawPrincipalView {
    param(
        [string]$PrincipalId,
        [string]$LogSuffix
    )

    $envMap = @{ APPFS_PRINCIPAL_ID = $PrincipalId }
    $statusOutput = Invoke-LoggedCommand -Name "claw-status-$LogSuffix" -FilePath $script:ClawExe -ArgumentList @(
        "status"
    ) -WorkingDirectory $MountPoint -Environment $envMap
    Assert-True ($statusOutput.Contains("Principal id      $PrincipalId")) "claw status reports principal $PrincipalId"
    Assert-True ($statusOutput.Contains("tinode [private/$PrincipalId/tinode]") -or $statusOutput.Contains("tinode [private\$PrincipalId\tinode]")) "claw status sees $PrincipalId private Tinode app"

    $skillsOutput = Invoke-LoggedCommand -Name "claw-skills-$LogSuffix" -FilePath $script:ClawExe -ArgumentList @(
        "--output-format", "json",
        "skills"
    ) -WorkingDirectory $MountPoint -Environment $envMap
    Assert-True ($skillsOutput.Contains("appfs-tinode")) "claw skills lists appfs-tinode for $PrincipalId"
}

function Main {
    Require-Command cargo
    Require-Command python

    [void][System.IO.Directory]::CreateDirectory($script:LogDir)
    [void][System.IO.Directory]::CreateDirectory($script:CargoCacheRoot)
    Stop-StaleSmokeProcesses
    Build-TestBinaries
    Stage-TestBinaries

    $mountParent = Split-Path -Parent $MountPoint
    if ($mountParent -and !(Test-Path $mountParent)) {
        New-Item -ItemType Directory -Path $mountParent -Force | Out-Null
    }
    if (Test-Path $MountPoint) {
        Remove-TestPath -Path $MountPoint -Recurse -RetryCount 40 -RetryDelayMs 500
    }

    Write-SmokeComposeFile
    Write-Host "[info] Tinode endpoint: $TinodeEndpoint"
    Write-Host "[info] Tinode login prefix: $script:TinodeLoginPrefix"
    Write-Host "[info] Compose file: $script:ComposePath"

    Write-Section "Start AppFS Compose"
    $oldTinodeEnv = @{
        APPFS_TINODE_ENDPOINT = [Environment]::GetEnvironmentVariable("APPFS_TINODE_ENDPOINT", "Process")
        APPFS_TINODE_API_KEY = [Environment]::GetEnvironmentVariable("APPFS_TINODE_API_KEY", "Process")
        APPFS_TINODE_LOGIN_PREFIX = [Environment]::GetEnvironmentVariable("APPFS_TINODE_LOGIN_PREFIX", "Process")
        APPFS_TINODE_CREDENTIAL_POLICY = [Environment]::GetEnvironmentVariable("APPFS_TINODE_CREDENTIAL_POLICY", "Process")
        APPFS_TINODE_ACCOUNT_PASSWORD = [Environment]::GetEnvironmentVariable("APPFS_TINODE_ACCOUNT_PASSWORD", "Process")
        APPFS_TINODE_PROTOCOL_VERSION = [Environment]::GetEnvironmentVariable("APPFS_TINODE_PROTOCOL_VERSION", "Process")
        APPFS_TINODE_TIMEOUT_MS = [Environment]::GetEnvironmentVariable("APPFS_TINODE_TIMEOUT_MS", "Process")
    }
    [Environment]::SetEnvironmentVariable("APPFS_TINODE_ENDPOINT", $TinodeEndpoint, "Process")
    [Environment]::SetEnvironmentVariable("APPFS_TINODE_API_KEY", $TinodeApiKey, "Process")
    [Environment]::SetEnvironmentVariable("APPFS_TINODE_LOGIN_PREFIX", $script:TinodeLoginPrefix, "Process")
    [Environment]::SetEnvironmentVariable("APPFS_TINODE_CREDENTIAL_POLICY", "auto-create", "Process")
    [Environment]::SetEnvironmentVariable("APPFS_TINODE_ACCOUNT_PASSWORD", $TinodeAccountPassword, "Process")
    [Environment]::SetEnvironmentVariable("APPFS_TINODE_PROTOCOL_VERSION", $TinodeProtocolVersion, "Process")
    [Environment]::SetEnvironmentVariable("APPFS_TINODE_TIMEOUT_MS", [string]$TinodeTimeoutMs, "Process")
    try {
        $script:AppfsHandle = New-LogHandle -Name "appfs-compose" -FilePath $script:AppfsExe -ArgumentList @(
            "appfs", "compose", "up", "-f", $script:ComposePath
        ) -WorkingDirectory $script:RepoRoot
    } finally {
        foreach ($key in $oldTinodeEnv.Keys) {
            [Environment]::SetEnvironmentVariable($key, $oldTinodeEnv[$key], "Process")
        }
    }

    $createPrincipalAction = Join-Path $MountPoint "_appfs\principals\create_principal.act"
    $defaultEnsure = Get-TinodePath -PrincipalId "default" -RelativePath "_app\ensure_credentials.act"
    Wait-Until -Description "default Tinode private app materialized" -TimeoutSec $MountBootstrapTimeoutSec -Condition {
        Ensure-ProcessRunning $script:AppfsHandle
        return (Test-Path $defaultEnsure) -and (Test-Path $createPrincipalAction)
    }
    Write-Success "default Tinode private app is materialized"

    Write-Section "Create Second Principal"
    $createPrincipalPayload = @{
        principal_id = $CodePrincipalId
        display_name = $CodePrincipalId
        description = "Implementation worker principal for multi-agent Tinode smoke."
        kind = "agent"
        client_token = "create-$CodePrincipalId-$($script:RunId)"
    } | ConvertTo-Json -Compress
    Append-Utf8JsonLine -Path $createPrincipalAction -JsonLine $createPrincipalPayload

    $codeEnsure = Get-TinodePath -PrincipalId $CodePrincipalId -RelativePath "_app\ensure_credentials.act"
    Wait-Until -Description "$CodePrincipalId Tinode private app materialized" -TimeoutSec 60 -Condition {
        Ensure-ProcessRunning $script:AppfsHandle
        return (Test-Path $codeEnsure)
    }
    Write-Success "$CodePrincipalId Tinode private app is materialized"

    $registryText = Read-TextIfExists -Path (Join-Path $MountPoint "_appfs\apps.registry.json")
    Assert-True ($registryText.Contains('"instance_id":"tinode--default"') -or $registryText.Contains('"instance_id": "tinode--default"')) "apps registry contains tinode--default"
    Assert-True ($registryText.Contains("tinode--$CodePrincipalId")) "apps registry contains tinode--$CodePrincipalId"

    Write-Section "Verify appfs-agent Principal Views"
    Assert-ClawPrincipalView -PrincipalId "default" -LogSuffix "default"
    Assert-ClawPrincipalView -PrincipalId $CodePrincipalId -LogSuffix "code"

    Write-Section "Ensure Tinode Credentials"
    $ensureDefaultToken = "ensure-default-$($script:RunId)"
    $ensureCodeToken = "ensure-code-$($script:RunId)"
    Append-Utf8JsonLine -Path $defaultEnsure -JsonLine (@{
        expected_profile_id = "tinode:default"
        client_token = $ensureDefaultToken
    } | ConvertTo-Json -Compress)
    Append-Utf8JsonLine -Path $codeEnsure -JsonLine (@{
        expected_profile_id = "tinode:$CodePrincipalId"
        client_token = $ensureCodeToken
    } | ConvertTo-Json -Compress)

    Wait-EventStreamContains -PrincipalId "default" -ClientToken $ensureDefaultToken -EventType "profile.credentials.ready"
    Wait-EventStreamContains -PrincipalId $CodePrincipalId -ClientToken $ensureCodeToken -EventType "profile.credentials.ready"
    Write-Success "Tinode credentials are ready for both principals"

    Write-Section "Send default to code-implementer"
    $defaultToCodeText = "smoke $($script:RunId): default delegates implementation details to $CodePrincipalId."
    $defaultToCodeToken = "msg-default-code-$($script:RunId)"
    Append-Utf8JsonLine -Path (Get-TinodePath -PrincipalId "default" -RelativePath "contacts\send_message.act") -JsonLine (@{
        to = "principal:$CodePrincipalId"
        text = $defaultToCodeText
        client_token = $defaultToCodeToken
    } | ConvertTo-Json -Compress)
    Wait-EventStreamContains -PrincipalId "default" -ClientToken $defaultToCodeToken -EventType "action.completed"
    Wait-InboxContains -PrincipalId $CodePrincipalId -ExpectedText $defaultToCodeText
    Write-Success "$CodePrincipalId inbox read-through sees default's message"

    Write-Section "Send code-implementer to default"
    $codeToDefaultText = "smoke $($script:RunId): $CodePrincipalId acknowledges and starts implementation."
    $codeToDefaultToken = "msg-code-default-$($script:RunId)"
    Append-Utf8JsonLine -Path (Get-TinodePath -PrincipalId $CodePrincipalId -RelativePath "contacts\send_message.act") -JsonLine (@{
        to = "principal:default"
        text = $codeToDefaultText
        client_token = $codeToDefaultToken
    } | ConvertTo-Json -Compress)
    Wait-EventStreamContains -PrincipalId $CodePrincipalId -ClientToken $codeToDefaultToken -EventType "action.completed"
    Wait-InboxContains -PrincipalId "default" -ExpectedText $codeToDefaultText
    Write-Success "default inbox read-through sees $CodePrincipalId's message"

    Write-Host "`nAppFS multi-agent Tinode smoke test passed." -ForegroundColor Green
}

try {
    Main
} finally {
    Cleanup-TestArtifacts
}

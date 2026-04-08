# AppFS + appfs-agent Windows HTTP demo integration smoke test
# Covers: http bridge + app registration + snapshot read + action submit via appfs-agent
# Contract checkpoint: IC-1 in integration/APPFS-appfs-agent-attach-contract-v1.1.md

param(
    [string]$AgentId = "appfs-agent-http-demo",
    [string]$MountPoint = "C:\mnt\appfs-agent-http-demo",
    [string]$AppId = "aiim",
    [string]$HttpEndpoint = "http://127.0.0.1:8080",
    [int]$MountBootstrapTimeoutSec = 180,
    [switch]$SkipCleanup,
    [switch]$KeepLogs
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$script:RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..\..")
$script:AppfsCliDir = Join-Path $script:RepoRoot "appfs\cli"
$script:AppfsAgentRustDir = Join-Path $script:RepoRoot "appfs-agent\rust"
$script:BridgeDir = Join-Path $script:RepoRoot "appfs\examples\appfs\bridges\http-python"
$script:DbPath = Join-Path $script:AppfsCliDir ".agentfs\$AgentId.db"
$script:AppfsHandle = $null
$script:BridgeHandle = $null
$script:Utf8NoBom = New-Object System.Text.UTF8Encoding($false)
$script:LogDir = Join-Path ([System.IO.Path]::GetTempPath()) ("appfs-agent-http-demo-{0}-{1}" -f $AgentId, ([guid]::NewGuid().ToString("N")))
$script:RuntimeBinDir = Join-Path $script:LogDir "bin"
$script:HadFailure = $false
$script:CargoCacheRoot = Join-Path ([System.IO.Path]::GetTempPath()) "appfs-platform-cargo-targets"
$script:AppfsCargoTargetDir = Join-Path $script:CargoCacheRoot "appfs-cli"
$script:ClawCargoTargetDir = Join-Path $script:CargoCacheRoot "appfs-agent-rust"
$script:AppfsExe = Join-Path $script:AppfsCargoTargetDir "debug\agentfs.exe"
$script:ClawExe = Join-Path $script:ClawCargoTargetDir "debug\claw.exe"

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

function Cleanup-StaleTempArtifacts {
    Stop-AgentfsProcessesForAgentId
    Stop-BridgeProcesses

    $tempRoot = [System.IO.Path]::GetTempPath()
    foreach ($pattern in @("appfs-agent-http-demo-*")) {
        Get-ChildItem -Path $tempRoot -Directory -Filter $pattern -ErrorAction SilentlyContinue |
            Where-Object { $_.FullName -ne $script:LogDir } |
            ForEach-Object { Remove-TestPath -Path $_.FullName -Recurse }
    }
}

function Wait-ProcessExitById {
    param(
        [int]$ProcessId,
        [int]$TimeoutMs = 2000
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
        [string]$ProcessLabel,
        [int]$TimeoutMs = 2000
    )

    if ($ProcessId -le 0) {
        return
    }

    try {
        Stop-Process -Id $ProcessId -Force -ErrorAction Stop
    } catch {
    }

    if (-not (Wait-ProcessExitById -ProcessId $ProcessId -TimeoutMs $TimeoutMs)) {
        try {
            & taskkill.exe /F /T /PID $ProcessId | Out-Null
        } catch {
        } finally {
            $global:LASTEXITCODE = 0
        }

        if (-not (Wait-ProcessExitById -ProcessId $ProcessId -TimeoutMs $TimeoutMs)) {
            Write-WarningLine "Cleanup could not fully stop $ProcessLabel (PID $ProcessId)."
        }
    }
}

function Wait-For-PathToDisappear {
    param(
        [string]$Path,
        [int]$TimeoutSec = 15
    )

    if (!(Test-Path $Path)) {
        return $true
    }

    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    while ((Get-Date) -lt $deadline) {
        Start-Sleep -Milliseconds 250
        if (!(Test-Path $Path)) {
            return $true
        }
    }

    return !(Test-Path $Path)
}

function Stop-AgentfsProcessesForAgentId {
    Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
        Where-Object {
            $_.Name -eq "agentfs.exe" -and
            $_.CommandLine -and
            $_.CommandLine -like "*$AgentId*"
        } |
        ForEach-Object {
            Stop-ProcessTreeById -ProcessId $_.ProcessId -ProcessLabel $_.Name
        }
}

function Stop-BridgeProcesses {
    Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
        Where-Object {
            $_.Name -eq "python.exe" -and
            $_.CommandLine -and
            $_.CommandLine -like "*bridge_server.py*"
        } |
        ForEach-Object {
            Stop-ProcessTreeById -ProcessId $_.ProcessId -ProcessLabel $_.Name
        }
}

function Stop-LoggedProcess {
    param($Handle)

    if ($null -eq $Handle) {
        return
    }

    if ($Handle.Process) {
        $processId = $Handle.Process.Id
        try {
            if (!$Handle.Process.HasExited) {
                try {
                    Stop-Process -Id $processId -Force -ErrorAction Stop
                } catch {
                    Write-WarningLine "Failed to stop $($Handle.Name): $_"
                }

                if (-not (Wait-ProcessExitById -ProcessId $processId -TimeoutMs 2000)) {
                    cmd /c "taskkill /F /T /PID $processId >nul 2>nul || exit /b 0" | Out-Null
                    $null = Wait-ProcessExitById -ProcessId $processId -TimeoutMs 2000
                }
            }
        } finally {
            try {
                $Handle.Process.Dispose()
            } catch {
            }
        }
    }
}

function Invoke-CleanupStep {
    param(
        [string]$Name,
        [scriptblock]$Action
    )

    $stepStarted = Get-Date
    Write-Host "[info] Cleanup: $Name"
    & $Action
    $elapsed = ((Get-Date) - $stepStarted).TotalSeconds
    Write-Host ("[info] Cleanup: {0} finished in {1:N1}s" -f $Name, $elapsed)
}

function Cleanup-TestArtifacts {
    $cleanupStarted = Get-Date
    Write-Host "[info] Starting HTTP demo cleanup"

    Invoke-CleanupStep "sweep lingering appfs/bridge processes" {
        Stop-AgentfsProcessesForAgentId
        Stop-BridgeProcesses
        Start-Sleep -Milliseconds 500
    }
    Invoke-CleanupStep "stop appfs" { Stop-LoggedProcess $script:AppfsHandle }
    Invoke-CleanupStep "stop bridge" { Stop-LoggedProcess $script:BridgeHandle }

    if (!$SkipCleanup) {
        Invoke-CleanupStep "remove mount and database artifacts" {
            if (Test-Path $MountPoint) {
                if (Wait-For-PathToDisappear -Path $MountPoint -TimeoutSec 15) {
                    Write-Host "[info] AppFS mountpoint auto-unmounted before filesystem cleanup"
                } else {
                    Write-WarningLine "AppFS mountpoint still exists 15s after shutdown; forcing lingering process sweep before removal."
                    Stop-AgentfsProcessesForAgentId
                }
            }
            Remove-TestPath -Path $MountPoint -Recurse
            Remove-TestPath -Path $script:DbPath
            Remove-TestPath -Path "$($script:DbPath)-shm"
            Remove-TestPath -Path "$($script:DbPath)-wal"
        }
    }

    if (!$KeepLogs -and !$script:HadFailure -and (Test-Path $script:LogDir)) {
        Invoke-CleanupStep "remove temporary log directory" {
            Remove-TestPath -Path $script:LogDir -Recurse
        }
    }

    $cleanupElapsed = ((Get-Date) - $cleanupStarted).TotalSeconds
    Write-Host ("[info] HTTP demo cleanup finished in {0:N1}s" -f $cleanupElapsed)
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
            (Join-Path $script:LogDir "bridge.stdout.log"),
            (Join-Path $script:LogDir "bridge.stderr.log"),
            (Join-Path $script:LogDir "appfs-up.stdout.log"),
            (Join-Path $script:LogDir "appfs-up.stderr.log"),
            (Join-Path $script:LogDir "claw-demo.log")
        )) {
            if (Test-Path $path) {
                Write-Host "`n--- tail: $path ---" -ForegroundColor Gray
                Get-Content $path -Tail 60 -ErrorAction SilentlyContinue
            }
        }
    }
    throw $Message
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

function Read-AppfsRuntimeManifest {
    param([string]$MountRoot)

    $manifestPath = Join-Path $MountRoot ".well-known\appfs\runtime.json"
    Assert-True (Test-Path $manifestPath) "AppFS runtime manifest exists at $manifestPath"

    $raw = Get-Content $manifestPath -Raw -ErrorAction Stop
    $manifest = $raw | ConvertFrom-Json -ErrorAction Stop

    Assert-True ($manifest.schema_version -eq 1) "AppFS runtime manifest schema_version is 1"
    Assert-True ($manifest.runtime_kind -eq "appfs") "AppFS runtime manifest runtime_kind is appfs"
    Assert-True (
        $manifest.multi_agent_mode -eq "shared_mount_distinct_attach"
    ) "AppFS runtime manifest multi_agent_mode is shared_mount_distinct_attach"
    Assert-True (
        -not [string]::IsNullOrWhiteSpace([string]$manifest.runtime_session_id)
    ) "AppFS runtime manifest runtime_session_id is populated"
    Assert-True (
        $manifest.capabilities.multi_agent_attach -eq $true
    ) "AppFS runtime manifest advertises multi_agent_attach capability"

    return [pscustomobject]@{
        Path = $manifestPath
        Document = $manifest
    }
}

function Ensure-ProcessRunning {
    param($Handle)

    if ($Handle.Process.HasExited) {
        throw "$($Handle.Name) exited unexpectedly with code $($Handle.Process.ExitCode)"
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
        Start-Sleep -Milliseconds 250
    }

    if ($lastError) {
        Fail-WithContext "Timed out waiting for $Description. Last error: $lastError"
    } else {
        Fail-WithContext "Timed out waiting for $Description"
    }
}

function Invoke-LoggedCommandResult {
    param(
        [string]$Name,
        [string]$FilePath,
        [string[]]$ArgumentList,
        [string]$WorkingDirectory
    )

    $logPath = Join-Path $script:LogDir "$Name.log"
    $stdoutPath = Join-Path $script:LogDir "$Name.stdout.tmp.log"
    $stderrPath = Join-Path $script:LogDir "$Name.stderr.tmp.log"
    $exitCode = 0
    $command = Get-Command $FilePath -ErrorAction SilentlyContinue
    $resolvedFilePath = if ($null -ne $command) { $command.Source } else { $FilePath }
    $stopwatch = [System.Diagnostics.Stopwatch]::StartNew()

    Write-Host (
        "[info] Starting {0} from {1}" -f $Name, $WorkingDirectory
    ) -ForegroundColor DarkGray

    Push-Location $WorkingDirectory
    try {
        $proc = Start-Process -FilePath $resolvedFilePath `
            -ArgumentList $ArgumentList `
            -WorkingDirectory $WorkingDirectory `
            -PassThru `
            -WindowStyle Hidden `
            -Wait `
            -RedirectStandardOutput $stdoutPath `
            -RedirectStandardError $stderrPath
        $exitCode = $proc.ExitCode
    } finally {
        $stopwatch.Stop()
        Pop-Location
    }

    Write-Host (
        "[info] Completed {0} in {1} with exit code {2}" -f
            $Name,
            (Format-WindowsIntegrationElapsed -Elapsed $stopwatch.Elapsed),
            $exitCode
    ) -ForegroundColor DarkGray

    $stdout = if (Test-Path $stdoutPath) { Get-Content $stdoutPath -Raw } else { "" }
    $stderr = if (Test-Path $stderrPath) { Get-Content $stderrPath -Raw } else { "" }
    $text = ($stdout + $stderr).TrimEnd()
    [System.IO.File]::WriteAllText($logPath, $text + [Environment]::NewLine, $script:Utf8NoBom)

    Remove-TestPath -Path $stdoutPath
    Remove-TestPath -Path $stderrPath

    return [pscustomobject]@{
        ExitCode = $exitCode
        Text = $text
        LogPath = $logPath
    }
}

function Invoke-LoggedCommand {
    param(
        [string]$Name,
        [string]$FilePath,
        [string[]]$ArgumentList,
        [string]$WorkingDirectory
    )

    $result = Invoke-LoggedCommandResult -Name $Name -FilePath $FilePath -ArgumentList $ArgumentList -WorkingDirectory $WorkingDirectory
    if ($result.ExitCode -ne 0) {
        Fail-WithContext "$Name failed with exit code $($result.ExitCode)"
    }
    if ($result.Text) {
        Write-Host $result.Text
    }
    return $result.Text
}

function Test-IsTransientClawDemoFailure {
    param([string]$Text)

    if ([string]::IsNullOrWhiteSpace($Text)) {
        return $false
    }

    $normalized = $Text.ToLowerInvariant()
    return (
        $normalized.Contains("failed to parse anthropic response") -or
        (
            (
                $normalized.Contains('"type":"error"') -or
                $normalized.Contains("unknown variant 'error'")
            ) -and (
                $normalized.Contains('"code":"429"') -or
                $normalized.Contains('"code":"500"') -or
                $normalized.Contains('"code":"502"') -or
                $normalized.Contains('"code":"503"') -or
                $normalized.Contains('"code":"504"') -or
                $normalized.Contains('"code":"529"')
            )
        ) -or
        $normalized.Contains('"code":"429"') -or
        $normalized.Contains('"code":"500"') -or
        $normalized.Contains('"code":"502"') -or
        $normalized.Contains('"code":"503"') -or
        $normalized.Contains('"code":"504"') -or
        $normalized.Contains('"code":"529"') -or
        $normalized.Contains("overloaded_error") -or
        $normalized.Contains("rate limit") -or
        $normalized.Contains("timed out") -or
        $normalized.Contains("timeout")
    )
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

function Write-Utf8TextFile {
    param(
        [string]$Path,
        [string]$Content
    )

    $parent = Split-Path -Parent $Path
    if ($parent) {
        [void][System.IO.Directory]::CreateDirectory($parent)
    }
    [System.IO.File]::WriteAllText($Path, $Content, $script:Utf8NoBom)
}

function ConvertFrom-JsonSafe {
    param([string]$Text)

    if ([string]::IsNullOrWhiteSpace($Text)) {
        return $null
    }

    try {
        return $Text | ConvertFrom-Json -ErrorAction Stop
    } catch {
        return $null
    }
}

function Read-RecentTextFile {
    param(
        [string]$Path,
        [int]$Tail = 200
    )

    return (@(Get-Content $Path -Tail $Tail -ErrorAction Stop) -join "`n")
}

function Write-DemoScript {
    param(
        [string]$Path,
        [string]$ClientToken
    )

    $demoScript = @"
#!/usr/bin/env bash
set -euo pipefail
echo "__PWD__"
pwd
echo "__SNAPSHOT__"
head -n 3 chats/chat-001/messages.res.jsonl
echo "__ACTION_WRITTEN__"
printf '%s\n' '{"version":2,"client_token":"$ClientToken","payload":{"text":"hello-from-agent-http-demo"}}' >> contacts/zhangsan/send_message.act
echo "__EVENT_TAIL__"
tail -n 20 _stream/events.evt.jsonl | grep "$ClientToken" || true
"@
    Write-Utf8TextFile -Path $Path -Content $demoScript
}

function Invoke-ClawDemoPrompt {
    param(
        [string]$AppRoot,
        [string]$Prompt,
        [int]$MaxAttempts = 2,
        [int]$RetryDelaySec = 5
    )

    $demoScriptPath = Join-Path $AppRoot ".ci-http-demo.sh"
    $lastResult = $null
    for ($attempt = 1; $attempt -le $MaxAttempts; $attempt++) {
        $clientToken = "agent-http-demo-$attempt"
        Write-DemoScript -Path $demoScriptPath -ClientToken $clientToken

        Push-Location $AppRoot
        try {
            $lastResult = Invoke-LoggedCommandResult -Name "claw-demo" -FilePath $script:ClawExe -ArgumentList @(
                "--dangerously-skip-permissions",
                "--output-format", "json",
                "--allowedTools", "bash",
                "prompt",
                $Prompt
            ) -WorkingDirectory $AppRoot
        } finally {
            Pop-Location
        }

        if ($lastResult.ExitCode -eq 0) {
            if ($lastResult.Text) {
                Write-Host $lastResult.Text
            }

            return [pscustomobject]@{
                ClientToken = $clientToken
                ResponseText = $lastResult.Text
                Attempts = $attempt
            }
        }

        $failureText = if (Test-Path $lastResult.LogPath) {
            Get-Content $lastResult.LogPath -Raw -ErrorAction SilentlyContinue
        } else {
            $lastResult.Text
        }

        if ($attempt -lt $MaxAttempts -and (Test-IsTransientClawDemoFailure -Text $failureText)) {
            $attemptLogPath = Join-Path $script:LogDir ("claw-demo.attempt-{0}.log" -f $attempt)
            if (Test-Path $lastResult.LogPath) {
                Copy-Item -Path $lastResult.LogPath -Destination $attemptLogPath -Force
            }
            Write-WarningLine "claw-demo hit a transient Anthropic/API failure on attempt $attempt/$MaxAttempts; retrying in ${RetryDelaySec}s."
            Start-Sleep -Seconds $RetryDelaySec
            continue
        }

        Fail-WithContext "claw-demo failed with exit code $($lastResult.ExitCode)"
    }

    Fail-WithContext "claw-demo exhausted $MaxAttempts attempts after transient Anthropic/API failures"
}

function Main {
    Require-Command cargo
    Require-Command python

    if ([string]::IsNullOrWhiteSpace($env:ANTHROPIC_API_KEY)) {
        throw "ANTHROPIC_API_KEY is required for the HTTP demo integration smoke test"
    }

    Clear-WindowsIntegrationExecutableTargets -ExecutablePaths @(
        $script:AppfsExe,
        $script:ClawExe
    )
    Cleanup-StaleTempArtifacts
    [void][System.IO.Directory]::CreateDirectory($script:LogDir)
    [void][System.IO.Directory]::CreateDirectory($script:CargoCacheRoot)
    Build-TestBinaries
    Stage-TestBinaries

    $mountParent = Split-Path -Parent $MountPoint
    if ($mountParent -and !(Test-Path $mountParent)) {
        New-Item -ItemType Directory -Path $mountParent -Force | Out-Null
    }

    if (Test-Path $MountPoint) {
        Remove-TestPath -Path $MountPoint -Recurse
    }
    Remove-TestPath -Path $script:DbPath
    Remove-TestPath -Path "$($script:DbPath)-shm"
    Remove-TestPath -Path "$($script:DbPath)-wal"

    Write-Section "Start HTTP Bridge"
    $bridgeUri = [Uri]$HttpEndpoint
    $bridgeHost = $bridgeUri.Host
    $bridgePort = $bridgeUri.Port
    $bridgeCommand = "set `"APPFS_BRIDGE_HOST=$bridgeHost`" && set `"APPFS_BRIDGE_PORT=$bridgePort`" && python -u bridge_server.py"
    $script:BridgeHandle = New-LogHandle -Name "bridge" -FilePath "cmd.exe" -ArgumentList @("/c", $bridgeCommand) -WorkingDirectory $script:BridgeDir
    Wait-Until -Description "HTTP bridge startup" -TimeoutSec 20 -Condition {
        Ensure-ProcessRunning $script:BridgeHandle
        try {
            $response = Invoke-RestMethod -Uri "$HttpEndpoint/connector/info" -Method Post -ContentType "application/json" -Body "{}" -ErrorAction Stop
            return $response.app_id -eq $AppId
        } catch {
            return $false
        }
    }
    Write-Success "HTTP bridge is serving at $HttpEndpoint"

    Write-Section "Init AppFS"
    Invoke-LoggedCommand -Name "appfs-init" -FilePath $script:AppfsExe -ArgumentList @(
        "init", $AgentId, "--force"
    ) -WorkingDirectory $script:AppfsCliDir | Out-Null
    Assert-True (Test-Path $script:DbPath) "Created database $script:DbPath"

    Write-Section "Start AppFS"
    $script:AppfsHandle = New-LogHandle -Name "appfs-up" -FilePath $script:AppfsExe -ArgumentList @(
        "appfs", "up", $script:DbPath, $MountPoint,
        "--backend", "winfsp",
        "--auto-unmount"
    ) -WorkingDirectory $script:AppfsCliDir

    $controlDir = Join-Path $MountPoint "_appfs"
    Wait-Until -Description "AppFS mount bootstrap" -TimeoutSec $MountBootstrapTimeoutSec -Condition {
        Ensure-ProcessRunning $script:AppfsHandle
        return (Test-Path (Join-Path $controlDir "register_app.act")) -and
            (Test-Path (Join-Path $controlDir "list_apps.act"))
    }
    Write-Success "AppFS mount is ready"
    $runtimeManifest = Read-AppfsRuntimeManifest -MountRoot $MountPoint
    Assert-True (
        $runtimeManifest.Document.control_plane.register_action -eq "/_appfs/register_app.act"
    ) "AppFS runtime manifest exposes the register action path"

    Write-Section "Register Demo App"
    $registerAction = Join-Path $controlDir "register_app.act"
    $registerPayload = @{
        app_id = $AppId
        transport = @{
            kind = "http"
            endpoint = $HttpEndpoint
            http_timeout_ms = 5000
            grpc_timeout_ms = 5000
            bridge_max_retries = 2
            bridge_initial_backoff_ms = 100
            bridge_max_backoff_ms = 1000
            bridge_circuit_breaker_failures = 5
            bridge_circuit_breaker_cooldown_ms = 3000
        }
        client_token = "reg-http-demo-001"
    } | ConvertTo-Json -Compress
    Append-Utf8JsonLine -Path $registerAction -JsonLine $registerPayload

    $appRoot = Join-Path $MountPoint $AppId
    $snapshotPath = Join-Path $appRoot "chats\chat-001\messages.res.jsonl"
    $eventsPath = Join-Path $appRoot "_stream\events.evt.jsonl"
    $actionPath = Join-Path $appRoot "contacts\zhangsan\send_message.act"

    Wait-Until -Description "registered app tree" -TimeoutSec 20 -Condition {
        Ensure-ProcessRunning $script:AppfsHandle
        return (Test-Path $snapshotPath) -and
            (Test-Path $eventsPath) -and
            (Test-Path $actionPath)
    }
    Write-Success "Registered app tree is visible"

    Wait-Until -Description "initial snapshot content" -TimeoutSec 20 -Condition {
        Ensure-ProcessRunning $script:AppfsHandle
        if (!(Test-Path $snapshotPath)) {
            return $false
        }
        try {
            $snapshotContent = Get-Content $snapshotPath -Raw -ErrorAction Stop
            return $snapshotContent.Contains('"text":"hello"') -or $snapshotContent.Contains("hello")
        } catch {
            return $false
        }
    }
    $snapshotPreview = Get-Content $snapshotPath -TotalCount 3 -ErrorAction Stop | Out-String
    Write-Success "Initial snapshot is readable from the mount"

    Write-Section "Run appfs-agent Demo Prompt"
    $prompt = 'Use bash only. Run `bash ./.ci-http-demo.sh` exactly once. Do not rewrite the script. Return the exact command output.'
    $clawDemoResult = Invoke-ClawDemoPrompt -AppRoot $appRoot -Prompt $prompt
    $clientToken = $clawDemoResult.ClientToken
    $promptResponse = $clawDemoResult.ResponseText

    $promptPayload = ConvertFrom-JsonSafe $promptResponse
    $promptMessage = if ($promptPayload -and $promptPayload.PSObject.Properties.Name -contains "message") {
        [string]$promptPayload.message
    } else {
        $promptResponse
    }
    $toolOutputText = $promptResponse
    if (
        $promptPayload -and
        $promptPayload.PSObject.Properties.Name -contains "tool_results" -and
        $promptPayload.tool_results -and
        $promptPayload.tool_results.Count -gt 0
    ) {
        $toolOutputRaw = [string]$promptPayload.tool_results[0].output
        $toolOutputPayload = ConvertFrom-JsonSafe $toolOutputRaw
        if ($toolOutputPayload -and $toolOutputPayload.PSObject.Properties.Name -contains "stdout") {
            $toolOutputText = [string]$toolOutputPayload.stdout
        } else {
            $toolOutputText = $toolOutputRaw
        }
    }

    $snapshotContentSeen =
        $toolOutputText.Contains('"text":"hello"') -or
        $promptMessage.Contains('"text":"hello"') -or
        $promptResponse.Contains('\"text\":\"hello\"')

    Assert-True ($promptMessage.Contains("__PWD__") -or $toolOutputText.Contains("__PWD__")) "Prompt ran the scripted bash command"
    Assert-True ($toolOutputText.Contains("/c/mnt/appfs-agent-http-demo/aiim")) "Prompt ran inside the mounted AppFS tree"
    Assert-True ($toolOutputText.Contains("__SNAPSHOT__") -or $promptMessage.Contains("__SNAPSHOT__")) "Prompt surfaced snapshot command output"
    Assert-True ($snapshotContentSeen) "Prompt returned mounted snapshot content"
    Assert-True ($toolOutputText.Contains("__ACTION_WRITTEN__") -or $promptMessage.Contains("__ACTION_WRITTEN__")) "Prompt executed the action append step"

    Wait-Until -Description "agent-submitted client token in mounted event stream" -TimeoutSec 30 -Condition {
        Ensure-ProcessRunning $script:AppfsHandle
        if (!(Test-Path $eventsPath)) {
            return $false
        }
        try {
            return (Read-RecentTextFile -Path $eventsPath).Contains($clientToken)
        } catch {
            return $false
        }
    }
    Write-Success "Mounted app event stream contains the agent-submitted client token"

    Wait-Until -Description "action.completed event in mounted event stream" -TimeoutSec 30 -Condition {
        Ensure-ProcessRunning $script:AppfsHandle
        if (!(Test-Path $eventsPath)) {
            return $false
        }
        try {
            return (Read-RecentTextFile -Path $eventsPath).Contains('"type":"action.completed"')
        } catch {
            return $false
        }
    }
    Write-Success "Mounted app event stream contains an action.completed event"

    Write-Host "`nAppFS + appfs-agent HTTP demo integration smoke test passed." -ForegroundColor Green
}

try {
    Main
} finally {
    Cleanup-TestArtifacts
}

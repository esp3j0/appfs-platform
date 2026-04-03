# AppFS + appfs-agent Windows HTTP demo integration smoke test
# Covers: http bridge + app registration + snapshot read + action submit via appfs-agent

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
$script:BridgeDir = Join-Path $script:RepoRoot "appfs\examples\appfs\http-bridge\python"
$script:DbPath = Join-Path $script:AppfsCliDir ".agentfs\$AgentId.db"
$script:AppfsHandle = $null
$script:BridgeHandle = $null
$script:Utf8NoBom = New-Object System.Text.UTF8Encoding($false)
$script:LogDir = Join-Path ([System.IO.Path]::GetTempPath()) ("appfs-agent-http-demo-{0}-{1}" -f $AgentId, ([guid]::NewGuid().ToString("N")))
$script:HadFailure = $false
$script:AppfsExe = Join-Path $script:AppfsCliDir "target\debug\agentfs.exe"
$script:ClawExe = Join-Path $script:AppfsAgentRustDir "target\debug\claw.exe"

function Write-Success { Write-Host "✓ $args" -ForegroundColor Green }
function Write-Fail { Write-Host "✗ $args" -ForegroundColor Red }
function Write-WarningLine { Write-Host "⚠ $args" -ForegroundColor Yellow }
function Write-Section { Write-Host "`n==== $args ====" -ForegroundColor Magenta }

function Remove-TestPath {
    param(
        [string]$Path,
        [switch]$Recurse
    )

    if (!(Test-Path $Path)) {
        return
    }

    try {
        Remove-Item -Path $Path -Force -Recurse:$Recurse -ErrorAction Stop
    } catch {
        if (Test-Path $Path -PathType Container) {
            if ($Recurse) {
                cmd /c "rmdir /s /q `"$Path`"" | Out-Null
            } else {
                cmd /c "rmdir `"$Path`"" | Out-Null
            }
        } elseif (Test-Path $Path -PathType Leaf) {
            cmd /c "del /f /q `"$Path`"" | Out-Null
        }
    }
}

function Stop-LoggedProcess {
    param($Handle)

    if ($null -eq $Handle) {
        return
    }

    if ($Handle.Process -and !$Handle.Process.HasExited) {
        try {
            Stop-Process -Id $Handle.Process.Id -Force -ErrorAction Stop
            $null = $Handle.Process.WaitForExit(5000)
        } catch {
            Write-WarningLine "Failed to stop $($Handle.Name): $_"
        }
    }
}

function Cleanup-TestArtifacts {
    Stop-LoggedProcess $script:AppfsHandle
    Stop-LoggedProcess $script:BridgeHandle

    if (!$SkipCleanup) {
        Remove-TestPath -Path $MountPoint -Recurse
        Remove-TestPath -Path $script:DbPath
        Remove-TestPath -Path "$($script:DbPath)-shm"
        Remove-TestPath -Path "$($script:DbPath)-wal"
    }

    if (!$KeepLogs -and !$script:HadFailure -and (Test-Path $script:LogDir)) {
        Remove-TestPath -Path $script:LogDir -Recurse
    }
}

function Fail-WithContext {
    param([string]$Message)

    $script:HadFailure = $true
    Write-Fail $Message
    if (Test-Path $script:LogDir) {
        Write-Host "`nLogs preserved at $script:LogDir" -ForegroundColor Gray
        foreach ($path in @(
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

function Invoke-LoggedCommand {
    param(
        [string]$Name,
        [string]$FilePath,
        [string[]]$ArgumentList,
        [string]$WorkingDirectory
    )

    $logPath = Join-Path $script:LogDir "$Name.log"
    $stdoutPath = Join-Path $script:LogDir "$Name.stdout.tmp.log"
    $stderrPath = Join-Path $script:LogDir "$Name.stderr.tmp.log"
    $process = Start-Process -FilePath $FilePath `
        -ArgumentList $ArgumentList `
        -WorkingDirectory $WorkingDirectory `
        -PassThru `
        -WindowStyle Hidden `
        -Wait `
        -RedirectStandardOutput $stdoutPath `
        -RedirectStandardError $stderrPath

    $stdout = if (Test-Path $stdoutPath) { Get-Content $stdoutPath -Raw } else { "" }
    $stderr = if (Test-Path $stderrPath) { Get-Content $stderrPath -Raw } else { "" }
    $text = ($stdout + $stderr).TrimEnd()
    [System.IO.File]::WriteAllText($logPath, $text + [Environment]::NewLine, $script:Utf8NoBom)

    Remove-TestPath -Path $stdoutPath
    Remove-TestPath -Path $stderrPath

    if ($process.ExitCode -ne 0) {
        Fail-WithContext "$Name failed with exit code $($process.ExitCode)"
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

    Invoke-LoggedCommand -Name "appfs-build" -FilePath "cargo" -ArgumentList @(
        "build",
        "--bin", "agentfs"
    ) -WorkingDirectory $script:AppfsCliDir | Out-Null
    Assert-True (Test-Path $script:AppfsExe) "Built AppFS CLI binary $script:AppfsExe"

    Invoke-LoggedCommand -Name "claw-build" -FilePath "cargo" -ArgumentList @(
        "build",
        "--manifest-path", (Join-Path $script:AppfsAgentRustDir "Cargo.toml"),
        "-p", "rusty-claude-cli"
    ) -WorkingDirectory $script:AppfsAgentRustDir | Out-Null
    Assert-True (Test-Path $script:ClawExe) "Built appfs-agent CLI binary $script:ClawExe"
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

function Main {
    Require-Command cargo
    Require-Command python

    if ([string]::IsNullOrWhiteSpace($env:ANTHROPIC_API_KEY)) {
        throw "ANTHROPIC_API_KEY is required for the HTTP demo integration smoke test"
    }

    [void][System.IO.Directory]::CreateDirectory($script:LogDir)
    Build-TestBinaries

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
        "--backend", "winfsp"
    ) -WorkingDirectory $script:AppfsCliDir

    $controlDir = Join-Path $MountPoint "_appfs"
    Wait-Until -Description "AppFS mount bootstrap" -TimeoutSec $MountBootstrapTimeoutSec -Condition {
        Ensure-ProcessRunning $script:AppfsHandle
        return (Test-Path (Join-Path $controlDir "register_app.act")) -and
            (Test-Path (Join-Path $controlDir "list_apps.act"))
    }
    Write-Success "AppFS mount is ready"

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
        client_token = "reg-http-demo-001" # @I-am-sure-its-safe
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
    $clientToken = "agent-http-demo-001" # @I-am-sure-its-safe
    $prompt = "Use bash only. Run these commands in order and show their outputs: pwd ; head -n 3 chats/chat-001/messages.res.jsonl ; printf '{""version"":2,""client_token"":""$clientToken"",""payload"":{""text"":""hello-from-agent-http-demo""}}`n' >> contacts/zhangsan/send_message.act ; tail -n 20 _stream/events.evt.jsonl | grep $clientToken"
    Push-Location $appRoot
    try {
        $promptOutput = Invoke-LoggedCommand -Name "claw-demo" -FilePath $script:ClawExe -ArgumentList @(
            "--dangerously-skip-permissions",
            "--allowedTools", "bash",
            "prompt",
            $prompt
        ) -WorkingDirectory $appRoot
    } finally {
        Pop-Location
    }

    Assert-True ($promptOutput.Contains("/c/mnt/")) "Prompt ran inside the mounted AppFS tree"
    Assert-True ($promptOutput.Contains("messages.res.jsonl")) "Prompt surfaced snapshot command output"
    Assert-True ($promptOutput.Contains($clientToken)) "Prompt surfaced the action event token"

    $eventMatch = Select-String -Path $eventsPath -Pattern $clientToken -SimpleMatch | Select-Object -Last 1
    Assert-True ($null -ne $eventMatch) "Mounted app event stream contains the agent-submitted client token"
    Assert-True (($eventMatch.Line -like '*"action.completed"*') -or ((Get-Content $eventsPath -Raw).Contains('"type":"action.completed"'))) "Mounted app event stream contains an action.completed event"

    Write-Host "`nAppFS + appfs-agent HTTP demo integration smoke test passed." -ForegroundColor Green
}

try {
    Main
} finally {
    Cleanup-TestArtifacts
}

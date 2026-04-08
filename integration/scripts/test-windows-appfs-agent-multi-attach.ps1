# AppFS + appfs-agent Windows multi-attach test
# Covers: appfs init + appfs up + shared runtime manifest + two independent appfs-agent attach flows
# Contract checkpoint: IC-2 in integration/APPFS-appfs-agent-attach-contract-v1.1.md

param(
    [string]$AgentId = "appfs-agent-multi-attach",
    [string]$MountPoint = "C:\mnt\appfs-agent-multi-attach",
    [string]$WorkspaceName = "workspace",
    [string]$AgentAAttachId = "agent-a",
    [string]$AgentBAttachId = "agent-b",
    [string]$AgentARole = "planner",
    [string]$AgentBRole = "reviewer",
    [int]$MountBootstrapTimeoutSec = 180,
    [switch]$SkipCleanup,
    [switch]$KeepLogs
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$script:RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..\..")
$script:AppfsCliDir = Join-Path $script:RepoRoot "appfs\cli"
$script:AppfsAgentRustDir = Join-Path $script:RepoRoot "appfs-agent\rust"
$script:DbPath = Join-Path $script:AppfsCliDir ".agentfs\$AgentId.db"
$script:AppfsHandle = $null
$script:Utf8NoBom = New-Object System.Text.UTF8Encoding($false)
$script:LogDir = Join-Path ([System.IO.Path]::GetTempPath()) ("appfs-agent-multi-attach-{0}-{1}" -f $AgentId, ([guid]::NewGuid().ToString("N")))
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

function Cleanup-StaleTempArtifacts {
    $tempRoot = [System.IO.Path]::GetTempPath()
    foreach ($pattern in @(
        "appfs-agent-smoke-*",
        "appfs-agent-http-demo-*",
        "appfs-agent-multi-attach-*",
        "appfs-agent-launcher-*"
    )) {
        Get-ChildItem -Path $tempRoot -Directory -Filter $pattern -ErrorAction SilentlyContinue |
            Where-Object { $_.FullName -ne $script:LogDir } |
            ForEach-Object { Remove-TestPath -Path $_.FullName -Recurse }
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
            (Join-Path $script:LogDir "appfs-build.log"),
            (Join-Path $script:LogDir "claw-build.log"),
            (Join-Path $script:LogDir "appfs-up.stdout.log"),
            (Join-Path $script:LogDir "appfs-up.stderr.log"),
            (Join-Path $script:LogDir "claw-status-agent-a.log"),
            (Join-Path $script:LogDir "claw-status-agent-b.log")
        )) {
            if (Test-Path $path) {
                Write-Host "`n--- tail: $path ---" -ForegroundColor Gray
                Get-Content $path -Tail 40 -ErrorAction SilentlyContinue
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

function Invoke-LoggedCommand {
    param(
        [string]$Name,
        [string]$FilePath,
        [string[]]$ArgumentList,
        [string]$WorkingDirectory,
        [hashtable]$EnvironmentOverrides = @{}
    )

    $logPath = Join-Path $script:LogDir "$Name.log"
    $stdoutPath = Join-Path $script:LogDir "$Name.stdout.tmp.log"
    $stderrPath = Join-Path $script:LogDir "$Name.stderr.tmp.log"
    $exitCode = 0
    $savedEnvironment = @{}

    foreach ($entry in $EnvironmentOverrides.GetEnumerator()) {
        $key = [string]$entry.Key
        $savedEnvironment[$key] = [System.Environment]::GetEnvironmentVariable($key, "Process")
        [System.Environment]::SetEnvironmentVariable($key, [string]$entry.Value, "Process")
    }

    Push-Location $WorkingDirectory
    try {
        $previousErrorActionPreference = $ErrorActionPreference
        $ErrorActionPreference = "Continue"
        & $FilePath @ArgumentList 1> $stdoutPath 2> $stderrPath
        if ($LASTEXITCODE -is [int]) {
            $exitCode = $LASTEXITCODE
        }
    } finally {
        $ErrorActionPreference = $previousErrorActionPreference
        Pop-Location
        foreach ($key in $savedEnvironment.Keys) {
            [System.Environment]::SetEnvironmentVariable($key, $savedEnvironment[$key], "Process")
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
    return [pscustomobject]@{
        Stdout = $stdout
        Stderr = $stderr
        Text = $text
        LogPath = $logPath
    }
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

    Invoke-LoggedCommand -Name "appfs-build" -FilePath "cargo" -ArgumentList @(
        "build",
        "--target-dir", $script:AppfsCargoTargetDir,
        "--bin", "agentfs"
    ) -WorkingDirectory $script:AppfsCliDir | Out-Null
    Assert-True (Test-Path $script:AppfsExe) "Built AppFS CLI binary $script:AppfsExe"

    Invoke-LoggedCommand -Name "claw-build" -FilePath "cargo" -ArgumentList @(
        "build",
        "--target-dir", $script:ClawCargoTargetDir,
        "--manifest-path", (Join-Path $script:AppfsAgentRustDir "Cargo.toml"),
        "-p", "rusty-claude-cli"
    ) -WorkingDirectory $script:AppfsAgentRustDir | Out-Null
    Assert-True (Test-Path $script:ClawExe) "Built appfs-agent CLI binary $script:ClawExe"
}

function New-AttachEnvironment {
    param(
        [pscustomobject]$RuntimeManifest,
        [string]$AttachId,
        [string]$AttachRole
    )

    $envMap = @{
        APPFS_ATTACH_SCHEMA = "1"
        APPFS_RUNTIME_MANIFEST = [string]$RuntimeManifest.Path
        APPFS_MOUNT_ROOT = [string]$MountPoint
        APPFS_RUNTIME_SESSION_ID = [string]$RuntimeManifest.Document.runtime_session_id
        APPFS_ATTACH_ID = $AttachId
    }

    if (-not [string]::IsNullOrWhiteSpace($AttachRole)) {
        $envMap["APPFS_AGENT_ROLE"] = $AttachRole
    }

    return $envMap
}

function Read-AgentStatusJson {
    param(
        [string]$Name,
        [string]$WorkspaceDir,
        [hashtable]$AttachEnvironment
    )

    $commandResult = Invoke-LoggedCommand -Name $Name -FilePath $script:ClawExe -ArgumentList @(
        "--output-format", "json",
        "status"
    ) -WorkingDirectory $WorkspaceDir -EnvironmentOverrides $AttachEnvironment

    try {
        return $commandResult.Stdout | ConvertFrom-Json -ErrorAction Stop
    } catch {
        Fail-WithContext "$Name did not emit valid JSON status output"
    }
}

function Get-JsonWarnings {
    param($StatusJson)

    if ($null -eq $StatusJson.appfs) {
        return @()
    }
    if ($null -eq $StatusJson.appfs.warnings) {
        return @()
    }
    return @($StatusJson.appfs.warnings)
}

function Assert-AttachStatusMatches {
    param(
        $StatusJson,
        [string]$ExpectedMountRoot,
        [string]$ExpectedRuntimeSessionId,
        [string]$ExpectedAttachId,
        [string]$ExpectedAttachRole,
        [string]$AgentLabel
    )

    Assert-True ($StatusJson.appfs.detected -eq $true) "$AgentLabel detected AppFS"
    Assert-True ($StatusJson.appfs.attach_source -eq "env") "$AgentLabel resolved attach from env"
    Assert-True ($StatusJson.appfs.mount_root -eq $ExpectedMountRoot) "$AgentLabel reported the shared mount root"
    Assert-True (
        $StatusJson.appfs.runtime_session_id -eq $ExpectedRuntimeSessionId
    ) "$AgentLabel reported the shared runtime session"
    Assert-True ($StatusJson.appfs.attach_id -eq $ExpectedAttachId) "$AgentLabel kept its expected attach_id"
    Assert-True (
        $StatusJson.appfs.multi_agent_mode -eq "shared_mount_distinct_attach"
    ) "$AgentLabel reported shared multi-agent attach mode"
    if (-not [string]::IsNullOrWhiteSpace($ExpectedAttachRole)) {
        Assert-True ($StatusJson.appfs.attach_role -eq $ExpectedAttachRole) "$AgentLabel kept its expected attach role"
    }
    Assert-True ((Get-JsonWarnings $StatusJson).Count -eq 0) "$AgentLabel reported no AppFS attach warnings"
}

function Main {
    Require-Command cargo

    Cleanup-StaleTempArtifacts
    [void][System.IO.Directory]::CreateDirectory($script:LogDir)
    [void][System.IO.Directory]::CreateDirectory($script:CargoCacheRoot)
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
    $workspaceDir = Join-Path $MountPoint $WorkspaceName
    $helloPath = Join-Path $workspaceDir "hello.txt"

    Wait-Until -Description "AppFS mount bootstrap" -TimeoutSec $MountBootstrapTimeoutSec -Condition {
        Ensure-ProcessRunning $script:AppfsHandle
        return (Test-Path (Join-Path $controlDir "register_app.act")) -and
            (Test-Path (Join-Path $controlDir "list_apps.act"))
    }
    Write-Success "AppFS mount is ready"
    $runtimeManifest = Read-AppfsRuntimeManifest -MountRoot $MountPoint

    Write-Section "Prepare Shared Workspace"
    New-Item -ItemType Directory -Path $workspaceDir -Force | Out-Null
    [System.IO.File]::WriteAllText($helloPath, "hello from appfs shared runtime`n", $script:Utf8NoBom)
    Assert-True (Test-Path $helloPath) "Created mounted workspace file $helloPath"

    Write-Section "Run Multi-Agent Attach Status"
    $agentAEnvironment = New-AttachEnvironment -RuntimeManifest $runtimeManifest -AttachId $AgentAAttachId -AttachRole $AgentARole
    $agentBEnvironment = New-AttachEnvironment -RuntimeManifest $runtimeManifest -AttachId $AgentBAttachId -AttachRole $AgentBRole

    $statusAgentA = Read-AgentStatusJson -Name "claw-status-agent-a" -WorkspaceDir $workspaceDir -AttachEnvironment $agentAEnvironment
    $statusAgentB = Read-AgentStatusJson -Name "claw-status-agent-b" -WorkspaceDir $workspaceDir -AttachEnvironment $agentBEnvironment

    Assert-AttachStatusMatches -StatusJson $statusAgentA `
        -ExpectedMountRoot $MountPoint `
        -ExpectedRuntimeSessionId ([string]$runtimeManifest.Document.runtime_session_id) `
        -ExpectedAttachId $AgentAAttachId `
        -ExpectedAttachRole $AgentARole `
        -AgentLabel "agent A"

    Assert-AttachStatusMatches -StatusJson $statusAgentB `
        -ExpectedMountRoot $MountPoint `
        -ExpectedRuntimeSessionId ([string]$runtimeManifest.Document.runtime_session_id) `
        -ExpectedAttachId $AgentBAttachId `
        -ExpectedAttachRole $AgentBRole `
        -AgentLabel "agent B"

    Assert-True (
        $statusAgentA.appfs.runtime_session_id -eq $statusAgentB.appfs.runtime_session_id
    ) "Both agents share the same runtime_session_id"
    Assert-True (
        $statusAgentA.appfs.attach_id -ne $statusAgentB.appfs.attach_id
    ) "Both agents keep distinct attach_id values"
    Assert-True (
        $statusAgentA.appfs.mount_root -eq $statusAgentB.appfs.mount_root
    ) "Both agents report the same mount root"

    Write-Host "`nAppFS + appfs-agent Windows multi-attach test passed." -ForegroundColor Green
}

try {
    Main
} finally {
    Cleanup-TestArtifacts
}

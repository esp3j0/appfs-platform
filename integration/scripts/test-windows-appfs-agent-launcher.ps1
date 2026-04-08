# AppFS + appfs-agent Windows launcher test
# Covers: appfs init + appfs launch + explicit attach env injection + mounted workspace cwd
# Contract checkpoint: IC-3 in integration/APPFS-appfs-agent-attach-contract-v1.1.md

param(
    [string]$AgentId = "appfs-agent-launcher",
    [string]$MountPoint = "C:\mnt\appfs-agent-launcher",
    [string]$WorkspaceName = "workspace",
    [string]$AttachId = "agent-launcher",
    [string]$AttachRole = "planner",
    [int]$StartupTimeoutMs = 15000,
    [string]$AppfsExePath = "",
    [string]$ClawExePath = "",
    [switch]$SkipBuild,
    [switch]$SkipCleanup,
    [switch]$KeepLogs
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$script:RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..\..")
$script:AppfsCliDir = Join-Path $script:RepoRoot "appfs\cli"
$script:AppfsAgentRustDir = Join-Path $script:RepoRoot "appfs-agent\rust"
$script:DbPath = Join-Path $script:AppfsCliDir ".agentfs\$AgentId.db"
$script:Utf8NoBom = New-Object System.Text.UTF8Encoding($false)
$script:LogDir = Join-Path ([System.IO.Path]::GetTempPath()) ("appfs-agent-launcher-{0}-{1}" -f $AgentId, ([guid]::NewGuid().ToString("N")))
$script:HadFailure = $false
$script:CargoCacheRoot = Join-Path ([System.IO.Path]::GetTempPath()) "appfs-platform-cargo-targets"
$script:AppfsCargoTargetDir = Join-Path $script:CargoCacheRoot "appfs-cli"
$script:ClawCargoTargetDir = Join-Path $script:CargoCacheRoot "appfs-agent-rust"
$script:AppfsExe = ""
$script:ClawExe = ""

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
    foreach ($pattern in @("appfs-agent-launcher-*")) {
        Get-ChildItem -Path $tempRoot -Directory -Filter $pattern -ErrorAction SilentlyContinue |
            Where-Object { $_.FullName -ne $script:LogDir } |
            ForEach-Object { Remove-TestPath -Path $_.FullName -Recurse }
    }
}

function Cleanup-TestArtifacts {
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
            (Join-Path $script:LogDir "appfs-launch.log"),
            (Join-Path $script:LogDir "appfs-build.log"),
            (Join-Path $script:LogDir "claw-build.log")
        )) {
            if (Test-Path $path) {
                Write-Host "`n--- tail: $path ---" -ForegroundColor Gray
                Get-Content $path -Tail 60 -ErrorAction SilentlyContinue
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
    $exitCode = 0

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

function Resolve-ExecutablePath {
    param(
        [string]$ExplicitPath,
        [string]$FallbackPath,
        [string]$Description
    )

    $candidate = if ([string]::IsNullOrWhiteSpace($ExplicitPath)) {
        $FallbackPath
    } else {
        $ExplicitPath
    }

    $resolved = Resolve-Path $candidate -ErrorAction Stop
    Assert-True (Test-Path $resolved.Path -PathType Leaf) "$Description exists at $($resolved.Path)"
    return $resolved.Path
}

function Initialize-BinaryPaths {
    if ($SkipBuild) {
        $script:AppfsExe = Resolve-ExecutablePath `
            -ExplicitPath $AppfsExePath `
            -FallbackPath (Join-Path $script:AppfsCargoTargetDir "debug\agentfs.exe") `
            -Description "Existing AppFS CLI binary"
        $script:ClawExe = Resolve-ExecutablePath `
            -ExplicitPath $ClawExePath `
            -FallbackPath (Join-Path $script:ClawCargoTargetDir "debug\claw.exe") `
            -Description "Existing appfs-agent CLI binary"
        Write-Success "Reusing existing binaries for launcher validation"
        return
    }

    if (-not [string]::IsNullOrWhiteSpace($AppfsExePath) -or -not [string]::IsNullOrWhiteSpace($ClawExePath)) {
        Fail-WithContext "Use -SkipBuild when supplying -AppfsExePath or -ClawExePath"
    }

    Require-Command cargo
    $script:AppfsExe = Join-Path $script:AppfsCargoTargetDir "debug\agentfs.exe"
    $script:ClawExe = Join-Path $script:ClawCargoTargetDir "debug\claw.exe"
    Build-TestBinaries
}

function Build-TestBinaries {
    Write-Section "Build Test Binaries"
    Initialize-WindowsRustBuildEnv

    Invoke-WithWindowsIntegrationBuildLock {
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
}

function Normalize-PathString {
    param([string]$Value)

    if ([string]::IsNullOrWhiteSpace($Value)) {
        return $Value
    }
    return ($Value -replace '/', '\').TrimEnd('\')
}

function Read-LauncherStatusJson {
    param(
        [string]$WorkspaceRelativePath
    )

    $commandResult = Invoke-LoggedCommand -Name "appfs-launch" -FilePath $script:AppfsExe -ArgumentList @(
        "appfs", "launch", $script:DbPath, $MountPoint,
        "--agent-bin", $script:ClawExe,
        "--backend", "winfsp",
        "--workspace", $WorkspaceRelativePath,
        "--attach-id", $AttachId,
        "--attach-role", $AttachRole,
        "--startup-timeout-ms", $StartupTimeoutMs,
        "--",
        "status",
        "--output-format",
        "json"
    ) -WorkingDirectory $script:AppfsCliDir

    try {
        return [pscustomobject]@{
            Json = ($commandResult.Stdout | ConvertFrom-Json -ErrorAction Stop)
            Raw = $commandResult
        }
    } catch {
        Fail-WithContext "appfs-launch did not emit valid JSON status output from the launched agent"
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

function Main {
    Cleanup-StaleTempArtifacts
    [void][System.IO.Directory]::CreateDirectory($script:LogDir)
    [void][System.IO.Directory]::CreateDirectory($script:CargoCacheRoot)
    Initialize-BinaryPaths

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

    Write-Section "Run Launcher"
    $statusResult = Read-LauncherStatusJson -WorkspaceRelativePath $WorkspaceName
    $statusJson = $statusResult.Json
    $expectedWorkspaceDir = Join-Path $MountPoint $WorkspaceName
    $expectedManifestPath = Join-Path $MountPoint ".well-known\appfs\runtime.json"

    Assert-True ($statusJson.kind -eq "status") "Launched child returned status payload"
    Assert-True ($statusJson.appfs.detected -eq $true) "Launched child detected AppFS"
    Assert-True ($statusJson.appfs.attach_source -eq "env") "Launcher attached the child through explicit env"
    Assert-True (
        (Normalize-PathString ([string]$statusJson.appfs.mount_root)) -eq (Normalize-PathString $MountPoint)
    ) "Launched child reported the shared AppFS mount root"
    Assert-True (
        -not [string]::IsNullOrWhiteSpace([string]$statusJson.appfs.runtime_session_id)
    ) "Launched child reported a runtime_session_id"
    Assert-True (
        $statusJson.appfs.attach_id -eq $AttachId
    ) "Launched child kept the expected attach_id"
    Assert-True (
        $statusJson.appfs.attach_role -eq $AttachRole
    ) "Launched child kept the expected attach role"
    Assert-True (
        $statusJson.appfs.multi_agent_mode -eq "shared_mount_distinct_attach"
    ) "Launched child reported shared multi-agent attach mode"
    Assert-True (
        (Normalize-PathString ([string]$statusJson.appfs.manifest_path)) -eq (Normalize-PathString $expectedManifestPath)
    ) "Launched child surfaced the expected runtime manifest path"
    Assert-True (
        (Normalize-PathString ([string]$statusJson.workspace.cwd)) -eq (Normalize-PathString $expectedWorkspaceDir)
    ) "Launched child ran inside the mounted AppFS workspace"
    Assert-True (
        @((Get-JsonWarnings $statusJson)).Count -eq 0
    ) "Launched child reported no AppFS attach warnings"

    Assert-True (
        $statusResult.Raw.Text.Contains("Mounted at $MountPoint")
    ) "Launcher brought up AppFS before running the child agent"

    Write-Host "`nAppFS + appfs-agent Windows launcher test passed." -ForegroundColor Green
}

try {
    Main
} finally {
    Cleanup-TestArtifacts
}

# AppFS + appfs-agent Windows smoke test
# Covers: appfs init + appfs up + mounted workspace + claw status (+ optional prompt)
# Contract checkpoint: IC-0 in integration/APPFS-appfs-agent-attach-contract-v1.1.md

param(
    [string]$AgentId = "appfs-agent-smoke",
    [string]$MountPoint = "C:\mnt\appfs-agent-smoke",
    [string]$WorkspaceName = "workspace",
    [int]$MountBootstrapTimeoutSec = 180,
    [switch]$SkipCleanup,
    [switch]$KeepLogs,
    [switch]$RunPrompt
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$script:RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..\..")
$script:AppfsCliDir = Join-Path $script:RepoRoot "appfs\cli"
$script:AppfsAgentRustDir = Join-Path $script:RepoRoot "appfs-agent\rust"
$script:DbPath = Join-Path $script:AppfsCliDir ".agentfs\$AgentId.db"
$script:AppfsHandle = $null
$script:Utf8NoBom = New-Object System.Text.UTF8Encoding($false)
$script:LogDir = Join-Path ([System.IO.Path]::GetTempPath()) ("appfs-agent-smoke-{0}-{1}" -f $AgentId, ([guid]::NewGuid().ToString("N")))
$script:RuntimeBinDir = Join-Path $script:LogDir "bin"
$script:HadFailure = $false
$script:CargoCacheRoot = Join-Path ([System.IO.Path]::GetTempPath()) "appfs-platform-cargo-targets"
$script:AppfsCargoTargetDir = Join-Path $script:CargoCacheRoot "appfs-cli"
$script:ClawCargoTargetDir = Join-Path $script:CargoCacheRoot "appfs-agent-rust"
$script:AppfsExe = Join-Path $script:AppfsCargoTargetDir "debug\agentfs.exe"
$script:ClawExe = Join-Path $script:ClawCargoTargetDir "debug\claw.exe"

. (Join-Path $PSScriptRoot "windows-rust-build-env.ps1")

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
                cmd /c "rmdir /s /q `"$Path`" 2>nul || exit /b 0" | Out-Null
            } else {
                cmd /c "rmdir `"$Path`" 2>nul || exit /b 0" | Out-Null
            }
        } elseif (Test-Path $Path -PathType Leaf) {
            cmd /c "del /f /q `"$Path`" 2>nul || exit /b 0" | Out-Null
        }
    }
}

function Stop-LoggedProcess {
    param($Handle)

    if ($null -eq $Handle) {
        return
    }

    if ($Handle.Process) {
        try {
            if (!$Handle.Process.HasExited) {
                try {
                    Stop-Process -Id $Handle.Process.Id -Force -ErrorAction Stop
                } catch {
                    Write-WarningLine "Failed to stop $($Handle.Name): $_"
                }

                if (-not $Handle.Process.WaitForExit(5000)) {
                    try {
                        & taskkill.exe /F /T /PID $Handle.Process.Id | Out-Null
                    } catch {
                    } finally {
                        $global:LASTEXITCODE = 0
                    }
                    $null = $Handle.Process.WaitForExit(5000)
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

function Cleanup-StaleTempArtifacts {
    $tempRoot = [System.IO.Path]::GetTempPath()
    foreach ($pattern in @("appfs-agent-smoke-*")) {
        Get-ChildItem -Path $tempRoot -Directory -Filter $pattern -ErrorAction SilentlyContinue |
            Where-Object { $_.FullName -ne $script:LogDir } |
            ForEach-Object { Remove-TestPath -Path $_.FullName -Recurse }
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
            (Join-Path $script:LogDir "claw-status.log"),
            (Join-Path $script:LogDir "claw-prompt.log")
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

function Main {
    Require-Command cargo

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

    Write-Section "Prepare Workspace"
    New-Item -ItemType Directory -Path $workspaceDir -Force | Out-Null
    [System.IO.File]::WriteAllText($helloPath, "hello from appfs mount`n", $script:Utf8NoBom)
    Assert-True (Test-Path $helloPath) "Created mounted workspace file $helloPath"
    Assert-True ((Get-Content $helloPath -Raw).Contains("hello from appfs mount")) "Mounted hello.txt is readable from PowerShell"

    Write-Section "Run appfs-agent Status"
    Push-Location $workspaceDir
    try {
        $statusOutput = Invoke-LoggedCommand -Name "claw-status" -FilePath $script:ClawExe -ArgumentList @(
            "status"
        ) -WorkingDirectory $workspaceDir
    } finally {
        Pop-Location
    }
    Assert-True ($statusOutput.Contains("Workspace")) "claw status rendered a workspace snapshot inside the mount"
    Assert-True ($statusOutput.Contains($workspaceDir)) "claw status reported the mounted workspace path"
    Assert-True ($statusOutput.Contains("Attach source     manifest")) "claw status attached through the AppFS runtime manifest"
    Assert-True (
        $statusOutput.Contains("Runtime session   $($runtimeManifest.Document.runtime_session_id)")
    ) "claw status reported the shared AppFS runtime session"
    Assert-True (
        $statusOutput.Contains("Multi-agent mode  shared_mount_distinct_attach")
    ) "claw status reported the shared multi-agent attach mode"

    if ($RunPrompt) {
        Write-Section "Run appfs-agent Prompt"
        if ([string]::IsNullOrWhiteSpace($env:ANTHROPIC_API_KEY)) {
            Fail-WithContext "RunPrompt requires ANTHROPIC_API_KEY in the environment"
        }

        Push-Location $workspaceDir
        try {
            $promptOutput = Invoke-LoggedCommand -Name "claw-prompt" -FilePath $script:ClawExe -ArgumentList @(
                "--dangerously-skip-permissions",
                "prompt",
                "Confirm the current working directory, list files in the current directory, and print the exact contents of hello.txt. Do not modify any files."
            ) -WorkingDirectory $workspaceDir
        } finally {
            Pop-Location
        }
        Assert-True ($promptOutput.Contains("hello from appfs mount")) "claw prompt surfaced hello.txt content from the mounted workspace"
    }

    Write-Host "`nAppFS + appfs-agent Windows smoke test passed." -ForegroundColor Green
}

try {
    Main
} finally {
    Cleanup-TestArtifacts
}

function Resolve-MsvcToolsInstallDir {
    if ($env:VCToolsInstallDir) {
        $vcToolsInstallDir = $env:VCToolsInstallDir.TrimEnd("\/")
        if (Test-Path (Join-Path $vcToolsInstallDir "include") -PathType Container) {
            return $vcToolsInstallDir
        }
    }

    $cl = Get-Command cl.exe -ErrorAction SilentlyContinue
    if ($null -ne $cl) {
        $clDir = Split-Path $cl.Source -Parent
        $vcToolsInstallDir = $clDir
        1..3 | ForEach-Object {
            $vcToolsInstallDir = Split-Path $vcToolsInstallDir -Parent
        }

        if (Test-Path (Join-Path $vcToolsInstallDir "include") -PathType Container) {
            return $vcToolsInstallDir
        }
    }

    $vswhere = Get-Command vswhere.exe -ErrorAction SilentlyContinue
    if ($null -eq $vswhere) {
        $vswhereDefaultPath = "C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe"
        if (Test-Path $vswhereDefaultPath -PathType Leaf) {
            $vswhere = Get-Item $vswhereDefaultPath
        }
    }

    if ($null -ne $vswhere) {
        $vswherePath = if ($vswhere.PSObject.Properties.Name -contains "Source") {
            $vswhere.Source
        } else {
            $vswhere.FullName
        }

        $installationPath = & $vswherePath `
            -latest `
            -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
            -property installationPath
        if ($LASTEXITCODE -eq 0 -and -not [string]::IsNullOrWhiteSpace($installationPath)) {
            $msvcRoot = Join-Path $installationPath.Trim() "VC\Tools\MSVC"
            $latestMsvcDir = Get-ChildItem -Path $msvcRoot -Directory -ErrorAction SilentlyContinue |
                Sort-Object Name -Descending |
                Select-Object -First 1 -ExpandProperty FullName
            if (-not [string]::IsNullOrWhiteSpace($latestMsvcDir)) {
                return $latestMsvcDir
            }
        }
    }

    throw "Unable to locate MSVC tools include directory. Install Visual Studio Build Tools or add cl.exe to PATH."
}

function Initialize-WindowsRustBuildEnv {
    $vcToolsInstallDir = Resolve-MsvcToolsInstallDir
    $vcInclude = Join-Path $vcToolsInstallDir "include"
    if (!(Test-Path $vcInclude -PathType Container)) {
        throw "MSVC include directory not found: $vcInclude"
    }

    $sdkIncludeRoot = "C:\Program Files (x86)\Windows Kits\10\Include"
    $sdkIncludeDir = Get-ChildItem -Path $sdkIncludeRoot -Directory -ErrorAction Stop |
        Sort-Object Name -Descending |
        Select-Object -First 1 -ExpandProperty FullName

    if ([string]::IsNullOrWhiteSpace($sdkIncludeDir)) {
        throw "Windows SDK include directory not found under $sdkIncludeRoot"
    }

    $includeDirs = @(
        $vcInclude,
        (Join-Path $sdkIncludeDir "ucrt"),
        (Join-Path $sdkIncludeDir "shared"),
        (Join-Path $sdkIncludeDir "um"),
        (Join-Path $sdkIncludeDir "winrt"),
        (Join-Path $sdkIncludeDir "cppwinrt")
    ) | Where-Object { Test-Path $_ -PathType Container }

    if ($includeDirs.Count -eq 0) {
        throw "Unable to resolve Windows SDK include directories for bindgen"
    }

    $clangArgs = ($includeDirs | ForEach-Object { '-isystem"{0}"' -f $_ }) -join " "
    $env:BINDGEN_EXTRA_CLANG_ARGS = $clangArgs
    $env:BINDGEN_EXTRA_CLANG_ARGS_x86_64_pc_windows_msvc = $clangArgs
    Set-Item "Env:BINDGEN_EXTRA_CLANG_ARGS_x86_64-pc-windows-msvc" $clangArgs

    if ([string]::IsNullOrWhiteSpace($env:LIBCLANG_PATH)) {
        $defaultLibclangPath = "C:\Program Files\LLVM\bin"
        if (Test-Path $defaultLibclangPath -PathType Container) {
            $env:LIBCLANG_PATH = $defaultLibclangPath
        }
    }
}

function Format-WindowsIntegrationElapsed {
    param([TimeSpan]$Elapsed)

    return [string]::Format(
        [System.Globalization.CultureInfo]::InvariantCulture,
        "{0:0.0}s",
        $Elapsed.TotalSeconds
    )
}

function Invoke-WithWindowsIntegrationBuildLock {
    param(
        [scriptblock]$ScriptBlock,
        [int]$TimeoutSec = 1800
    )

    $mutex = $null
    $acquired = $false
    $waitStopwatch = [System.Diagnostics.Stopwatch]::StartNew()
    $holdStopwatch = $null
    try {
        $mutex = New-Object System.Threading.Mutex($false, "Global\appfs-platform-windows-build-cache")
        Write-Host "[info] Waiting for Windows integration build cache lock (timeout ${TimeoutSec}s)" -ForegroundColor DarkGray
        $acquired = $mutex.WaitOne([TimeSpan]::FromSeconds($TimeoutSec))
        $waitStopwatch.Stop()
        if (-not $acquired) {
            throw "Timed out waiting for the Windows integration build cache lock after ${TimeoutSec}s"
        }

        Write-Host (
            "[info] Acquired Windows integration build cache lock after {0}" -f (
                Format-WindowsIntegrationElapsed -Elapsed $waitStopwatch.Elapsed
            )
        ) -ForegroundColor DarkGray
        $holdStopwatch = [System.Diagnostics.Stopwatch]::StartNew()
        & $ScriptBlock
    } finally {
        if ($null -ne $mutex) {
            if ($acquired) {
                try {
                    if ($null -ne $holdStopwatch) {
                        $holdStopwatch.Stop()
                        Write-Host (
                            "[info] Releasing Windows integration build cache lock after holding it for {0}" -f (
                                Format-WindowsIntegrationElapsed -Elapsed $holdStopwatch.Elapsed
                            )
                        ) -ForegroundColor DarkGray
                    }
                    [void]$mutex.ReleaseMutex()
                } catch {
                }
            }
            $mutex.Dispose()
        }
    }
}

function Resolve-NormalizedWindowsPath {
    param([string]$Path)

    if ([string]::IsNullOrWhiteSpace($Path)) {
        return $null
    }

    return [System.IO.Path]::GetFullPath($Path).TrimEnd("\/")
}

function Clear-WindowsIntegrationExecutableTargets {
    param(
        [string[]]$ExecutablePaths,
        [int]$WaitTimeoutMs = 5000
    )

    $normalizedPaths = @(
        $ExecutablePaths |
            Where-Object { -not [string]::IsNullOrWhiteSpace($_) } |
            ForEach-Object { Resolve-NormalizedWindowsPath $_ } |
            Where-Object { -not [string]::IsNullOrWhiteSpace($_) } |
            Select-Object -Unique
    )

    if ($normalizedPaths.Count -eq 0) {
        return
    }

    $targetPathSet = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
    foreach ($path in $normalizedPaths) {
        [void]$targetPathSet.Add($path)
    }

    $processes = @(Get-CimInstance Win32_Process -Filter "name = 'agentfs.exe' OR name = 'claw.exe'" -ErrorAction SilentlyContinue)
    $stopped = $false
    foreach ($process in $processes) {
        $executablePath = Resolve-NormalizedWindowsPath $process.ExecutablePath
        if ([string]::IsNullOrWhiteSpace($executablePath) -or -not $targetPathSet.Contains($executablePath)) {
            continue
        }

        try {
            Stop-Process -Id $process.ProcessId -Force -ErrorAction Stop
            $stopped = $true
            Write-Host "[warn] Stopped stale process $($process.Name) (PID $($process.ProcessId)) using $executablePath" -ForegroundColor Yellow
        } catch {
            Write-Host "[warn] Failed to stop stale process $($process.Name) (PID $($process.ProcessId)): $_" -ForegroundColor Yellow
        }
    }

    if ($stopped) {
        $deadline = (Get-Date).AddMilliseconds($WaitTimeoutMs)
        do {
            Start-Sleep -Milliseconds 200
            $remaining = @(
                Get-CimInstance Win32_Process -Filter "name = 'agentfs.exe' OR name = 'claw.exe'" -ErrorAction SilentlyContinue |
                    Where-Object {
                        $executablePath = Resolve-NormalizedWindowsPath $_.ExecutablePath
                        -not [string]::IsNullOrWhiteSpace($executablePath) -and $targetPathSet.Contains($executablePath)
                    }
            )
        } while ($remaining.Count -gt 0 -and (Get-Date) -lt $deadline)

        foreach ($process in $remaining) {
            try {
                & taskkill.exe /F /T /PID $process.ProcessId | Out-Null
                if ($LASTEXITCODE -eq 0) {
                    Write-Host "[warn] taskkill forced stale process $($process.Name) (PID $($process.ProcessId))" -ForegroundColor Yellow
                } else {
                    Write-Host "[warn] taskkill reported exit code $LASTEXITCODE for stale process $($process.Name) (PID $($process.ProcessId))" -ForegroundColor Yellow
                }
            } catch {
                Write-Host "[warn] taskkill failed for stale process $($process.Name) (PID $($process.ProcessId)): $_" -ForegroundColor Yellow
            } finally {
                $global:LASTEXITCODE = 0
            }
        }
    }

    foreach ($path in $normalizedPaths) {
        if (-not (Test-Path $path -PathType Leaf)) {
            continue
        }

        try {
            Remove-Item -Path $path -Force -ErrorAction Stop
        } catch {
            Write-Host "[warn] Failed to delete stale executable $path before rebuild: $_" -ForegroundColor Yellow
        }
    }
}

function Copy-WindowsIntegrationExecutableForRun {
    param(
        [string]$SourcePath,
        [string]$DestinationDirectory
    )

    $resolvedSourcePath = Resolve-NormalizedWindowsPath $SourcePath
    if ([string]::IsNullOrWhiteSpace($resolvedSourcePath) -or -not (Test-Path $resolvedSourcePath -PathType Leaf)) {
        throw "Executable not found for staging: $SourcePath"
    }

    [void][System.IO.Directory]::CreateDirectory($DestinationDirectory)
    $destinationPath = Join-Path $DestinationDirectory (Split-Path $resolvedSourcePath -Leaf)
    Copy-Item -Path $resolvedSourcePath -Destination $destinationPath -Force
    return (Resolve-NormalizedWindowsPath $destinationPath)
}

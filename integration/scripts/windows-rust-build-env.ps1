function Initialize-WindowsRustBuildEnv {
    $cl = Get-Command cl.exe -ErrorAction Stop
    $clDir = Split-Path $cl.Source -Parent
    $vcToolsVersionDir = $clDir

    1..3 | ForEach-Object {
        $vcToolsVersionDir = Split-Path $vcToolsVersionDir -Parent
    }

    $vcInclude = Join-Path $vcToolsVersionDir "include"
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

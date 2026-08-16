# desktop/scripts/add-defender-exclusion.ps1
#
# Adds Windows Defender process exclusions for the AppFS desktop binaries
# (agentfs.exe, claw.exe) so real-time protection does not trigger the
# AV<->WinFsp read-loop on mounted control-plane files
# (e.g. _appfs/principals.registry.json). See desktop/DEFENDER_EXCLUSION.md.
#
# Idempotent. Must run elevated (admin). Apply manually on dev/CI machines;
# the NSIS installer applies the same exclusions automatically on install.
#
# Usage (admin PowerShell):
#   powershell -ExecutionPolicy Bypass -File desktop\scripts\add-defender-exclusion.ps1

$ErrorActionPreference = 'Continue'
$processes = @('agentfs.exe', 'claw.exe')

# Require elevation; Add-MpPreference needs admin.
$principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Write-Error 'This script must be run as Administrator. Re-launch from an elevated PowerShell.'
    exit 1
}

foreach ($name in $processes) {
    try {
        Add-MpPreference -ExclusionProcess $name -ErrorAction Stop
        Write-Host "Added Defender process exclusion: $name"
    } catch {
        Write-Host "Exclusion for $name already present or unavailable: $($_.Exception.Message)"
    }
}

Write-Host ''
Write-Host 'Current Defender process exclusions:'
(Get-MpPreference).ExclusionProcess

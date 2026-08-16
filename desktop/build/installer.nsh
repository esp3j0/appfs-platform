; electron-builder auto-includes build/installer.nsh into the NSIS installer.
; See desktop/DEFENDER_EXCLUSION.md for the full rationale.
;
; Best-effort: add Windows Defender process exclusions for the AppFS binaries.
; Why: Defender's real-time protection scans the WinFsp-mounted control-plane
; files (e.g. _appfs/principals.registry.json) and falls into an AV<->WinFsp
; read-loop that burns CPU and can crash MsMpEng.exe. Excluding the serving
; process (agentfs.exe) breaks the loop regardless of which volume path the
; file is accessed through (host-path exclusions do NOT cover the WinFsp
; volume device path, which is why a process exclusion is required).
;
; Idempotent. Non-fatal: if the installer is not elevated or Defender is
; unavailable, the call fails silently and the user can apply the standalone
; script (desktop/scripts/add-defender-exclusion.ps1) as admin instead.

!macro customInstall
  nsExec::ExecToLog 'powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "Add-MpPreference -ExclusionProcess agentfs.exe; Add-MpPreference -ExclusionProcess claw.exe"'
  Pop $0
!macroend

!macro customUnInstall
  nsExec::ExecToLog 'powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "Remove-MpPreference -ExclusionProcess agentfs.exe; Remove-MpPreference -ExclusionProcess claw.exe"'
  Pop $0
!macroend

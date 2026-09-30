; Tauri NSIS hooks: the user-data safety net.
;
; On install:
;   * Snapshot the user's app-data dir (settings, history, ...) to %TEMP%
;     before the rest of the installer (or the previous uninstaller, which
;     auto-update invokes in passive mode) gets a chance to touch it.
;     PREINSTALL runs before any file deletion.
;   * Restore the snapshot when the key file vanished during the install.
;     Belt-and-suspenders against Tauri NSIS template variants that wipe app
;     data on upgrade.

!include "LogicLib.nsh"

; App-data dir must match `identifier` in tauri.conf.json. Tauri 2's
; `app_data_dir` resolves to `%APPDATA%\<identifier>\` on Windows. The
; backup lives in %TEMP% so it disappears on reboot even if a restore is
; somehow skipped — never accumulates stale snapshots.
!define SUBCLAVE_DATA_DIR    "$APPDATA\dev.rendy.subclave"
!define SUBCLAVE_DATA_BACKUP "$TEMP\subclave-userdata-backup"

!macro NSIS_HOOK_PREINSTALL
  ; --- snapshot user data --------------------------------------------------
  ; Only snapshot when there's something to save; a fresh install has no
  ; data dir and we don't want to seed an empty backup that the post-hook
  ; would then "restore" over a clean install.
  IfFileExists "${SUBCLAVE_DATA_DIR}\*.*" 0 subclave_preinstall_no_backup
    ; Wipe any previous backup so a re-run starts clean.
    RMDir /r "${SUBCLAVE_DATA_BACKUP}"
    ; xcopy ships with Windows; /E recursive, /I treat target as dir,
    ; /Y silent overwrite, /H copy hidden + system, /K preserve attrs,
    ; /Q quiet. nul redirection suppresses console output during /PASSIVE.
    nsExec::ExecToLog 'cmd /c xcopy "${SUBCLAVE_DATA_DIR}" "${SUBCLAVE_DATA_BACKUP}" /E /I /Y /H /K /Q >nul 2>&1'
    ; nsExec pushes the exit code whether or not you want it. Leaving it on
    ; the stack corrupts whatever the Tauri template pops next.
    Pop $0
  subclave_preinstall_no_backup:
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; --- restore user data ---------------------------------------------------
  ; If PREINSTALL took a snapshot, copy it back. One key file (settings) gates
  ; the restore — if it is missing post-install we assume the install flow
  ; wiped the dir and replay the snapshot. /Y forces overwrite so the
  ; pre-install state always wins; the new app hasn't started yet, so nothing
  ; in the data dir is worth keeping. On a clean install the backup never
  ; existed and this is a no-op.
  IfFileExists "${SUBCLAVE_DATA_BACKUP}\*.*" 0 subclave_postinstall_no_restore
    StrCpy $5 "0"
    IfFileExists "${SUBCLAVE_DATA_DIR}\subclave-settings.json" +2 0
      StrCpy $5 "1"
    ${If} $5 == "1"
      CreateDirectory "${SUBCLAVE_DATA_DIR}"
      nsExec::ExecToLog 'cmd /c xcopy "${SUBCLAVE_DATA_BACKUP}" "${SUBCLAVE_DATA_DIR}" /E /I /Y /H /K /Q >nul 2>&1'
      Pop $0
    ${EndIf}
    RMDir /r "${SUBCLAVE_DATA_BACKUP}"
  subclave_postinstall_no_restore:
!macroend

; Keep the per-user app location stable across upgrades. The narrowly scoped
; privileged helper requests ordinary Windows administrator approval itself.
!macro NSIS_HOOK_POSTINSTALL
  ExecWait '"$INSTDIR\sanser-uac-service.exe" --installer-enable' $0
  ${If} $0 != 0
    DetailPrint "Sanser UAC helper setup failed or administrator approval was declined."
    MessageBox MB_OK|MB_ICONEXCLAMATION "Sanser is installed, but Windows administrator-window support could not be updated. Approve Windows administrator access when reinstalling, or use Host > Windows administrator prompts > Update UAC support." /SD IDOK
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; Tauri passes /UPDATE to upgrade uninstalls. Keep the existing service and
  ; account binding until POSTINSTALL replaces it with the new bundled files.
  ${If} $UpdateMode != 1
    ExecWait '"$INSTDIR\sanser-uac-service.exe" --installer-remove' $0
    ${If} $0 != 0
      MessageBox MB_OK|MB_ICONEXCLAMATION "Approve Windows administrator access to remove Sanser UAC support. The helper could not be removed." /SD IDOK
      Abort
    ${EndIf}
  ${EndIf}
!macroend

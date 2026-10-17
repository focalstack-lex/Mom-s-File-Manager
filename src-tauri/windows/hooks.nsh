; Move to Tote (2026-10-17): File Explorer's right-click entry for every
; file, for her account only (HKCU, no administrator rights). On Windows 11
; it sits under "Show more options"; Windows keeps the top of the new menu
; for signed packaged extensions. Explorer starts one Tote per selected file
; ("Player" lifts the 15-file limit); a running Tote takes the file through
; the single-instance plugin and files it (src-tauri/src/filing.rs).
; The updater runs this installer too, so an updated Tote gets the entry.

!define TOTE_VERB_KEY "Software\Classes\*\shell\Tote.MoveTo"

!macro NSIS_HOOK_POSTINSTALL
  WriteRegStr HKCU "${TOTE_VERB_KEY}" "MUIVerb" "Move to Tote"
  WriteRegStr HKCU "${TOTE_VERB_KEY}" "Icon" "$INSTDIR\${MAINBINARYNAME}.exe"
  WriteRegStr HKCU "${TOTE_VERB_KEY}" "MultiSelectModel" "Player"
  WriteRegStr HKCU "${TOTE_VERB_KEY}\command" "" '"$INSTDIR\${MAINBINARYNAME}.exe" --move-to-tote "%1"'
!macroend

; An update runs the old uninstaller with /UPDATE before the new files go
; in; the entry stays then, so it never goes missing between the two.
!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    DeleteRegKey HKCU "${TOTE_VERB_KEY}"
  ${EndIf}
!macroend

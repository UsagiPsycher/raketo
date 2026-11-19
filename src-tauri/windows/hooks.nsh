; Raketo installer hooks.
; - Firewall: allow inbound connections for LAN sharing, so the app never
;   needs a firewall prompt or any manual setup.
; - Command line: install `raketo.com` next to the app and put the folder on
;   PATH, so `raketo print …` works in any terminal.

!macro NSIS_HOOK_POSTINSTALL
  nsExec::Exec 'netsh advfirewall firewall delete rule name=all program="$INSTDIR\${MAINBINARYNAME}.exe"'
  nsExec::Exec 'netsh advfirewall firewall add rule name="Raketo" dir=in action=allow program="$INSTDIR\${MAINBINARYNAME}.exe" enable=yes profile=any description="Raketo: share the printer on the local network"'
  CopyFiles /SILENT "$INSTDIR\raketo-cmd.exe" "$INSTDIR\raketo.com"
  InitPluginsDir
  File "/oname=$PLUGINSDIR\raketo-path.ps1" "${__FILEDIR__}\..\..\..\..\src-tauri\windows\path.ps1"
  nsExec::Exec 'powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$PLUGINSDIR\raketo-path.ps1" -Dir "$INSTDIR"'
  SendMessage 0xFFFF 0x1A 0 "STR:Environment" /TIMEOUT=5000
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  nsExec::Exec 'netsh advfirewall firewall delete rule name=all program="$INSTDIR\${MAINBINARYNAME}.exe"'
  Delete "$INSTDIR\raketo.com"
  InitPluginsDir
  File "/oname=$PLUGINSDIR\raketo-path.ps1" "${__FILEDIR__}\..\..\..\..\src-tauri\windows\path.ps1"
  nsExec::Exec 'powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$PLUGINSDIR\raketo-path.ps1" -Dir "$INSTDIR" -Remove'
  SendMessage 0xFFFF 0x1A 0 "STR:Environment" /TIMEOUT=5000
!macroend

; Raketo installer hooks: allow inbound connections for LAN sharing, so the
; app never needs a firewall prompt or any manual setup.

!macro NSIS_HOOK_POSTINSTALL
  nsExec::Exec 'netsh advfirewall firewall delete rule name=all program="$INSTDIR\${MAINBINARYNAME}.exe"'
  nsExec::Exec 'netsh advfirewall firewall add rule name="Raketo" dir=in action=allow program="$INSTDIR\${MAINBINARYNAME}.exe" enable=yes profile=any description="Raketo: share the printer on the local network"'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  nsExec::Exec 'netsh advfirewall firewall delete rule name=all program="$INSTDIR\${MAINBINARYNAME}.exe"'
!macroend

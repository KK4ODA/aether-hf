; Aether HF's installer hooks (tauri.conf.json: bundle > windows > nsis > installerHooks).
;
; The uninstaller removes the program. What the operator made is theirs, and it stays
; unless they say otherwise — asked one kind at a time, every question answered "keep" by
; Enter:
;
;   settings and caches   station.toml and its backups, the dial memories, the installers
;                         kept for going back, the window's stored data
;   profiles and history  profiles, the logs, the stations heard, the session history
;   recordings            the default recordings folder, with a warning of its own
;
; A recordings folder set elsewhere with [record] dir is never touched. An update runs the
; old uninstaller with /UPDATE, and a passive or silent uninstall asks nothing: those keep
; everything. Only files Aether HF writes are named, so a folder the operator has put
; something else into is left with that in it.

!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
  ${AndIf} $PassiveMode <> 1
  ${AndIfNot} ${Silent}
    SetShellVarContext current
    StrCpy $R0 "$APPDATA\aether-hf"
    StrCpy $R1 "$LOCALAPPDATA\aether-hf"
    ${If} ${FileExists} "$R0\*.*"
    ${OrIf} ${FileExists} "$R1\*.*"
      MessageBox MB_YESNO|MB_ICONQUESTION|MB_DEFBUTTON2 \
        "Also remove Aether HF's settings and caches?$\n$\n\
Settings: station.toml and its backups, the dial memories.$\n\
Caches: the installers kept for going back, the window's stored data.$\n$\n\
They are in $R0 and $R1.$\n$\n\
Choose No to keep everything: a reinstall or a newer version picks up where this one \
left off. Your profiles, logs, history and recordings are asked about next." \
        /SD IDNO IDNO aether_keep
      Delete "$R0\station.toml"
      Delete "$R0\station.toml.bak-*"
      Delete "$R0\station.toml.new"
      Delete "$R0\frequencies.json"
      Delete "$R0\frequencies.json.tmp"
      RMDir /r "$R1"
      RMDir /r "$APPDATA\${BUNDLEID}"
      RMDir /r "$LOCALAPPDATA\${BUNDLEID}"

      MessageBox MB_YESNO|MB_ICONQUESTION|MB_DEFBUTTON2 \
        "Also remove your profiles, logs and history?$\n$\n\
Profiles: the named station setups.$\n\
Logs: aetherd.log and the one before it.$\n\
History: the stations heard and the sessions.$\n$\n\
Choose No to keep them, and your recordings." \
        /SD IDNO IDNO aether_done
      RMDir /r "$R0\profiles"
      Delete "$R0\profiles.json"
      Delete "$R0\aetherd.log"
      Delete "$R0\aetherd.prev.log"
      Delete "$R0\heard.json"
      Delete "$R0\heard.json.tmp"
      Delete "$R0\sessions.json"
      Delete "$R0\sessions.json.tmp"

      ${If} ${FileExists} "$R0\recordings\*.*"
        MessageBox MB_YESNO|MB_ICONEXCLAMATION|MB_DEFBUTTON2 \
          "Delete your recordings as well?$\n$\n\
$R0\recordings holds the audio and the reports of your recorded sessions and Test \
sessions: on-air evidence that cannot be recorded again.$\n$\n\
This cannot be undone. Choose No to keep them." \
          /SD IDNO IDNO aether_done
        RMDir /r "$R0\recordings"
      ${EndIf}

      aether_done:
      ; gone only if nothing is left in it
      RMDir "$R0"
    ${EndIf}
    aether_keep:
  ${EndIf}
!macroend

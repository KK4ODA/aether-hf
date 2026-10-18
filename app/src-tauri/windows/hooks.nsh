; Aether HF's installer hooks (tauri.conf.json: bundle > windows > nsis > installerHooks).
;
; Before the installer replaces the program, and before the uninstaller removes it, nothing
; may hold its files. Tauri's own check (CheckIfAppIsRunning, from bundler 2.10) asks the
; Restart Manager once to close whatever holds aether-hf.exe and stops the installer at the
; first refusal ("Failed to kill Aether HF"): an update's installer starts while the shell
; that launched it is still exiting, and a file held by something else — an antivirus,
; Explorer, a copy started as administrator, a process stuck on its way out — cannot be
; closed that way at all. The PREINSTALL and PREUNINSTALL hooks run before that check and
; leave it nothing to find, for the shell and the modem alike:
;
;   wait      for the program to let go by itself: 20 s when nobody is there to ask (an
;             update, a passive or silent run), a glance otherwise
;   close     the shell, through the Restart Manager as Tauri does, after asking when
;             someone is there; it stops the modem and releases the radio on its way out.
;             The modem is never closed by force: killed, it could leave a radio keyed by
;             CAT on the air. It is asked to stop over its control port instead.
;   set aside what is still held: renamed to <name>.old, which Windows allows for a program
;             in use where it refuses overwriting or deleting it. The new file is written
;             beside it, and the shell deletes the old one at its next start.
;   say       which programs held it, in the details pane, so a report names the culprit.
;
; An update never runs the old uninstaller, so these run from the first update into a build
; that has them. A manual upgrade that chooses "Uninstall before installing" runs the old
; build's uninstaller, with the old build's hooks.
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

!define /ifndef ERROR_MORE_DATA 234

; How long a program gets to let go of its file by itself, every 250 ms, when nobody is there
; to ask: the shell that started an update is still exiting. Interactive runs take a glance,
; for a window closed a moment ago, and then ask. (tools/installer_hooks/ tests these hooks
; with shorter waits, defined before it includes the file.)
!define /ifndef AETHER_POLL_MS 250
!define /ifndef AETHER_UNATTENDED_MS 20000
!define /ifndef AETHER_GLANCE_MS 2000
; How long the shell gets to close when asked, and the modem to stop: up to 22 s to end a
; session on the air first (a DISC, the answer and both closing identifiers: WIND_DOWN in
; aetherd), then the radio released; the shell itself waits 30 s (SHUTDOWN_TIMEOUT).
!define /ifndef AETHER_CLOSE_MS 30000
; Where the shell finds the modem's control port (CONTROL in main.rs), so where it is asked to
; stop. `POST /v1/shutdown` over loopback needs no token.
!define AETHER_SHUTDOWN_URL "http://127.0.0.1:8515/v1/shutdown"
; RM_PROCESS_INFO: the process id and start time (12 bytes), the application's name (256
; wide characters), the service's (64), its type, status, session and restartability.
!define AETHER_RM_PROCESS_INFO 668

; How many processes hold `path` open, into `out` (0 when the Restart Manager cannot say).
; Uses $0-$3 and $9.
!macro AETHER_HOLDERS path out
  StrCpy ${out} 0
  !insertmacro RestartManager_StartSession $9
  ${If} $9 != ""
    !insertmacro RestartManager_RegisterFile $9 "${path}"
    ${If} $0 = 0
      System::Call 'RSTRTMGR::RmGetList(i r9, *i .r1, *i 0 r2, p 0, *i .r3) i .r0'
      ${If} $0 = ${ERROR_MORE_DATA}
        StrCpy ${out} $1
      ${EndIf}
    ${EndIf}
    !insertmacro RestartManager_EndSession $9
  ${EndIf}
!macroend

; Wait up to `limit` ms for nothing to hold `path`; how many still do, into $7. Uses $0-$3,
; $6 and $9.
!macro AETHER_AWAIT path limit
  StrCpy $6 0
  ${Do}
    !insertmacro AETHER_HOLDERS "${path}" $7
    ${If} $7 = 0
    ${OrIf} $6 >= ${limit}
      ${ExitDo}
    ${EndIf}
    Sleep ${AETHER_POLL_MS}
    IntOp $6 $6 + ${AETHER_POLL_MS}
  ${Loop}
!macroend

; Ask what holds `path` to close: Windows' "close for an update" message (WM_ENDSESSION),
; which the shell answers by stopping the modem and exiting, and after the Restart Manager's
; timeout an end by force for a program that did not answer. Uses $0 and $9.
!macro AETHER_ASK_TO_CLOSE path
  !insertmacro RestartManager_StartSession $9
  ${If} $9 != ""
    !insertmacro RestartManager_RegisterFile $9 "${path}"
    ${If} $0 = 0
      System::Call 'RSTRTMGR::RmShutdown(i r9, i ${RmForceShutdown}, p 0) i .r0'
    ${EndIf}
    !insertmacro RestartManager_EndSession $9
  ${EndIf}
!macroend

; Ask the modem to stop over its control port: it ends a session on the air and releases the
; radio before it exits, which a modem ended by force would not. curl.exe comes with Windows
; 10 (1803) and later; the 64-bit one is reached through Sysnative from this 32-bit installer.
; Without it the modem is only waited for. Uses $0 and $5.
!macro AETHER_ASK_MODEM_TO_STOP
  StrCpy $5 "$WINDIR\Sysnative\curl.exe"
  ${IfNot} ${FileExists} "$5"
    StrCpy $5 "$SYSDIR\curl.exe"
  ${EndIf}
  ${If} ${FileExists} "$5"
    nsExec::Exec '"$5" --silent --max-time 5 --request POST --header "Content-Type: application/json" --data "{}" ${AETHER_SHUTDOWN_URL}'
    Pop $0
  ${EndIf}
!macroend

; Name in the details pane the processes that hold `path`, at most 16. Uses $0-$9.
!macro AETHER_SAY_HOLDERS path
  !insertmacro RestartManager_StartSession $9
  ${If} $9 != ""
    !insertmacro RestartManager_RegisterFile $9 "${path}"
    ${If} $0 = 0
      System::Call 'RSTRTMGR::RmGetList(i r9, *i .r1, *i 0 r2, p 0, *i .r3) i .r0'
      ${If} $0 = ${ERROR_MORE_DATA}
        ; room for one or two more that start meanwhile
        IntOp $2 $1 + 2
        ${If} $2 > 16
          StrCpy $2 16
        ${EndIf}
        IntOp $4 $2 * ${AETHER_RM_PROCESS_INFO}
        System::Alloc $4
        Pop $4
        System::Call 'RSTRTMGR::RmGetList(i r9, *i .r1, *i r2 r2, p r4, *i .r3) i .r0'
        ${If} $0 = 0
          StrCpy $5 0
          ${DoWhile} $5 < $2
            IntOp $6 $5 * ${AETHER_RM_PROCESS_INFO}
            IntPtrOp $6 $4 + $6
            System::Call '*$6(i .r7, i, i, &w256 .r8)'
            DetailPrint "  held by $8 (process $7)"
            IntOp $5 $5 + 1
          ${Loop}
        ${EndIf}
        System::Free $4
      ${EndIf}
    ${EndIf}
    !insertmacro RestartManager_EndSession $9
  ${EndIf}
!macroend

; Rename `path` to `path.old`, or .old2 to .old9 while an earlier one is still held, so the
; new file can be written however long the old one stays in use. Uses $5 and $6.
!macro AETHER_SET_ASIDE path
  StrCpy $5 "${path}.old"
  StrCpy $6 1
  ${Do}
    Delete "$5"
    ${IfNot} ${FileExists} "$5"
    ${OrIf} $6 >= 9
      ${ExitDo}
    ${EndIf}
    IntOp $6 $6 + 1
    StrCpy $5 "${path}.old$6"
  ${Loop}
  ClearErrors
  Rename "${path}" "$5"
  ${If} ${Errors}
    DetailPrint "Windows would not let it be moved aside either."
  ${Else}
    DetailPrint "Moved it aside to $5; Aether HF deletes it at its next start."
  ${EndIf}
!macroend

; How long a program gets to let go by itself, into `out`: long when nobody is there to ask.
!macro AETHER_PATIENCE out
  StrCpy ${out} ${AETHER_GLANCE_MS}
  ${If} $UpdateMode = 1
  ${OrIf} $PassiveMode = 1
  ${OrIf} ${Silent}
    StrCpy ${out} ${AETHER_UNATTENDED_MS}
  ${EndIf}
!macroend

; Leave nothing holding the shell's or the modem's file (the header says how).
!macro AETHER_LET_GO
  Push $0
  Push $1
  Push $2
  Push $3
  Push $4
  Push $5
  Push $6
  Push $7
  Push $8
  Push $9

  ; the shell
  !insertmacro AETHER_PATIENCE $8
  StrCpy $7 0
  ${If} ${FileExists} "$INSTDIR\${MAINBINARYNAME}.exe"
    !insertmacro AETHER_HOLDERS "$INSTDIR\${MAINBINARYNAME}.exe" $7
  ${EndIf}
  ${If} $7 > 0
    DetailPrint "Waiting for ${PRODUCTNAME} to close..."
    !insertmacro AETHER_AWAIT "$INSTDIR\${MAINBINARYNAME}.exe" $8
  ${EndIf}
  ${If} $7 > 0
    ${If} $UpdateMode <> 1
    ${AndIf} $PassiveMode <> 1
    ${AndIfNot} ${Silent}
      ${If} ${Cmd} `MessageBox MB_OKCANCEL|MB_ICONINFORMATION "${PRODUCTNAME} is running.$\n$\nOK closes it. It stops the modem and releases the radio first.$\nCancel leaves it running and stops here." /SD IDOK IDCANCEL`
        Pop $9
        Pop $8
        Pop $7
        Pop $6
        Pop $5
        Pop $4
        Pop $3
        Pop $2
        Pop $1
        Pop $0
        Abort "${PRODUCTNAME} is still running. Close it, then try again."
      ${EndIf}
    ${EndIf}
    DetailPrint "Asking ${PRODUCTNAME} to close..."
    !insertmacro AETHER_ASK_TO_CLOSE "$INSTDIR\${MAINBINARYNAME}.exe"
    !insertmacro AETHER_AWAIT "$INSTDIR\${MAINBINARYNAME}.exe" ${AETHER_CLOSE_MS}
  ${EndIf}
  ${If} $7 > 0
    DetailPrint "$INSTDIR\${MAINBINARYNAME}.exe is still in use:"
    !insertmacro AETHER_SAY_HOLDERS "$INSTDIR\${MAINBINARYNAME}.exe"
    !insertmacro AETHER_SET_ASIDE "$INSTDIR\${MAINBINARYNAME}.exe"
  ${EndIf}

  ; the modem, which a closing shell has just asked to stop
  !insertmacro AETHER_PATIENCE $8
  StrCpy $7 0
  ${If} ${FileExists} "$INSTDIR\aetherd.exe"
    !insertmacro AETHER_HOLDERS "$INSTDIR\aetherd.exe" $7
  ${EndIf}
  ${If} $7 > 0
    DetailPrint "Waiting for the modem to stop..."
    !insertmacro AETHER_AWAIT "$INSTDIR\aetherd.exe" $8
  ${EndIf}
  ${If} $7 > 0
    DetailPrint "Asking the modem to stop..."
    !insertmacro AETHER_ASK_MODEM_TO_STOP
    !insertmacro AETHER_AWAIT "$INSTDIR\aetherd.exe" ${AETHER_CLOSE_MS}
  ${EndIf}
  ${If} $7 > 0
    DetailPrint "$INSTDIR\aetherd.exe is still in use:"
    !insertmacro AETHER_SAY_HOLDERS "$INSTDIR\aetherd.exe"
    !insertmacro AETHER_SET_ASIDE "$INSTDIR\aetherd.exe"
  ${EndIf}

  Pop $9
  Pop $8
  Pop $7
  Pop $6
  Pop $5
  Pop $4
  Pop $3
  Pop $2
  Pop $1
  Pop $0
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro AETHER_LET_GO
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro AETHER_LET_GO
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; what PREUNINSTALL had to set aside goes too, unless it is still in use
  Delete "$INSTDIR\${MAINBINARYNAME}.exe.old*"
  Delete "$INSTDIR\aetherd.exe.old*"
  ${If} ${FileExists} "$INSTDIR\*.old*"
    DetailPrint "Still in use, so left in $INSTDIR to delete later: the old ${PRODUCTNAME} files ending .old."
  ${EndIf}
  RMDir "$INSTDIR"

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

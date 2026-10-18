; Drives the installer's hooks (app/src-tauri/windows/hooks.nsh) one scenario at a time, with
; waits shortened to a tenth: /SCENARIO=letgo|unletgo|say|post, /UPDATE and /P as the real
; installer takes them, and /D=<folder> last for $INSTDIR. It writes what it found to
; result.txt beside itself: the registers and the stack it must get back as it left them, and
; the last holder the report read. `run.py` builds it twice, silent and with a window
; (-DINTERACTIVE), with -DHOOKS=<path to hooks.nsh>.
Unicode true
!define AETHER_POLL_MS 100
!define AETHER_UNATTENDED_MS 1500
!define AETHER_GLANCE_MS 300
!define AETHER_CLOSE_MS 2000
!define MAINBINARYNAME "aether-hf"
!define PRODUCTNAME "Aether HF"
!define BUNDLEID "org.aetherhf.hooktest"
!include LogicLib.nsh
!include FileFunc.nsh
!include "Win\RestartManager.nsh"
!include "${HOOKS}"
Var UpdateMode
Var PassiveMode
Name "Aether HF hooks test"
RequestExecutionLevel user
!ifdef INTERACTIVE
  OutFile "harness-ui.exe"
  SilentInstall normal
  AutoCloseWindow true
!else
  OutFile "harness.exe"
  SilentInstall silent
!endif

Section
  ${GetParameters} $R0
  StrCpy $UpdateMode 0
  StrCpy $PassiveMode 0
  ClearErrors
  ${GetOptions} $R0 "/UPDATE" $R1
  ${IfNot} ${Errors}
    StrCpy $UpdateMode 1
  ${EndIf}
  ClearErrors
  ${GetOptions} $R0 "/P" $R1
  ${IfNot} ${Errors}
    StrCpy $PassiveMode 1
  ${EndIf}
  ${GetOptions} $R0 "/SCENARIO=" $R2
  StrCpy $R5 ""
  ; what the hooks must leave as they found it
  StrCpy $0 s0
  StrCpy $1 s1
  StrCpy $2 s2
  StrCpy $3 s3
  StrCpy $4 s4
  StrCpy $5 s5
  StrCpy $6 s6
  StrCpy $7 s7
  StrCpy $8 s8
  StrCpy $9 s9
  Push sentinel
  ${If} $R2 == "letgo"
    !insertmacro NSIS_HOOK_PREINSTALL
  ${ElseIf} $R2 == "unletgo"
    !insertmacro NSIS_HOOK_PREUNINSTALL
  ${ElseIf} $R2 == "say"
    !insertmacro AETHER_SAY_HOLDERS "$INSTDIR\probe.exe"
    StrCpy $R5 "$7|$8"
  ${ElseIf} $R2 == "post"
    !insertmacro NSIS_HOOK_POSTUNINSTALL
  ${EndIf}
  Pop $R4
  FileOpen $R3 "$EXEDIR\result.txt" w
  FileWrite $R3 "scenario=$R2$\r$\n"
  FileWrite $R3 "modes=update $UpdateMode passive $PassiveMode$\r$\n"
  FileWrite $R3 "registers=$0,$1,$2,$3,$4,$5,$6,$7,$8,$9$\r$\n"
  FileWrite $R3 "stack=$R4$\r$\n"
  FileWrite $R3 "say=$R5$\r$\n"
  FileClose $R3
SectionEnd

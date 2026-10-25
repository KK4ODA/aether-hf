# ADR-0097: A host program that starts Aether as VARA starts a stand-in

**Status:** accepted, 2026-10-25. The desktop shell only (`app/src-tauri/src/stand_in.rs`,
`main.rs`): no change to the modem, the wire or the configuration.

## 1. Context

RMS Trimode starts its modem from a path the operator sets, the way it starts VARA:
`<path> TCPIP 127.0.0.1 <port>`. It watches the process it started and, to restart the modem
("Restart Vara", after a command went unanswered or a TNC setting changed), it kills that
process ("Trying to stop Vara TNC … Successfully stopped Vara TNC") and starts the path again.
For VARA that is right: the process is the modem. For Aether it was the window.

The gateway's logs for 2026-10-24/25 (KK4ODA-1, Trimode 1.4.4.0, Aether beta.122) show the
consequences. Trimode started `aether-hf.exe` at 23:57:04 and the modem started in the same
second. The same modem then ran until 03:45, through Trimode's restarts at 00:00:38 and five
between 03:31 and 03:40. Each restart killed the window Trimode had started and started
another. The modem ran on with no window watching it, and the new window found it running and
adopted it. That is the window the author saw "terminate and close abnormally, as if Trimode
was trying to open it or close it". An adopted modem was not started again after a profile
switch, until beta.125's fix (2 min 44 s with no modem at 04:09). Two more things came out of
the same logs:

* When a modem restarts while a second copy of the shell is starting, the second copy starts a
  modem of its own before the single-instance check sends it away. At 03:45:43 Trimode's launch
  started the modem the station ran on for the next 25 minutes, and the shell that launch began
  exited at once. The window that was open then had restarted nothing (`ensure_daemon` found a
  modem answering), so it stopped watching any.
* Winlink Express starts its TNC with no arguments ("Launching Vara TNC: …aether-hf.exe"). On
  the truck's laptop that window was the process Winlink Express had started. When Winlink
  Express closed at 03:32:33 the modem ran on with no window, and the next window adopted it.

## 2. Decision

1. **Started with VARA's `TCPIP` argument, `aether-hf.exe` is a stand-in.** It opens no window.
   It starts Aether as a process of its own when no modem answers: at once, or after up to 20 s
   when the modem's log says it is stopping to be started again (a profile switch is back in a
   second or a few). It leaves a running modem alone and does not bring the window forward.
   It stays while the modem answers, and leaves a minute after the modem has gone, as VARA's
   process would end, so the host program starts it again as it would VARA. Trimode's kill ends
   the stand-in and nothing else. On Windows Aether is started out of any job the host program
   runs its modem in (`CREATE_BREAKAWAY_FROM_JOB`), the plain start when the job refuses.
2. **A window whose restart finds another modem answering adopts it** (`watch_daemon`), as a
   window does that finds one at its own start, and it watches that modem from then on.

## 3. Consequences

* Trimode's restarts no longer close the window, and the window no longer comes forward at each
  of them. Closing Aether while Trimode runs ends the stand-in a minute later, and Trimode starts
  Aether again, as it would VARA. That is a gateway's behaviour.
* Winlink Express and VarAC start their TNC without `TCPIP`, so this changes nothing for them.
  The guides' advice stands: start Aether first, and leave the program's automatic launch off.
* Tests: `stand_in.rs` (the launch told from the operator's, a running modem left alone and
  watched until it goes, a restart waited for, a restart that never comes, a cold start, a modem
  back within the minute); the shell's test that holds `RESTART_SAID` to the modem's own words.

## 4. Not done

* A second copy of the shell still starts a modem before the single-instance check, when none
  answers. Moving the start after the check changes when the window first loads the panel, and
  that can only be tested on Windows. The adoption in §2.2 makes the orphan a watched modem
  instead.
* Telling Winlink Express's launch from the operator's: it passes nothing that says so, and
  reading the parent process is more machinery than the setup advice that already avoids it.

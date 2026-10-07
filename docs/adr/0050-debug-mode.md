# ADR-0050: Debug mode sends the host program's sessions to the project

**Status:** accepted, 2026-10-07 (the author's decision: on by default, said once, audio
included). `[record] send_to_project` (default `true`, live, never in a profile); configuration
schema 11 (`debug_mode`, a step that changes nothing); `core/aetherd/src/debug.rs` (the queue),
`control::methods::start_debug_upload`, `status.debug`; the panel's Setup step 5 *Debug mode*
and the one-time notice (`#debug-banner`). No wire change.

## 1. Context

The next field trials are Winlink through a gateway on Aether: trusted operators run Winlink
Express (or Pat) against the author's RMS gateway, with Aether in place of VARA HF at both ends.
Every session of that kind is worth having from both sides — the gateway's recording says what
it heard, the client's what it heard back, and the two together are how ADR-0040, ADR-0048 and
ADR-0049 were found.

Getting the client's side has always been the hard part. ND1J's log of 2026-09-27 was gone by
the time it was asked for (the shell overwrote it at each start, beta.71); KE4QCM's files took
a PowerShell line in an email (2026-10-05); WC4Y's arrived as GitHub issue attachments a day
later. *Send files…* (beta.85–88) made it one button, but somebody still has to press it, after
the fact, for the right hours.

## 2. Decision

1. **What is sent.** Every session a *host program* runs: one a program attached to the host
   interface (Winlink Express, Pat, VarAC, BPQ32) had when it came up — its session, as far as
   the station can tell (`Session.host`). Not the panel's own sessions, not Test sessions
   (which have their own report), not beacons or probes.
2. **What goes with it.** The session's recording — the sidecar and the audio — the daemon's
   logs (this run, the one before, the dated runs written to since), the session history, the
   stations heard and the diagnostic bundle (the settings without their secrets), as one zip
   (`share.rs`, the same format as *Send files…*). The audio is included: it is what makes a
   session replayable, and the author chose it.
3. **Recording.** With the setting on, a host program's session is recorded whatever
   `record.auto` says (`StationConfig::record_host_sessions`); a session's own recording ends
   with it even if the setting or the program went in between (`recording_session`).
4. **When.** A minute after the last such session ended (`QUIET_MS`: a client that calls again
   at once puts both in one zip), only while the station is idle and not transmitting, never
   over an upload the operator started, at most six sessions a zip. The zip is written and
   uploaded on the upload's own thread (`upload::Job.zip`): the audio is tens of megabytes, and
   the run loop must not stand still while it is copied.
5. **Failure.** Tried again after 30 minutes, then 60; after three tries the files stay in the
   recordings folder and the log says whose they were. At most 24 sessions wait; past that the
   oldest stays on this computer, said in the log. The queue is not kept across a restart: the
   recordings are, and *Send files…* sends them.
6. **Where.** The project's upload script (`upload::PROJECT_ENDPOINT`, the panel's
   `PROJECT_UPLOAD_URL`; a test holds them equal), with no code. The script's daily bound is
   raised to 80 uploads and 8 GB, and a refused email (Apps Script's 100 a day) no longer fails
   an upload whose file arrived.
7. **Consent.** On by default, because the trial is what the setting is for and an opt-in that
   nobody ticks sends nothing. The operator is told, once, on whatever tab is open, in plain
   words — what is sent, where, that the audio carries what was sent — with *Keep it on* and
   *Turn it off* (`aether.debugNoticeSeen`). Setup step 5 has the switch. A headless gateway says
   so in its log at every start. The setting is `Scope::Machine`: it is this computer's
   operator's consent, and loading a profile from somebody else must not change it.
8. **What the operator sees.** A `debug` log line when a zip goes, when it has gone and when it
   failed; `status.debug` (`on`, `waiting`, `sending`, `sent`, `last_error`).

## 3. Consequences

* Every tester running a beta with this build sends their Winlink sessions without being asked,
  unless they turn it off.
* The project's Drive takes about 6 MB a minute of session. The script's daily bound keeps a
  public address from filling it; the author watches the free space.
* The audio of a Winlink session can be decoded by anyone with Aether: the messages it carried
  are in it. Amateur traffic is not private (§97.113(a)(4)), but the notice says so plainly.
* No wire change: beta.88 and this build interoperate.

## 4. Not done

* Sending the panel's own sessions, or every session: the trial is Winlink, and a chat with a
  friend is not the project's business unless the operator sends it.
* Keeping the queue across a restart: the recordings are kept, and a restart is rare during a
  session's minute.
* Sending without the audio by default: the author chose the audio.

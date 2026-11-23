# ADR-0050: Debug mode sends the host program's sessions to the project

**Status:** accepted, 2026-10-07 (the author's decision: on by default, said once, audio
included). `[record] send_to_project` (default `true`, live, never in a profile); configuration
schema 11 (`debug_mode`, a step that changes nothing); `core/aetherd/src/debug.rs` (the queue),
`control::methods::start_debug_upload`, `status.debug`; the panel's Setup step 5 *Debug mode*
and the one-time notice (`#debug-banner`; since §7 a window, `#debug-dialog`, and nothing goes
before it is answered). No wire change.

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
   `record.auto` says (`StationConfig::record_program_sessions`); a session's own recording ends
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
* A station whose audio is simulated (`[sim]`, `--channel`) or a dry run never sends its
  sessions, whatever the setting (`DaemonState::simulated`, 2026-10-08): the bench, the
  scenario harness and CI's two-daemon tests would otherwise fill the project's folder.

## 4. Not done

* Sending the panel's own sessions, or every session: the trial is Winlink, and a chat with a
  friend is not the project's business unless the operator sends it.
* Keeping the queue across a restart: the recordings are kept, and a restart is rare during a
  session's minute.
* Sending without the audio by default: the author chose the audio.

## 5. Amended 2026-11-12: every program's sessions

The author's decision, after a morning of RadioPost sessions between KK4ODA and KK4ODA-1 with
debug mode on at both ends left nothing in the upload folder: since ADR-0106 debug mode had
recorded only a session owned by the program on the host interface, and RadioPost reaches the
station on the control API. Now a session owned by any program that holds callsigns
(`Station::owned_by_program`: the session has an owner), on either interface, is recorded and
sent; the session history keeps `program` beside `host` (`host` still says whose `CWID ON` is
honoured, ADR-0075 §2.7). The panel's own sessions have no owner and Tests are left out, as
before. `StationConfig::record_host_sessions` is now `record_program_sessions`. The notice,
Setup's tooltip, the start-up log line, the manual and the guides name RadioPost.

## 6. Amended 2026-11-18: offline is not a failure, and the queue outlives a restart

On 2026-11-18 the mobile (`KK4ODA-2`) had no Internet. Its first upload failed with "No such host
is known" and waited half an hour for a second try. Twenty-one minutes later the daemon restarted
for a profile switch, and the queue was gone; the author sent the files by hand that evening
(report 19 §0).

* **An upload that never reached the script counts no try.** That is a connection that failed
  before the script answered: the begin's `POST`, or, for debug mode's zip, a request made before
  the zip is written (`Http::reachable`, any answer will do, through the environment's proxy as the
  upload goes). `upload::Progress.unreached` says so. The sessions wait at the front and go again
  every `OFFLINE_RETRY_MS` (10 minutes) for as long as it takes (`DebugUploads::unreached`). The
  log says so once a spell, not at every try. Nothing is zipped while nothing answers: a mobile
  offline for hours would have written tens of megabytes at every try. A failure after the script
  answered counts its tries as before (§2, point 5).
* **The queue is kept beside the configuration** (`debug::QUEUE_FILE`, `debug-waiting.json`): what
  waits and what is on its way, written when it changes (`save_if_changed`). At start what an
  earlier run left goes a minute later, as after a session; one whose recording is no longer on
  this computer is let go, and the log says how many of each. Point 5's "the queue is not kept
  across a restart" no longer holds.
* Tests: `offline_costs_no_try_and_the_sessions_wait_until_the_internet_is_back`,
  `the_queue_outlives_a_restart` (`debug.rs`),
  `a_station_offline_is_told_from_a_failure_and_writes_no_zip_for_nowhere` (`upload.rs`).

No configuration key. The uninstaller leaves the file, as it leaves `program-logs.json`.

## 7. Amended 2026-11-23: asked in a window, and nothing goes before the answer

On 2026-11-22 two stations new to the project ran two Winlink P2P sessions with each other, and
both uploaded on their own a minute after each. Nothing says they read the one-line notice; it shared the banner strip with the
setup banners of a first start, and uploads began whether or not anyone had. The author's
decisions: a window at first start, no upload before it is answered, and every station asked
once, including those that had only the banner.

* **The window** (`#debug-dialog`, the panel's second `<dialog>`) opens over every tab while
  `status.debug.ask` is true and debug mode is on. It says what is sent (the recording with its
  audio, the logs, the settings without passwords), that the messages can be read from the audio,
  when (a minute after each session a program runs; never the panel's own sessions or Tests), and
  to whom. *Keep it on* or *Turn it off*; Escape does not close it. The one-line banner
  (`#debug-banner`, `aether.debugNoticeSeen` in the browser) is gone.
* **The answer is the daemon's**, `debug.answer {send}`: it sets `[record] send_to_project` as
  `config.set` would and writes `debug::ANSWER_FILE` (`debug-answered.json`) beside the
  configuration. It is not a configuration key: it is what the operator was told, not a setting,
  so there is no schema bump, a profile cannot carry it, and an earlier version going back over
  the folder ignores it. A new browser or a reinstall over the same folder does not ask again; a
  station that had only the banner has no file and is asked once.
* **Nothing goes before the answer.** `DebugUploads::due` is false until `answered`. Sessions a
  program runs meanwhile are recorded and wait in the queue (said in the log, one line a session);
  *Keep it on* sends them, *Turn it off* clears the queue as turning debug mode off always has.
* **A station with no panel** says at every start that it waits, and how to answer it: `POST
  /v1/debug.answer {"send": true}` (or `false`). A simulated or dry-run station is never asked
  (`ask` is false): its sessions never go.
* Tests: `nothing_goes_before_the_notice_is_answered` (`debug.rs`),
  `the_operator_is_asked_once_and_the_answer_sets_debug_mode` (`control/methods.rs`).

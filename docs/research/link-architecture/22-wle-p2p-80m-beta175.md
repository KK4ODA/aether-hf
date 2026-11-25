# Winlink Express P2P on 80 m, portable to home, with long transfers (2026-11-24)

Seven sessions on 80 m, 22:46–23:48Z (local 17:46–18:48), beta.175 at both ends: Winlink Express
at the portable calls Winlink Express at home in a *Vara HF P2P* session. Six ran at 500 Hz and
carried 0.1–2.7 kB each. The last ran at 2300 Hz with a 161.6 kB file. Winlink Express said its
inactivity timeout twice, and after 13 minutes and 6.4 kB the operator aborted.

The author asked for six things to be looked at, in this order: the disconnect messages during
the long transfer (§1), the callsign `KK4ODA-1` at the start (§2), the indicators of what an
external program controls (§3), throughput (§4), the *Receiving at* line (§5) and the Mode tile
(§6). §7 is the regression testing, §8 what is proposed, §9 the questions for the author and §10 what
the author decided and what was built. Each
finding gives the observation, the evidence, the cause and its confidence, a classification, a
severity, a proposed resolution and how to validate it.

* **Who**:
  * **The portable**: Lab599 TX-500, 10 W, linked EFHW, the 22-core PC. Aether's own callsign was
    `KK4ODA-2` until 22:42:58, then `KK4ODA`. Winlink Express as `WD5EMA` on host port 8500, keying
    the radio on `PTT ON`. Aether identifies in the data signal.
  * **Home**: FTDX10, 50 W, T2D, the 4-core mini PC. Aether's own callsign was `KK4ODA-1` (RadioPost
    Gateway's) until 22:42:28, then `KK4ODA`. Winlink Express as `KK4ODA` on host port 8700,
    keying the radio on `PTT ON`. Aether identifies in Morse.
* **The files**, none committed: twelve debug zips, `20261124-225118Z` … `20261124-234926Z`, all
  named for `KK4ODA` because both stations' Aether callsign was `KK4ODA` by then, fetched through
  temporary copies. Each holds one station's side of one or two sessions with audio, its logs with
  the host trace, and the diagnostic bundle. Recordings named `KK4ODA_WD5EMA` are home's, and
  `WD5EMA_KK4ODA` the portable's.
* **Read with** `tools/rung_reading.py` (both sidecars of each session), `tools/air_account.py
  --sidecars`, `tools/two_sides.py --from-loops`, the logs and the sidecars' frames.
* **Not in hand**: Winlink Express's own log for the long session. §1 says what it would settle.

| # | UTC | bandwidth | home read the portable at | delivered to home | how it ended |
|---|---|---|---|---|---|
| 1 | 22:46 | 500 Hz | −5 to −1 dB | ≈ 1.7 kB, four-tone rungs, 52–157 bit/s an exchange | the programs' `FQ`, a normal close |
| 2 | 22:53 | 500 Hz | −7 to −2 dB | ≈ 1.6 kB, tone rungs, 12–91 bit/s | a normal close |
| 3 | 23:18 | 500 Hz | +3 to +6 dB | ≈ 2.7 kB, QPSK ⅔ and 8PSK ½, 130–155 bit/s | a normal close |
| 4 | 23:23 | 500 Hz | +5 dB | 0.1 kB | a normal close |
| 5 | 23:24 | 500 Hz | 0 dB | ≈ 1.4 kB, tone4x100-93, 124 bit/s | the operator's `ABORT` |
| 6 | 23:28 | 500 Hz | +1 to +2 dB | ≈ 0.9 kB, QPSK ⅔ and 8PSK ½, 120–188 bit/s | the operator's `ABORT` |
| 7 | 23:35 | **2300 Hz** | −1 dB median, dips to −12 | 6.4 kB of 161.6 kB in 13 minutes, ≈ 67 bit/s | the operator's `ABORT` |

In every session the portable called and sent: no radiated overlap, answers 70–160 ms into their
slots, no forfeits, and no capture skips during the long session at either station.

## 1. The disconnect messages during the long transfer

**Observation.** About ten minutes into the long transfer Aether seemed to say it was
disconnecting, twice. The transfer went on. The operator aborted after about 6 kB.

**Evidence** (the portable's `aetherd.log`, times UTC):

| time | what |
|---|---|
| 23:35:07.091 | Winlink Express: `CONNECT WD5EMA KK4ODA`, at 2300 Hz: *calling KK4ODA at 2300 Hz, as the program placing the call asked, though it was last heard running 500 Hz* |
| 23:35:09.872 | connected, master |
| ≈ 23:35:28 | Winlink Express writes the message: `BUFFER 161605` |
| 23:40:27.362 | Winlink Express: `DISCONNECT`, answered `OK`. *the program asked to disconnect 299 s after it last wrote or was handed data, with 158812 bytes of its own still going: taken for its inactivity timer and not acted on, so the transfer goes on* (ADR-0134) |
| 23:45:27.545 | `DISCONNECT` again, 300 s after the first: held the same way, *600 s after it last wrote or was handed data, with 156455 bytes … still going* |
| 23:48:13.454 | Winlink Express: `ABORT` (the operator). Aether: *disconnected: aborted*, `BUFFER 155339` |
| 23:48:26.191 | home: *disconnected: peer disconnected* (it decoded the abort's DISC), then its Morse identifier, 23:48:26.6–34.2 |
| 23:48:38.472 | the portable: *the closing identifier went out as WD5EMA, in the data signal*; `DISCONNECTED` told to Winlink Express |
| 23:48:39 | both recordings stop; at the upload a minute later both stations are idle: nothing queued, no close under way, no identifier owed (`status` in both bundles) |

Home's Winlink Express sent no `DISCONNECT` during the long session: it was handed data
throughout, and that resets its timer (report 21 §9).

**What the panel showed.** Aether's panel says *Disconnecting* only from the station's state
(`disconnecting`) or a close under way (`status.closing`, `bannerFor` in `app/ui/app.js`). A held
`DISCONNECT` changes neither: the station stayed connected and the adapter wrote nothing to
Winlink Express but `OK`. The Log carried one *host* line each time, at info level. Winlink
Express, though, prints its own line in its session window at each timeout: *\*\*\* P2P session
inactivity timeout. Disconnecting.* (its log on 2026-11-24, report 21 §9). The two messages came at
exactly 300 and 600 s after it wrote the file. So the *disconnecting* the operator saw was most
likely Winlink Express's own session window, not Aether's (high confidence; its log for
23:35–23:49Z, or a look at that window, would settle it).

**Root cause.**
* What triggered each message: Winlink Express's P2P inactivity timer. It fires 300 s after the
  program last wrote, whatever the modem says, and fires again every 300 s while it waits.
* Where the request came from: the host application, at the portable. Neither modem asked to leave.
* Whether the modem entered a disconnecting state: no. No DISC was sent, nothing was retried or
  acknowledged, and the link carried data throughout (`BUFFER` fell from 158 812 to 155 339 after
  the first `DISCONNECT`).
* Whether a timer of Aether's expired: none did. No link timeout, no watchdog trip, no silent-slot
  ending, no drain.
* Whether the 10 minutes matter: yes. They are Winlink Express's 300 s timer firing twice.
* Whether the final abort was handled correctly: yes, at both ends. The DISC went on the floor,
  home answered it and identified, the portable identified after, each program was told
  `DISCONNECTED` after its own station's identifier, and both stations went idle with no state
  left over. The next calls of the evening, after the earlier aborts at 23:26:42 and 23:29:44,
  connected without a restart.

**Classification.** Possibility 1, a misleading message with no protocol malfunction; the message
is Winlink Express's. There is also a host-interface gap, by design: after its timer fires,
Winlink Express believes it is disconnecting while the modem carries on (ADR-0134 accepted it).
Two consequences were not weighed when ADR-0134 was built:
* **A hopeless transfer is kept going.** 155 kB at the session's 67 bit/s is about 5 hours. The
  hold kept the session up until the operator aborted. Had the `DISCONNECT` been acted on,
  ADR-0135's drain would have given up at once.
* **An operator's own Disconnect in Winlink Express can be taken for the timer.** If the program
  has been quiet for four minutes and the last held `DISCONNECT` was more than a minute ago, the
  rule cannot tell the two apart. It sees the same command. Whether Winlink Express's Disconnect
  button sends another `DISCONNECT` once its timer has fired is not known.

**Severity.** Low for the protocol: it behaved as designed throughout. Medium for the operator:
two programs say different things, and the hold can keep a transfer going that will never finish.

**Proposed resolution** (both the author's call, §9):
1. *Say it in Aether's panel.* While a `DISCONNECT` is held, a banner on every tab says *Winlink
   Express asked to disconnect (its 5-minute timeout); the transfer goes on. Disconnect or Abort
   here to end it.*, with the two buttons. GUI only.
2. *Hold only a transfer that can finish.* A held `DISCONNECT` stays held while what is queued
   could go within a set time at the rate the session has delivered (ADR-0135's own measure), and
   is acted on otherwise, so that the drain gives up at once and Winlink Express hears
   `DISCONNECTED`. A timeout policy: proposed 15 minutes. At this session's rate the portable would
   have closed at 23:40:27.

**Validation.** The host adapter's tests: timer `DISCONNECT`s 300 s apart, each held, with the
queue that fits; acted on with the queue that does not; `ABORT` always ends it. A harness stand-in
for Winlink Express with its 300 s timer (§7), run in real time, with a file that fits and one that
does not. Then a long transfer on the air, with Winlink Express's log.

## 2. The callsign `KK4ODA-1` at the start

**Observation.** The first calls failed. The operator saw the portable calling `KK4ODA-1` while
home listened on `KK4ODA`.

**Evidence.**
* The portable's Winlink Express called `KK4ODA` every time. Its host trace reads `CONNECT WD5EMA
  KK4ODA` at 22:40:58, 22:43:20 and 22:46:10, and the earlier call at 21:33 reached home as a call
  to `KK4ODA` too.
* Home's Aether answered only `KK4ODA-1`, its own callsign in Setup step 1. RadioPost Gateway
  claims that callsign, but it had stopped at 21:31:24. No program was attached to home's host
  port until 22:45:39: no `host:` line, no claim.
* Home logged twice: *mismatch: WD5EMA is calling KK4ODA, which this station does not answer to: it
  answers as KK4ODA-1. Answer as KK4ODA too, or ask WD5EMA to call KK4ODA-1.* (21:33:12,
  22:41:05). The panel shows this as the banner across every tab, and it is the only place
  `KK4ODA-1` was named.
* At 22:42:28 home's Aether callsign was changed to `KK4ODA` (restart). The next call, at 22:43:27,
  was refused differently: *a call from WD5EMA to KK4ODA not answered: the host program keys the
  radio, and none is attached*.
* Home's Winlink Express attached at 22:45:39: `MYCALL KK4ODA`, `LISTEN ON` at 22:45:43. The
  portable's call at 22:46:10 connected at once.
* At 22:42:58 the portable's Aether callsign was also changed, from `KK4ODA-2` to `KK4ODA`.

**Root cause** (high confidence). Home's Winlink Express was not connected to Aether. Aether
answered for itself as its own callsign, `KK4ODA-1`, as ADR-0108 has it with no program attached.
The mismatch advice then pointed at the callsign rather than at the missing program. It said what
the station answered to and suggested calling that. It did not say that the station is set up for
an external program (*External programs* on, keyed by *External program*), that none was attached,
and that the program, once there, would answer for `KK4ODA`.

* **Callsign handling.** `MYCALL` and the program's claim behaved as specified (ADR-0106,
  ADR-0108). At attach the adapter claims the station's own callsigns, as VARA uses its configured
  callsign until `MYCALL`; the program's `MYCALL` then replaces them. Calls were matched by the
  whole callsign, an SSID included, as specified.
* **Stale settings or sessions.** None influenced the callsign called: Winlink Express named its
  target itself.
* **Regression.** No: the same rules since ADR-0108 (2026-11-12).

**Something to change back.** Both stations' Aether callsign is now `KK4ODA`. While no program is
attached, a call to `KK4ODA` would be answered by whichever station hears it, and both would also
answer to it. The portable was `KK4ODA-2`, which was right.

**Classification.** Host-integration usability defect: the advice is misleading, while the
matching is correct. **Severity**: medium, because it cost four failed calls and two restarts and
sent the operator after the wrong cause.

**Proposed resolution** (the message wording is the author's call, §9). When a station set up for
an external program has none attached, a call to one of its kin callsigns says so: *WD5EMA is
calling KK4ODA: no external program is attached to answer it. Start Winlink Express, or check that
it points at port 8700. Aether itself answers as KK4ODA-1.* The same goes for a call to the
station's own callsign refused because nothing can key the radio (22:43:27), which today is a log
line and no banner.

**Validation.** Station tests: a call to a kin callsign with *External programs* on and no program
attached gives the new sentence; with a program attached, or with *External programs* off, it gives
today's. A harness scenario: a Winlink-like stand-in attaches 30 s after the first call, and the
call after it connects.

## 3. The Host indicators

**Observation.** Setup's callsign and bandwidth used to say **Host** when an external program
controlled them, and no longer do.

**Evidence.** They were removed on purpose by `6d50a62` (2026-11-23, beta.173), the author's
decision after a 2300 Hz call was refused at a station whose greyed Bandwidth showed only "Host",
hiding the 500 Hz it ran (`CLAUDE.md`, *The portable's sessions of 2026-11-23*). Since then:
* Setup step 1's box always shows Aether's own callsign. Its note reads *(for Aether standalone
  use; an external program's MYCALL is in use now)* while a program is attached (`applyCallsignOwner`).
* Step 4's Bandwidth always shows the station's own. An external program's `BW` is said only by
  the header's bandwidth chip.

Host-control detection still works. The daemon knows both values: `status.claims` names the
program's callsigns (`WD5EMA` at the portable), and `status.bandwidth` carries `bandwidth_hz`
2300, `by`, `why: host` and `home_hz` 500.

**Root cause.** Not a regression. The design change left two gaps:
* The callsign note does not say which callsign the program uses.
* While Winlink Express runs the station at 2300 Hz, Setup step 4 reads 500 Hz, the station's own
  setting. Only the header chip says 2300.

**Classification.** GUI defect: the values shown are the station's own settings, not always the
ones in use. **Severity**: medium.

**Proposed resolution** (labels are the author's call, §9). Keep the station's own values in their
boxes, as decided on 2026-11-23, and add after each the value in use and whose it is, while that
differs:
* *In use now: WD5EMA (Winlink Express)* after the callsign.
* *In use now: 2300 Hz (Winlink Express asked)* after the bandwidth.

Both come from `status`, which the panel already polls.

**Validation.** Panel tests in headless Chromium against a stand-in modem: no program attached;
Winlink Express attached with another `MYCALL`; `BW2300` over a 500 Hz station; a program that
leaves.

## 4. Throughput at 500 and 2300 Hz

**Observation.** Small transfers went well. The large file at 2300 Hz crawled.

**Evidence, the long session** (`rung_reading.py`; data read at home):

| rung | frame | sent | decoded alone | never detected | delivered | air (exchanges) | bit/s |
|---|---|---|---|---|---|---|---|
| 1 | tone-36 | 8 | 8 | 0 | 264 B | 59 s | 36 |
| 2 | tone50-51 | 29 | 28 | 0 | 1 008 B | 191 s | 42 |
| 3 | tone50-75 | 46 | 32 | 3 | 2 448 B | 260 s | 75 |
| 4 | tone100-105 | 22 | 4 | 0 | 612 B | 138 s | 36 |
| 7 | BPSK ⅓ | 9 | 6 | 1 | 258 B | 16 s | 126 |
| 9 | QPSK ⅓ | 47 | 17 | 18 | 1 564 B | 90 s | 139 |
| 10 | QPSK ½ | 3 | 0 | 0 | 141 B | 9 s | 126 |

By two-minute windows (time from the recording's start, its 30 s pre-roll included; bytes decoded
at home, copies included):

| minutes | bytes | bit/s | SNR read at home | rungs |
|---|---|---|---|---|
| 0–2 | 2 057 | 137 | +3.6 dB | OFDM 6–10 |
| 2–4 | 408 | 27 | −1.6 | 2, 4, a few OFDM |
| 4–6 | 438 | 29 | +0.9 | mostly 4, which failed 12 of 13 |
| 6–8 | 942 | 63 | −4.8 | 2–4 |
| 8–10 | 1 329 | 89 | −3.1 | 3 |
| 10–12 | 705 | 47 | −1.1 | 1, 3 |
| 12–14 | 714 | 48 | −3.0 | 1–4 |

* **Bandwidth**: 2300 Hz, by Winlink Express's own `BW2300` and the operator's new profile
  *Winlink Express Aether TX-500 P2P 2300* (ADR-0102: a program's call goes in the bandwidth it
  chose).
* **Application against modem throughput**: 6.4 kB delivered in 13 minutes is about 67 bit/s. The
  frames themselves carried 42–79 bit/s on the tone rungs and 203–357 on the OFDM rungs; the
  difference is the answers, the copies and the failures.
* **Time on the air**: the master transmitted 21–27 s an exchange (four or five tone frames). Home
  answered with a 0.43 s ordinary control frame 21 times and a 3.2 s floor control frame 15 times
  (during the fade, after silent slots). The air between transmissions was 0.66–1.6 s.
* **Protocol overhead** was small. In the steady part an exchange took 23.4 s, of which 21.4 s
  was data (four tone frames, the header left out): about 8 % overhead. During the fade, with the
  answers on the floor, it was nearer 20 %. Silent slots were few: three headers lost in the fade,
  one answer unheard.
* **Retransmissions**: rung 4 failed 18 of 22, and rung 9 had 18 of 47 never detected. Copies
  delivered 34 blocks after a first copy failed; combining delivered 5.

**Did it deteriorate as the session went on?** No. The rate followed the path: 137 bit/s at
+3.6 dB, a fade from minute 2 to minute 8 to −5 dB, and 89 bit/s again once it eased. Nothing grew
with time: no buffer, no timer, no state. Timing stayed as it began, and no capture audio was lost.

**Expected physical-layer limits.** At −1 dB on the 3 kHz reference, the 2300 Hz air runs its
tone rungs, 54–228 bit/s on the air at best. On the same path at 500 Hz the four-tone rungs
delivered 52–157 bit/s an exchange (sessions 1, 2, 5), and the 2300 Hz session's tone rungs
36–75. At this SNR 500 Hz is the better choice. A 161 kB file at either bandwidth on this path is
hours: about 5 at the rate seen. That is the path, not a bug.

**Avoidable inefficiencies**, all the rate controller's choices:
* 138 s at rung 4 (tone100-105), 18 of 22 failed.
* 250 s on rungs 1–2 at 36–42 bit/s, while rung 3 delivered 75.
* Rung 9 kept through 18 frames never detected in the first two minutes.

By the author's rule (*The rate controller waits for the air*) these are evidence for its next
review, not a patch. They add to the common-case losses already logged: stays at a rung that fails
most of its frames, and slow returns.

**Classification.** Performance limitation; no defect. **Severity**: low.

**Proposed resolution.** None in code. In the manual's Winlink Express chapter: on a weak path
(the SNR tile under about +3 dB) choose 500 Hz in Winlink Express's session window. Add this
session to the rate review's evidence.

## 5. The *Receiving at* line

**Observation.** During longer sessions the green dashed *Receiving at* line on the Status tab's
speed chart stopped appearing.

**Evidence and intended behaviour.** The line is `metrics.receiving_rung`: the rung of the last
data frame decoded from the session's other station, while that was less than 30 s ago
(`Station::data_rungs`, `THROUGHPUT_WINDOW_S`; set by `note_session_frame`). It was introduced with
*Sending at* by ADR-0073 so the chart shows the rate the data moves at in each direction.
* **At the portable**, the sender, home sent data only in the B2F handshake (two frames, at 33 and
  47 s). The line ended about 30 s later and stayed gone. Correct: nothing was being received.
* **At home**, the receiver, the line dropped out twice, for 66 s from minute 2 and 59 s from
  minute 4. In both the portable's frames were arriving but not decoding. Counting detected
  frames, decoded or not, leaves no gap over 30 s.

**Root cause** (high confidence). The line follows decoded data only, so a fade blanks it while
data is still arriving. At the sender it is absent because nothing comes back. No timeout of the
link, no buffer limit and no state slip: the chart keeps 1 400 points over its ten minutes, and the
modem's state never changed.

**Classification.** At the sender, expected behaviour. At the receiver, a GUI defect of meaning:
the line answers "what decoded lately", where the operator reads "what is arriving". **Severity**:
low.

**Proposed resolution** (the meaning is the author's call, §9). Let the receiving rung follow every
detected data frame of the session's other station, decoded or not. The engine already files a
failed copy under a sequence number of the session (`last_data`), which is the evidence it is the
other station's. With no data arriving for 30 s the line ends, as now.

**Validation.** A station test: a run of failed but detected session frames keeps
`receiving_rung`; 30 s of none ends it. The long session's sidecar, replayed, gives no gap.

## 6. The Mode tile

**Observation.** The tile sometimes shows one number and sometimes two.

**What the numbers are** (`applyRung` in `app/ui/app.js`):
* **Two numbers**, *sending / receiving*: this station's data rung, then the other station's,
  when data moved both ways in the last 30 s. The line under them names the two frames.
* **One number, *receiving* under it**: only the other station's data moved in the last 30 s.
* **One number, *sending***: only this station's data moved.
* **One number, *… to send at***: no data moved in the last 30 s. It is the rung this station's
  own rate controller would send at now (`metrics.mode`, `GrantedEngine::current_mode`). Its
  tooltip says *as the other station last recommended*, which has been wrong since ADR-0099
  (2026-11-04): no station recommends a rung to the other any more.

The values match the modem's state: they are the rungs the frames went at, read off the frames.
The switching between one and two numbers is intended (ADR-0076, report 13 §13.3). The stale
tooltip is a regression of wording.

**Classification.** GUI: intended behaviour, one stale tooltip. **Severity**: low.

**Proposed resolution.** Waits on the author's choice of labels (§9). A proposal: always two
cells, **Sending** and **Receiving**, each showing its rung and frame, or *—* with *nothing in the
last 30 s*. The tile's last case becomes *Next: rung N*, under **Sending**.

## 7. Regression testing

What exists already, and passes:
* **Unit tests**: the host adapter's held `DISCONNECT` (three tests, ADR-0134); the hand-overs, the
  DISC's acknowledgement and the drain's give-up, in the model and the port (ADR-0135, the vectors'
  `hand-overs` session); the keyed-audio watch (ADR-0136).
* **Harness scenarios**:
  * Winlink-shaped exchanges at both bandwidths (`*-winlink-exchange-*`, `40m-wle-p2p-500`).
  * Aborts and recall (`40m-abort-and-recall-2300`), slow drains (`80m-disconnect-slow-drain-500`),
    leaving together (`80m-leave-together-500`).
  * A long attachment (`40m-long-attachment-cwid-2300`), Pat's buffer wait in real time
    (`80m-pat-fade-500`).
  * Outages, stalls and capture skips, lopsided paths, flutter.
* **The weekly** runs a gateway's day (`bench/scenarios/long/`), five hours of air, with memory,
  open files and threads watched for growth.

What is missing, and proposed, under accelerated or real-time clocks as each needs:

| test | where | covers |
|---|---|---|
| a Winlink Express stand-in (`host a wle`): writes a message whole, sends `DISCONNECT` 300 s after its last write and every 300 s after, its Abort an `ABORT` | `session_matrix.py` | the host program's timer |
| `80m-wle-long-file-500` and `-2300`, real time (the adapter's 240 s is wall-clock): a file that fits in 15 minutes, a 40 s outage in the middle, both stations' programs attached | `bench/scenarios/` (nightly) | long sessions at both bandwidths, sustained transfer, signal loss and recovery, timers during active transfer |
| `80m-wle-hopeless-file-2300`: a file that cannot finish; with §1's policy it closes at the first timeout | same | the policy, if adopted |
| `80m-wle-both-ways-500`: replies and messages both ways for 20 minutes | same | sustained bidirectional traffic, mode changes, the telemetry's lines |
| `40m-wle-abort-and-recall-500`: the program aborts mid-file, calls again without a restart, completes | same | manual termination and reconnection |
| `80m-wle-repeated-disconnect-500`: two `DISCONNECT`s within a minute mid-file; a remote `DISCONNECT` from the called program mid-file | same | local and remote, repeated disconnect requests |
| adapter tests: timer `DISCONNECT`s 300 s apart held; held then acted on by `ABORT`; an operator's `DISCONNECT` within a minute; the queue that cannot finish (§1) | `host/server.rs` | the timer rule |
| station tests: kin-callsign advice with and without a program attached (§2); `receiving_rung` through failed frames (§5) | `station.rs` | callsign advice, telemetry |
| harness scenario: Aether's own callsign with an SSID, the program's `MYCALL` without; a call to each before and after the program attaches | `bench/scenarios/` | callsigns with and without SSIDs, host-controlled callsign |
| panel tests in headless Chromium against a stand-in modem: the in-use callsign and bandwidth (§3), the Mode tile (§6), the speed chart's two lines through a fade and through 30 minutes | `tools/` (CI's panel step) | GUI telemetry consistency |

The harness runs two to five times faster than the air. A test of a wall-clock timer, the
adapter's 240 s or Winlink Express's 300 s, runs in real time (`realtime = true`, as
`80m-pat-fade-500` does) and belongs in the nightly, not CI's quick set.

## 8. What is proposed, in order

1. §1: the held `DISCONNECT` said in Aether's panel; the hold limited to a transfer that can
   finish. The tests of §7's first, third and sixth rows.
2. §2: the callsign advice when no external program is attached. Change the portable's Aether
   callsign back to `KK4ODA-2` (a setting, the operator's).
3. §3: *In use now* after the callsign and the bandwidth.
4. §4: no code; a sentence in the manual, and the session added to the rate review's evidence.
5. §5: the receiving rung through failed frames.
6. §6: the Mode tile's labels as the author chooses, and the stale tooltip.

None changes the air protocol. All keep Winlink Express and RadioPost working as they do. §1's
second half is a timeout policy, and §2, §3, §5 and §6 change what an operator reads, so each waits
on the author's answer.

## 9. Questions for the author

1. §1: should a held `DISCONNECT` be acted on when what is queued could not go within a set time at
   the session's rate (proposed 15 minutes), so a hopeless transfer ends at Winlink Express's first
   timeout?
2. §1: should Aether's panel say when it holds Winlink Express's `DISCONNECT`, with Disconnect and
   Abort in the banner?
3. §2: the new advice when no external program is attached: wording as proposed?
4. §3: *In use now: …* after the callsign and the bandwidth, wording as proposed?
5. §5: should *Receiving at* follow every frame detected from the other station, or only those
   decoded (today)?
6. §6: the Mode tile's labels: always two cells, *Sending* and *Receiving*, or keep one number when
   only one direction moves?
7. §1: is Winlink Express's log for 23:35–23:49Z (the portable, *RMS Express* log of 2026-11-24)
   available, to confirm the two *Disconnecting* lines were its own?

## 10. What the author decided, and what was built (2026-11-25)

The author answered questions 1–6 as proposed. Question 7 (Winlink Express's own log) is still
open. Nothing below changes the air protocol: beta.175 and the build after it interoperate.

1. **A held `DISCONNECT` is acted on past 15 minutes** (ADR-0134 §6). The engine now states the
   session's rate and how long what it still has would take at it (`session_rate`,
   `queue_finish_s`; model first, the vectors' digests compare both). The control API's `status`
   carries them as `session_rate_bps` and `queue_finish_s`. The adapter holds a timer
   `DISCONNECT` only while `queue_finish_s` is under `HOLD_FINISH_S` (900 s), or while no rate is
   known yet. Past that it acts on it, and ADR-0135's drain gives up at once. Adapter tests:
   held under 15 minutes, acted on past them, held when no rate is known.
2. **The panel says so**: `status.host.disconnect_held_ms`, and the `#held-banner` across every
   tab: *The external program asked to disconnect (its inactivity timeout). The transfer goes
   on.* with **Disconnect** and **Abort**. The banner goes when the session ends.
3. **The advice when no external program is attached** (§2): a call to another SSID of the
   station's callsigns, or to a station keyed by its external program, while none is attached, is
   a mismatch `program` whose sentence names the host port to check and, for an SSID, the
   callsigns Aether answers as itself (`note_no_program`). Two station tests.
4. **The settings an external program sets are greyed, not hidden** (§3; the author's refinement:
   Aether's own setting stays visible, marked inactive, with a legend). Setup step 1's callsign
   and step 4's bandwidth keep their values, greyed with a dashed edge, and the note beside each
   reads *In use: KK4ODA · Winlink Express* or *In use: 2300 Hz · external program*. With no
   program, step 1's note reads *(for Aether standalone use)*. The header's bandwidth chip reads
   *2300 Hz · external*.
5. **Receiving at follows every frame heard** (§5): `metrics.receiving_rung` follows each data
   frame the engine files in the session, decoded or not, so a fade no longer blanks the line.
   Checked through the vectors' `last_data`, which the station reads.
6. **The Mode tile is two cells, always** (§6): *Sending* and *Receiving*, each a rung and its
   name; idle, Sending shows the rung the next transfer starts at (*next: 7*). The tile's tooltip
   is replaced.

The manual (1.23.0) has the new banner, step 1 and step 4's greyed fields (Figure 14, new), the
Mode tile and the 15-minute rule; `docs/user/host-programs.md` the rule and a note on weak paths.
Every new label was checked to fit at 600, 900 and 1440 px and in Compact.

**Tests added for §7.** The harness's Winlink Express stand-in (`host a wle`): it writes a message
whole, sends `DISCONNECT` 300 s after it last wrote or was handed anything, and every 300 s after;
steps `host-write N` and `host-closed S`; `[expect]` keys `wle_timer` (its timer fired),
`wle_held` (the session ran on past it) and `wle_closes` (it closed within two minutes of it).
Two real-time scenarios: `80m-wle-long-message-500` (a message that takes over five minutes and
under fifteen: held, delivered, clean end) and `80m-wle-hopeless-file-500` (60 kB on a −2 dB
path: closed at the first timer). The other rows of §7's table stay to do.

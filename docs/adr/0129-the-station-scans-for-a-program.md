# ADR-0129: The station scans its channels for the program that asks, and never leaves one a caller may be on

**Status:** accepted for building, 2026-11-17. The author agreed the design and §7's four
answers the same day (§8). Proposed earlier the same day in answer to the author's question of how
RadioPost Gateway should scan, as Winlink gateways do with RMS Trimode. The timing is estimated on
the model's own call in `docs/research/scanning/00-timing.md`, and the harness judges the design
(§6) before it ships. RadioPost's side, with drafts for RP-4 and RP-5, is its
`docs/design/03-scanning.md` (wish W50).

## 1. Context

**Trimode's way.** A Winlink gateway scans with RMS Trimode. The program tunes the radio itself
and turns the modem's `LISTEN` off and on every 3.5 s or so, deaf for about 0.5 s each time. It
does this on one channel too. Through Aether's VARA port that design has cost five fixes, and it
still loses calls:

* ADR-0051: a `CONNECTED` said while the program's `LISTEN` is off is ignored. The soak then found
  one crossing a `LISTEN OFF` on the wire, and the adapter learned the program's beat.
* ADR-0096, ADR-0100, ADR-0103: `PENDING` first; a held `CONNECTED` that waits no longer than the
  program's scan; the acceptance's `PTT ON` before `CONNECTED`. Each stopped a "No PTT … hung"
  abort or a session the program never took up.
* 2026-11-07: a probe went unanswered because it landed in one of the program's `LISTEN OFF` gaps.
* The calling bench: 85 of 90 first decoded calls fell in the Trimode stand-in's deaf second
  (`docs/research/calling/01-results.md` §3.1). The callsign fixed where the tries fell against
  the scan, so every draw repeated the miss (§3.4).

The cause is one design choice. The scanner is a separate program, and it can only say "don't
answer now". It cannot see a frame arriving, and it cannot see audio still in the receiver from
the last channel. On some modems `LISTEN OFF` ends a call under way without a word to the other
station (Mercury's does, RadioPost's RP-4 §9.6).

**RadioPost Gateway today** listens on one dial. It tunes away for a partner's window and back
(RP-4 §4.1, through `frequency.set`). Its record can list several channels (RP-5 §4.4), but it
listens on only one of them. A client that calls another listed channel is never answered.

**Aether today:**

* `frequency.set` (`Station::tune_to`) refuses during a transmission, a session, a call, a probe,
  a Test, or while something that belongs to this dial is queued. It does not refuse while a frame
  is arriving.
* A dial move of 10 kHz or more makes the busy detector learn the band again. Nothing else
  restarts.
* The receiver keeps its lookback, and a floor frame is final only 18 symbols (0.72 s) after it
  ends (ADR-0009 §8). So a call heard on one dial can be decoded after a move, and answered on the
  next dial.

**The call** (ADR-0126) is two tries by default. From the engine, the same on both airs (`00-timing.md`
§1):

* A both-ways try is 6.41 s: the OFDM data frame (1.05 s), then tone-24 (5.36 s).
* Tries start 11–13 s apart, and 26–34 s before the last try (the pause).
* Two tries find a gateway scanning two to eight channels 26–63 % of the time (`00-timing.md` §4).

## 2. Decision (proposed)

### 2.1 The station steps the radio, the program says where

A program owns the plan: which channels, and when to stop scanning for calls of its own.
RadioPost Gateway is one such program; any control-API client can be. The station owns the
mechanism, and runs it on its run loop: the thread that hears frames, answers calls and keys the
radio. That thread already knows everything that should keep the radio where it is. Nothing uses
`listen` to scan: answering stays on throughout.

### 2.2 The control API

| Method or event | What it does |
|---|---|
| `scan.start {dials}` | Starts a scan, or replaces this connection's. `dials` is 2 to 11 distinct dials in hertz, the first the **home** dial. Answers `{dials, dwell_s, linger_s, scan_s}` (§2.3, §2.4). Refusals: `unsupported` when the keying interface cannot tune; `regulatory` for a dial where the rules would refuse this station's answer, with the dial named; `bad_params`; `claimed` when another connection's scan runs. |
| `scan.stop` | Stops this connection's scan. The radio goes to the home dial once nothing holds it where it is. |
| `status.scan` | `null`, or `owner` and `app` (as `claims`), `dials`, `at_hz`, `dwell_s`, `linger_s`, `scan_s`, `held` (why it may not move now, in a sentence, or `null`), `lingering_until_ms`, `hops`, and `dropped` (dials left out, and why). |
| `scan` event | Started, changed (a dial dropped) or stopped, with `why`: `asked`, `connection closed`, `tuned by another program`, `knob`, or `radio`. Each move is also the `dial` event it is today. |
| `connect {…, call_s}` | Optional. The call's tries keep starting for at least `call_s` seconds (§2.5). The answer carries `tries`. |

### 2.3 The rules of a move

1. **Move only when the dial is free.** All of `tune_to`'s conditions apply, and three more:
   * no frame announced and not yet reported (decoded or refused);
   * no identifier owed for a setup on this dial (ADR-0055's 30 s);
   * no linger running (rule 5).
2. **A move is a capture loss.** The audio from the tune command until the radio's change has
   come through the sound card is dropped the way a capture gap is (`Station::skip_captured`,
   `Air::lose_captured`). That is `set_frequency_hz`'s own time, the card's input latency, and
   the radio's settling: a
   constant measured on the bench (§6 step 2; the timing model assumed 0.5 s in all). The
   receiver starts again after it, so no frame begun on one dial is decoded on the next.
3. **The dwell is computed, not set.** It is the longest gap between two tries' starts before the
   pause, in the station's own call (both plans, any number of tries), plus rule 2's deafness:
   13.5 s today. A visit that long always holds the start of a try while a call is going on. A
   shorter dwell locks in phase with the tries: 3.5 s across three channels catches 0.66 of calls,
   and jitter only lifts that to 0.72 (`00-timing.md` §3). A longer dwell lengthens the cycle for
   nothing.
4. **A frame heard holds the dial until it is reported**, and an answer, a session and its closing
   identifiers hold it as they do today.
5. **Linger.** An announced frame that comes to nothing, or a probe this station answered, keeps
   the station on the dial until no frame has been announced for the call's longest gap plus
   rule 2's deafness (34.5 s today). It is ended early by a frame decoded as another station's:
   a call to another callsign, a frame of a session not this station's, a beacon. A visit lasts at
   most the longest call the engine makes (fifteen tries, about 207 s). On a path where each part
   of a try decodes half the time, lingering takes the calls caught from 79–89 % to 98–99 %
   (`00-timing.md` §5).
6. **After a session or an answered call or probe**, the scan goes on from that dial with a
   linger. A caller often calls again.
7. **The list is judged once, at `scan.start`.** Each dial is judged as this station's answer would
   be under its control mode (`regulatory_check`, direction `respond`). A dial outside the rules
   refuses the whole list, by name. The gate still judges every transmission at the dial the
   radio reports.
8. **Every move is verified.** `set_frequency_hz` reads the dial back. A dial the radio will not
   take is dropped from the scan and named in a `scan` event, once. Below two dials the scan
   stops at the home dial (`why`: `radio`). *Amended in §13:* a dial the radio refuses goes at
   once; a move it does not answer keeps the dial until three in a row go unanswered.
9. **The operator wins.** These stop the scan and tell its owner, who does not start it again by
   itself:
   * a `frequency.set` from any other connection, the panel's dial list included;
   * a dial reading, taken after a move was confirmed, that is not the scan's dial (the knob
     turned).
10. **A scan belongs to its connection** (ADR-0106). When the connection closes, the scan stops
    and the radio goes home once the dial is free.
11. **One bandwidth: the station's own.** A 2 300 Hz station answers a 500 Hz call by moving
    (ADR-0026). A 500 Hz station is reached by a 2 300 Hz caller that moves after its tries
    (ADR-0058). The scan never changes the bandwidth.

### 2.4 What the program publishes

The program publishes `scan_s` = (dials − 1) × dwell: how long a caller's tries must keep starting
for one of them to fall in a visit to its channel. It is RP-5's `scan_s` (RadioPost's draft).

### 2.5 `call_s` on `connect`

Model first. `GrantedEngine.connect(…, call_s=None)` takes the fewest tries, from 2 to 15, whose
tries before the pause start over at least `call_s`, and never fewer than `call_tries`. For a
gateway scanning *n* channels that is *n* + 2 both-ways tries, the fewest that catch 99 % of calls
in the timing model (`00-timing.md` §4). A `call_s` beyond fifteen tries' reach gets fifteen; the
answer's `tries` says so. Fifteen tries reach eleven channels, which is why `scan.start` takes at
most eleven.

### 2.6 What the station shows and records

* **The panel's dial row:** *Scanning for RadioPost Gateway: 7.101 MHz (2 of 4)*, with **Stop**,
  and the dial list held while it runs.
* **A recording** carries each move as a `hop` event with the samples dropped, so that
  `aetherd --replay` starts its receiver again in the same place. Each dial is recorded as today.

## 3. What it removes

* **The listen dance.** Trimode's design never reaches RadioPost: nothing tunes the radio from
  outside the station, and RP-4 needs no "do not answer while I tune". The VARA port's holds for a
  scanning program (ADR-0051, 0096, 0100, 0103) stay: Trimode still uses them.
* **Channels nobody listens on.** RadioPost's side lists in a gateway's record only the dials it
  listens on: the scan's, or its one dial (RadioPost gap G21).
* **Jitter.** It was suggested before the timing was measured, and it does not help
  (`00-timing.md` §3).
* **Settings.** None are added. The dwell and the linger come from the call's timing, and the list
  is the program's.

## 4. Alternatives considered

* **Trimode's way through RPMI** (the gateway tunes and cycles `listen`). Rejected: §1.
* **The gateway tunes and leaves `listen` alone, relying on `tune`'s refusals.** Better, but still
  blind to a frame arriving: RPMI has no event for one, and adding one would put the modem's
  receiver timing in a host's hands. It still answers a call heard before the move on the new dial,
  and every other modem would need the same logic again. It is the fallback should a modem without
  a scan of its own ever gain a dial.
* **A short dwell that stops when it hears something.** At three channels it catches 0.66 of
  calls (phase lock), and it depends on finding a frame from its middle.
* **Several receivers in one passband** (the receive mixer's offset, ADR-0079). This hears 500 Hz
  channels inside one sideband passband at once, with no moves at all. It does not cover channels
  on different bands, which is what gateways scan. Not now.
* **A second radio** (RadioPost's wish W26, built). No scanning at all; the best answer where a
  station has the radio.
* **A call that keeps trying until it hears the gateway.** This breaks ADR-0126's cap. `call_s`
  is bounded, and the program asks for it only when the gateway says it scans.

## 5. Consequences

* **A call to a scanning gateway is longer.** With plan H it takes *n* + 2 tries. At four
  channels that is six tries, and a call nobody answers lasts about 91 s instead of 42 s.
  ADR-0126's cap is relaxed only for calls whose program asks.
* **A caller connects later.** The median, to the end of the try that was caught, is 6–46 s at two
  to eight channels, against 1–6 s on a gateway that only listens. A caller who keeps the default
  two tries misses it 37–74 % of the time.
* **The busy detector learns each channel again** after a move of 10 kHz or more, about 3.5 s.
  Only the station's own calls and probes wait for it, and the program stops the scan before it
  calls.
* **CAT traffic** is one dial command per dwell. Trimode's habit of resending the whole radio
  setup is not needed: the dial is read back.
* **The antenna** must take every channel without a tuning cycle (a resonant antenna, or a tuner
  with memory). The manual will say so beside the scan.
* **The rules.** Every dial is judged at the start. Nothing is keyed by a move.
* **The interface.** New in the control API: `scan.start`, `scan.stop`, `status.scan`, the `scan`
  event and `connect`'s `call_s`. No wire change, no configuration key, no schema bump.

## 6. How it is judged (registered before any run)

**Step 0, the harness learns dials.** The channel server keeps a dial for each station and joins
only the stations on the same dial. A move is modelled as deafness. New scenario steps: `scan WHO
d1,d2,…`, and a caller placed on one of the dials. The Trimode stand-in stays as it is.

**Step 1, model first.** `call_s` in `GrantedEngine`, with a granted vectors session that uses it.

**Step 2, the port.** The scan in the station, the control API, the panel's line, the recording's
`hop` event and the replay. The radio's settling time is measured on the bench, at the radio.

**Step 3, the harness.** A narrow miss goes to the author with its trace.

| | Criterion |
|---|---|
| **S1, wrong dial** | No frame is keyed on a dial other than the one the frame it answers was heard on. No acceptance follows a move for a call heard before it. Zero in every run. |
| **S2, held** | The scan never moves while a frame is announced and not reported, while an answer is queued, while a session or call is up, or while an identifier is owed for the dial (the station's own events say). Zero. |
| **S3, a good path** | Callers at +10 dB, on AWGN and ITU Moderate, at 2 300 and 500 Hz, against 2, 3 and 4 channels, each calling with the rule's `call_s`: at least 9 of 10 draws connect in each cell. |
| **S4, a weak path** | At the SNR where one try decodes about half the time (the calling bench), 3 channels: at least 8 of 10 draws connect. Linger on does at least as well as linger off, paired by draw. |
| **S5, no harm** | No scan running: every scenario × 3 draws stays within `scenarios.csv`'s ranges. |
| **S6, the end** | The owner's connection closing stops the scan, and the radio is on the home dial once the dial is free. Another connection's `frequency.set` stops it and the owner is told. Zero failures. |
| **S7, each part earns its place** | Linger off fails S4. The hold on an announced frame off fails S1 or S3. The move as a capture loss off fails S1 (a planted call that ends just before a move). A part that fails nothing when taken out goes. |

**Step 4, the air.** The home gateway scans two channels, and the mobile calls it on each.

## 7. For the author

1. **The operator wins** (rule 9): a `frequency.set` from another program, or the knob, stops the
   scan. The other way round, the scan would refuse them and the operator would have to stop it
   first.
2. **At most 11 channels**, which is fifteen tries' reach. A lower limit would cap the time a call
   nobody answers holds the channel: four channels give 91 s.
3. **The panel** could start a scan of the station's own, for an operator waiting for calls on
   several channels. Proposed: not now.
4. **Step 0** is harness work that must come before any criterion can run. The air could come
   first, with the harness after.

## 8. The author's answers (2026-11-17)

1. **The operator wins** (rule 9): another program's `frequency.set`, or the knob, stops the scan,
   and its owner is told.
2. **At most 11 channels.**
3. **The panel does not start scans of its own**, for now.
4. **The harness first**: step 0 and the criteria before the air.

## 9. What was built (2026-11-17, steps 1, 2 and 0)

**Step 1, the model and the port.** `GrantedEngine.call_offsets(plan)` gives each try's start of a
call nobody answers, from the helpers that schedule it, and a test holds it to what the engine does
through the simulator to a microsecond (both airs, every plan, with a turnaround and an answer gap).
`call_gaps()` gives the dwell's and the linger's gaps: 13.01 s and 34.43 s today. `tries_lasting`
and `connect(…, call_s)` are §2.5. The granted vectors gained the `call-lasting` session and a
`call_timing` table, and a planted fault in the level step fails both suites.

**Step 2, the station** (`core/aetherd/src/station/scan.rs`). §2.3's rules as written, with these
choices made in building it:

* `tune_to`'s checks are one function, `Station::dial_movable`, which the scan shares, so the two
  cannot disagree.
* A move restarts the receiver with `Air::restart_front_end` and `Air::lose_captured(0)`, at once
  and again `move_deaf_s()` later (the card's 0.25 s and `MOVE_SETTLE_S`, 0.25 s, until the bench
  measures it). The busy detector relearns on a move of 10 kHz or more, as any dial reading does.
* **The panel's Stop** is `scan.stop` from a connection with no claim: as the panel acts on any
  program's call or session (ADR-0106), it stops any program's scan, as the operator. The radio
  stays where it is, and `why` is a sixth word, `operator`.
* `connect`'s answer carries `tries`: the call's plan, which depends on how the station called was
  heard (ADR-0126 §2.3).
* **The linger starts from a frame reported that came to nothing yet was acquired past the gate
  the station trusts** (`DETECT_CONFIDENCE_TRUSTED`, the rule its receive window keeps for "a burst
  is there at all"), or from a probe it answered; a frame decoded as another station's ends it. An
  announcement holds the dial (rule 4) and starts no linger. Two builds came first. One started the
  linger from any frame reported undecoded (read back before any run: on a quiet band the receiver
  reports a noise candidate every few seconds). The other started it from every heeded
  announcement: the first grid found 2 of 128 visits held 41–44 s on a dial nobody used, each
  from a tone announcement on noise at confidence 1.01 (a tone arrival is heeded whatever its
  confidence) that no report followed, and the caller lost a whole cycle (a call of 80 s for
  20 s). Rerun on the same draw, the gateway left on its dwell and the call took 19.6 s.
* **The dwell covers a deferred try** (`GrantedEngine::call_deferral_s`, model first): a caller
  that hears a frame arriving moves its next try past it (ADR-0016), and a tone arrival on noise
  is heeded, so a try can stand apart from the last by the gap the engine schedules plus the
  longest frame and the wait after it (5.71 s). The grid's next run found it: the caller heard
  noise at confidence 1.01 and 1.03, two of its gaps grew to 14.5 and 15.1 s against a 13.9 s
  dwell, and a try began 0.35 s after the gateway had left. The dwell and the linger now include
  it: about 19.2 s and 40.1 s at the defaults, 19.6 s on the harness's stations. `scan_s` grows
  with it, and so does a call asked to last.
* **A call asked to last keeps its bandwidth.** The same run's caller went on at 500 Hz after four
  unanswered tries (ADR-0058): its station scans, and an unanswered try says it was elsewhere. A
  call with `call_s` no longer moves (`Station::call_lasting`).
* A recording carries each move as a `hop` event (`dial <hz> deaf <s>`), and `aetherd --replay`
  restarts its receiver at both points.
* The panel's dial box has a **Scanning** row (the dial, its place, the program; *held* or
  *lingering*; **Stop**), and the dial list is held while a scan runs. The manual (1.3.0) says so.

**Step 0, the harness.** The daemon on a channel server greets `AEC4` and has a radio
(`channel_link::ChannelRig`) that keys nothing and has a dial: it tells the server where it tuned
(message 8), and the server tells it where the radio is (the scenario's `dial_hz`, and `knob WHO HZ`
commands). The server joins only stations on one dial, runs every path on silence otherwise so its
draws stay as they were, counts collisions per dial, records each station's dials and each
transmission's, and judges S1 from the air (`dial_faults`): an acceptance with no call on its dial
in the minute before it, or after its station moved since that call. `session_matrix.py` judges
those as invariants on every run, and has the steps `scan WHO D1,D2,…` (a program's connection held
for the scan's life, `tools/control_ws.py`), `scan-stop WHO`, `knob WHO HZ`, `tune WHO HZ` and
`connect call_s=N|scan`. Every scenario before dials runs with every station on one dial, as before.

The first run (`bench/scenarios/40m-scanning-gateway-2300.toml`, three dials, AWGN +10 dB): the
gateway left 7.101 MHz for 10.145 MHz at 14.0 s, the caller's first try at 7.7 s went unheard and
its second at 19.1 s was answered at 26.2 s; no dial fault, no collision. The harness's quick set
passed unchanged.

## 10. The harness's verdict (2026-11-17, step 3)

`docs/research/scanning/01-results.md` has every run. S1, S3, S5 and S6 pass: no frame keyed on a
wrong dial in 240 runs, 120 of 120 good-path calls over 2 to 4 dials at both bandwidths, nothing
slower or failing more than the baseline, and the operator's tune and the knob end a scan. **S4
fails:** at −16 dB on Moderate, three dials, 6 and 5 of 10 calls connected against 8; the same
calls to a gateway on a fixed dial connected 9 and 10 of 10. The broken assumption is the timing
model's (`00-timing.md` §4): it took each part of a try to decode half the time, independently,
where at such a path a try connects about one time in three, so a call of *n* + 2 tries puts too
few on the gateway's dial. S7: the hold and the restart each cost calls when taken out (calls a
cycle later without the hold; a call lost without the restart, the gateway lingering on the dial
it had moved to), though neither showed the fault registered for it; the linger showed nothing
(12 of 20 without it, 11 with).

Step 3 stops here, as the rule for a failing criterion says. The fix is the caller's rule: a call
long enough to put four or five tries on the gateway's dial at a weak path (about twice `scan_s`),
or fewer dials for a gateway that serves weak paths, or the cost accepted and said. That, and
whether the linger stays, is the author's decision. RP-4 and RP-5 1.1.0 wait on it.


## 11. Amended: a short listen on a quiet dial, a stay where a call is under way (2026-11-17)

**The author's correction** after §10: waiting on a quiet dial makes no sense. A gateway that hears
nothing for a few seconds can move on, as Trimode does; it needs to sit through a caller's gap
between tries only where a try is under way. The dwell of §2.3 had been sized for a gateway that
could catch a try only from its start. The author asked that the detector be measured first, and
that the dials be capped where it is best.

**The measurement** (`docs/research/scanning/02-sense-and-stay.md`): a receiver started partway
through a caller's real try announces the tone frame's middle or end sync block as an arrival,
without the block before it, within about 1.1–1.5 s. From any point of the try but its last second,
3.5 s of listening noticed it on good paths at either air; at −16 dB on Moderate three to six times
in six. Twenty minutes of noise reached confidence 1.07; the first notice of a try heard partway
through stood at 1.1 or more in four of five at −16 dB, its median 1.58. Fed to a timing model
against the engine's tries, the rule reads every call on good paths up to four dials and, at the
weak path S4 is set at, 85 % at 500 Hz and 96 % at 2300 Hz with four dials, falling to 76 % and
85 % at five.

**What changes** (rules 3 and 5 of the module, `station/scan.rs`):

* **The dwell** on a dial is the move's deafness and `NOTICE_LISTEN_S` (3.5 s), 4.0 s, and a part
  drawn at random up to `DWELL_JITTER_S` (1 s), where it had been the caller's gap (19.2 s). The
  random part came from the harness: three dials at exactly 4 s are a 12 s round against tries
  11.4–12.4 s apart, and at one phase of the grid every visit fell in the quiet between tries, on
  both airs, and the call was never heard (`02-sense-and-stay.md` §4).
* **A notice** is an arrival announced, or a frame reported that did not decode, at
  `NOTICE_CONFIDENCE` (1.1) or more over its threshold. It starts the linger; under it an
  announcement only holds the dial, as before. The linger had started only from a frame reported at
  the trusted gate (1.3), never from an announcement.
* **The linger** is the caller's gap: the longest gap between two tries' starts before a call's
  pause, the most a try can be moved past a frame heard arriving, and the move (19.2 s; 19.6 on the
  harness's stations), counted from the notice. It had been the longest call gap of all (40 s).
* **`scan_s`** is a round of the dials at their longest dwell and two lingers, `n × 5.0 + 2 × 19.2`
  rounded up: 49, 54 and 59 s for 2, 3 and 4 dials. A try noticed is followed by two the scan stays
  for.
* **The cap** is four dials (`MAX_SCAN_DIALS`), where it had been eleven. Three-quarters of the
  Winlink HF gateways listed on 2026-11-01 publish four or fewer.

**Found on the way:** a dial reading asked of the radio just before a move and answered after it
carried the dial the radio had left, and the station took it: a scan that ended then thought the
radio home and left it on another dial. A reading taken before the one the station holds is now not
taken (`radio_reads.rs`).

The harness judges the amended scan as it judged the first (§6): S1, S3, S4 and S7 again, S6
unchanged.

## 12. The amended scan on the harness, and silence behind the new receiver (2026-11-17)

`docs/research/scanning/02-sense-and-stay.md` §4–§6 has every run.

**The jittered build** (`76853d1`, §11 as written): S1, no frame keyed on a wrong dial in 260
runs of it and of the two builds below with a part taken out; S3, 120 of 120 good-path calls (median 10.9–19.6 s, the longest 43.4 s at four dials); S4,
9 and 9 of 10 weak calls, 9 and 8 run again on the same draws (8 and 8 for exactly 4 s; a gateway
on a fixed dial took 9 and 10). S7:

* **The hold** shows nothing on a strong path, where a try heard from its start is announced at
  1.1 or more and the notice's linger keeps the dial longer than the hold would. On S4's draws
  the build without it connected 15 of 20 against 17 and 18: an announcement under 1.1 is what
  only the hold keeps the dial for. It stays.
* **The restart**, as built, kept the receiver from reading a try it arrived into. A new
  receiver has no audio from before it started, and a tone frame needs its first sample inside
  the receiver's stream, so a try begun on the new dial before the gateway got there was
  noticed and stayed for, never read. Without the restart, ten tries the gateway came home into
  2–3 s late were each read at once (11 s), where the full build waited for the next try (39 s)
  or lost the call; S4's draws connected 18 of 20; and nothing was keyed on a wrong dial. The
  restart is still what keeps the old dial's audio out of the new one's frames (the first
  design lost a call without it, §10).

**What changes:** rule 2's second start of the receiver, once the move's deafness is over, has
the longest tone frame of silence behind it (`Air::listen_after_silence`), fed as a muted
transmission is. A tone frame begun on the new dial before the station arrived is found by its
middle and end sync blocks (ADR-0049, ADR-0070) and read with its first part erased; nothing of the
old dial reaches it. On the model's receiver a try arrived into within its first 2.6 s is read as
a receiver never started again reads it, and none later (its middle sync block gone). It costs
the receiver 80 ms of work at each move. An OFDM frame begun before the arrival is lost either way.
No constant, wire or setting changes, and RadioPost's `scan_s` is the same.

**On the harness** (`02-sense-and-stay.md` §7): S1, no frame keyed on a wrong dial in 200 runs;
S3, 120 of 120, the median call at three and four dials 15.2 s (19.5 before) and the longest
34.2 s (43.4); S7, 20 of 20 planted calls (18 before), nine tries reached 2.0–2.8 s late read at
once (10.9 s, against 39 s). S4: 54 of 60 over three passes of its twenty draws, as the build
before (54 of 60). Two cells of the six came out at 7, under the 8 S4 asks; every call lost in
them connected in another pass of the same build, and at a 90 % rate a cell of ten falls to 7 one
time in fourteen. S4's per-cell count is noisier than what it was set to see: the pooled count is
the verdict, and on it the weak path is unchanged. S6 is unchanged by the change, and S5 is the
nightly's.

**For the author:** a call that does not last `scan_s` reaches a scanning gateway far less often.
The planted calls are the station's own two tries, with the pause before the last (15 s and up to
a try's length), which is longer than the linger: through the timing model a two-try call reaches
a scanning gateway on a good path 72, 55 and 43 % of the time with 2, 3 and 4 dials. A linger
long enough to cover the pause (26–29 s) would make that 98–100, 83–85 and 63–64 %, and `scan_s`
15–20 s longer, with every false notice costing as much more. RadioPost's client calls for
`scan_s`; the VARA port's hosts and the panel's Connect cannot ask for a call's length. Not
changed.

## 13. Amended: a move the radio did not answer keeps its dial (2026-11-17)

**What happened** (report 18 §3, `docs/research/link-architecture/18-radiopost-40m-beta162.md`).
The gateway's FTDX10 released its key after its closing identifier at 20:14:50.876, and the scan
asked for 3.590 MHz 5 ms later. A Yaesu says nothing back to `FA…;`, so the CAT backend reads the
dial back, and the radio, just out of transmit, did not answer in time. Rule 8, as built, dropped
the dial on any error; one dial was left, so the scan ended, and the radio stayed on 7.101 MHz for
the rest of the day. After the five sessions before it the same move followed the identifier and
the radio answered, so it was timing, and it will happen again. A dial the radio did not answer
for is not a dial it will not take, which is what rule 8 meant.

**What changes** (rule 8; `station/scan.rs`, `ptt.rs`):

* **A refusal and no answer are told apart by type, not by the error's words.** `PttError::Refused`
  is the radio, or the program in front of it, answering no:
  * the dial read back after the move is another (a locked VFO, a band the radio has not got);
  * an Icom answers NG (`FA`);
  * `rigctld` returns one of Hamlib's soft errors, the rig having answered: `RIG_IS_SOFT_ERRCODE`
    in Hamlib's `rig.h` (invalid parameter, not implemented, rejected by the rig, not available,
    VFO not targetable, invalid VFO, out of the function's domain, deprecated, security, not
    powered on), less `RIG_ETRUNC` (a command performed, its argument truncated);
  * `FLRig` answers with a fault.

  Everything else stays `PttError::Backend`, which says nothing of the dial: no read-back, an
  Icom's silence or only its echo, the keying thread's bound run out or its port still busy,
  `rigctld`'s timeout, I/O or bus errors, `rigctld` or `FLRig` not reachable, an HTTP error.
* **A refused dial goes at once**, as before.
* **A dial the radio did not answer for is kept** until `UNANSWERED_TO_DROP` (3) moves to it in a
  row go unanswered. A move to it that the radio answers clears the count. On the third miss the
  dial goes, its `why` saying `no answer 3 times in a row`. Below two dials the scan ends as before
  (`why`: `radio`).
* **Where the scan is after a miss.** As far as the station knows the radio has not moved, but
  it may have. So:
  * the receiver starts again as after a move, at once and after the move's deafness (rule 2), so
    no frame begun on one dial is answered on the other, whichever the radio is on. The recording
    has a `hop` event ending `unanswered`, and the replay follows it as it follows a move;
  * the scan keeps its place for a dwell, with its random part, and then asks for the dial again
    at its next move. It never asks again at the next pass, after a first move from a dial off
    its list either: three misses in a third of a second would drop a dial the radio missed once;
  * the first reading of the dial taken after the miss says where the radio is. The station reads
    it every second while idle. On the dial asked for, the move went through after all: the scan
    is there, and goes on from it. On the dial it was on, nothing changes. Neither is the knob
    (rule 9); any other dial still ends the scan.
* **The log says so:** `the radio did not answer at 3.5900 MHz; the scan tries it again after a
  dwell (miss 1 of 3: …)`; `the radio reads 3.5900 MHz: the move went through after all`; and on
  the third, `the radio did not answer at 3.5900 MHz 3 times in a row (…); the scan leaves it
  out`.

**Asked again at the next move, not a round later.** With two dials, as the gateway had, the two
are the same. With three or four, a round later would leave the dial unheard a round more, and
the dial the radio is on gets its extra dwell either way.

**What it costs:** a miss keeps the radio a dwell (4–5 s) more on the dial it was on. A radio that
answers nothing loses each dial after three moves to it, at least a dwell apart: the scan ends in
about 15 s with two dials and 25 s with three, where it ended at the first move.

**What it removes:** nothing. It narrows what drops a dial. No wire, setting, constant of the
timing, or field of `status.scan` changes, and `scan_s` is the same.

**How it is tested:** station tests in `station/scan.rs` with a radio that misses one read-back and
then answers (both dials kept, the scan goes on, a dwell before it asks again), misses one but went
there (the next reading finds it; without it that reading is taken for the knob), reads another
frequency (dropped at the first move), misses three in a row (dropped, and two misses with an
answer between them are not in a row), and misses the first move from a dial off the list (asked
again after a dwell). Each fails with its part of the change taken out. `ptt.rs` tests each
backend's answers: the read-back, the Icom's FB, NG, silence and echo, `rigctld`'s `RPRT -9`,
`-1` and `-5` and no `rigctld` at all, `FLRig`'s fault and HTTP error, and the keying thread. No
harness scenario: the channel server's radio always answers, and teaching it to miss a read-back
is more than the fix.

## 14. Amended: a broadcast keeps the radio where it was heard (2026-11-21)

**What happened.** The author asked how a RadioPost alert (RadioPost's RP-8, wish W52) reaches a
station whose RadioPost Gateway has Aether scan, with the RadioPost client listening beside it on
the same radio. An alert is a broadcast: an Aether datagram of three tone frames, about 16 s at
rung 1, and each answer two, about 11 s. Read against the scan as built, two things lose it:

* **The copies are lost between dials.** A decoded datagram piece counted as another station's
  frame (rule 5's `_ => idle`) and ended any linger. A sender that sends its alert several times
  to outlast the scan (below) leaves a gap between copies, and the scan moved in it. A station
  that heard a copy partway through could not join it (a datagram needs all its pieces), and
  left before the next.
* **The answer goes out on the wrong dial.** A datagram waiting for a clear channel goes with the
  radio when it moves (`holds_the_dial`: right for a beacon or a KISS frame the operator moves off a
  busy dial). The scan used the same test, so the client's automatic *heard*, queued a moment after
  the alert ended, went out on the next dial the scan reached, where the sender was not. That is
  §6's S1 (no frame keyed on a dial other than the one the frame it answers was heard on): Aether
  cannot see that a datagram answers one, so the scan has to keep the dial for any.

**What RadioPost does** (its side, the author's decisions of 2026-11-21): an alert to a station that
runs a listed gateway goes on one of that gateway's channels, and its copies keep coming for the
gateway's published `scan_s`; an alert to the whole circle on a watch dial a listed gateway scans
does the same; the station alerted answers the copies of one sending once.

**What changes** (rule 11; `station/scan.rs`):

* **A piece of a datagram decoded starts a linger**, as a try noticed does (`scan_heard`): about
  19 s, renewed by each piece, so the next copy, and the rest of one heard partway through, are
  heard where they come. It no longer ends one. The visit's most (`cap_s`, the longest call and a
  dwell) still bounds it: a dial busy with KISS programs' datagrams cannot keep the scan for
  longer than a call could.
* **A datagram waiting to go out holds the dial** (`scan_held`, `datagram_waiting`): in the
  station's datagram queue or handed to the transmitter's. `status.scan.held` says *a datagram is
  waiting to go out on this dial*. The hold too lasts at most `cap_s` from the move to the dial: a
  datagram that a busy channel keeps back past it goes with the radio, as before. `frequency.set`
  and the panel's dial list are unchanged: a tune still takes a waiting datagram to the new dial.

**What it costs:** a scan stays about 19 s, where it stayed 4–5 s, on a dial where any datagram is
heard, and longer while one of its own waits. A gateway whose channels carry other programs'
broadcasts (VarAC's over KISS) is held on them more, up to `cap_s` a visit.

**What it removes:** nothing. No wire, setting or constant changes, and `scan_s` is the same: a
sender outlasting the scan sends for as long as a caller's tries must keep starting.

**How it is tested:** station tests in `station/scan.rs`: a datagram piece decoded starts a linger
and a second renews it, and the scan moves once it runs out (without the change it moved after the
dwell); a datagram that never goes holds the dial past the dwell and a linger, names the hold, does
not hold a tune, and lets the scan move once `cap_s` has passed since the move. No harness scenario
yet: the harness's stations send no datagrams while scanning; RadioPost's sending side is what
will exercise it.

## 15. Amended: an answer goes back to the dial it answers (2026-11-21)

**What happened.** §14 keeps the scan on a dial while a datagram waits there, which covers the
answer a program sends at once: RadioPost's automatic *heard*. Its other answers come later. The
operator acknowledges the alarm (*seen*) or types a reply minutes after the alert, and by then the
scan has moved on: the answer went out on whatever dial the scan was on, while the sender stayed on
the one it alerted on. The author chose (2026-11-21) to send them on the alert's dial. A program
cannot know that dial unless the station says it, and cannot send there while a scan it does not own
runs.

**What changes** (rule 12; `station/scan.rs`, `station/datagrams.rs`, the control API):

* **A datagram heard says its dial.** The `datagram` event carries `dial_hz`, the radio's dial when
  the datagram was heard whole (`ReceivedDatagram::dial_hz`, from the station's last dial
  reading); null when the station cannot read the dial.
* **A datagram may name its dial.** `datagram.send` takes `dial_hz` (`DatagramRequest::dial_hz`,
  kept on the queued datagram). It is a whole number of hertz above 0; anything else is refused
  `bad_params`.
* **A scan sends it there.** While a scan runs and the dial is one of its own (within the knob's
  50 Hz), the datagram is not fed until the radio is on that dial (`scan_goes_elsewhere_for`), it
  does not hold the dial the radio is on (§14's hold is for a datagram that belongs here), and when
  the visit ends the scan goes to that dial next instead of round its list (`datagram_dial_index`).
  There §14's hold keeps it until the datagram has gone, and the scan goes on from there. The queue
  goes in order, so a datagram behind it waits too: a few seconds.
* **Otherwise it goes where the radio is**: no scan, a dial the scan has not (or dropped), or a
  radio that cannot report its dial. The station never tunes for a datagram: with no scan the
  operator, or the program that owns the radio, decides where the radio is.

**What it costs:** the answer waits for the current visit to end, 4–5 s on a quiet dial and up to
`cap_s` where the scan lingers for a call, and then for the move's deafness. A KISS program's
frame queued behind it waits as long.

**What it removes:** nothing. No wire change, setting or constant; `scan_s` is the same. A
program that never says `dial_hz` sees no change, and one that says it to an older station is not
refused: the member is ignored and the datagram goes where the radio is.

**How it is tested:** station tests in `station/scan.rs`: a datagram heard whole during a scan
says the scan's dial; a datagram that names another of the scan's dials is not sent and does not
hold the dial the radio is on, the scan goes to its dial next (not to the next in order), and it
goes out there; one naming a dial the scan has not goes where the radio is. Each fails with its part
taken out (fed anywhere, the dial left out of the heard datagram, the scan going round its list).
`control/methods.rs` tests `dial_hz`'s refusals.
